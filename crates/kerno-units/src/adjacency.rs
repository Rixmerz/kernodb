//! The adjacency index as posting blocks (ADR-0026, layer version 9,
//! `docs/file-format/units-v9.md`).
//!
//! # Design
//!
//! The adjacency list of a unit, a direction and a relation type is one
//! posting list of [`crate::blocks`]: the entries `(relation id, unit at the
//! other end)` in relation id order, cut in blocks of at most
//! [`ADJ_BLOCK_MAX`]:
//!
//! * `REL_OUT` / `REL_IN` `tag ‖ unit[16] ‖ type ‖ 0x00 ‖ 0x00 ‖ bound[16]` or
//!   `tag ‖ unit[16] ‖ type ‖ 0x00 ‖ 0x01` (the tail block) →
//!   `envelope[2] ‖ (edge[16] ‖ other[16]) × n`.
//!
//! The range of a unit and direction is `[tag ‖ unit, tag ‖ unit + 1)`, and the
//! range of one type is `[tag ‖ unit ‖ type ‖ 0x00, … ‖ 0x01)`: a relation
//! type has no NUL, so the lists of two types never share a prefix. A read of
//! a list is a range scan of a few blocks, not one record per relation.
//!
//! The `REL` record of each relation is still one record, and the counts of
//! [`crate::adj_count`] still guard a read against a lost block.
//!
//! # A write
//!
//! Creating or deleting a relation writes its `REL` record and keeps the two
//! adjacency changes, with the counts of the two lists, in the change set (one
//! map by list: a second relation of a list reads its count there).
//! [`ChangeSet::flush_adjacency`] writes them at commit: the count record of
//! each list once, and each block a list changes read once and written once
//! (cut when over the limit, deleted when emptied; the tail by a point read,
//! see `blocks::flush`). A list the change set knows to be empty has
//! no block to read: its blocks are written directly.
//! The change set knows it from the count record of the list, and when that
//! record is missing from the blocks themselves: the writer counts them with
//! one seek (`adj_count::count_entries`), so a list whose record was lost is
//! never taken for an empty one and its tail block is never written over
//! unread.
//!
//! A read inside the change set ([`ChangeSet::relation_of_unit`]) sees both:
//! the blocks of the transaction and what the change set keeps.

use std::ops::Bound;

use kerno_core::contract::{Snapshot, Txn, TxnEngine};

use crate::adj_count::{ListPlan, count_key, encode_value};
use crate::blocks::{self, Block, Id};
use crate::codec::{ADJ_BLOCK_MAX, AdjBlock, Key, TAG_REL_IN, TAG_REL_OUT};
use crate::db::{ChangeSet, IdHasher, UnitSnapshot, Write, context};
use crate::error::{DecodeError, UnitError};
use crate::types::{Direction, RelationId, UnitId};

/// Blocks read per scan call when looking for a surviving entry.
const PAGE: usize = 16;

/// What a change set will change in one adjacency list: its entries and its count record.
pub(crate) struct AdjList {
    /// The list has no block in the transaction: nothing to read before writing it.
    fresh: bool,
    /// The count of the list as the change set leaves it (`0`: no record).
    pub(crate) count: u64,
    /// The count record is written (or, at `0`, removed) at commit.
    dirty: bool,
    /// Per relation, ascending by relation id: the unit at the other end
    /// (`Some`: the entry is put) or `None` (the entry is removed); the last
    /// operation of a relation wins.
    ///
    /// ponytail: a sorted `Vec`, not a map: most lists get one or two operations per change
    /// set and ids are minted ascending, so an insert is an append. A change set that removes
    /// many relations of one list pays a shift per removal, bounded by `max_changeset_ops`.
    ops: Vec<(Id, Option<UnitId>)>,
}

/// One adjacency list of a change set: the unit, the key tag of its direction and its relation
/// type, interned in [`PendingAdjacency`]. Hashed and compared without building the list key.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct ListId {
    unit: UnitId,
    tag: u8,
    ty: u32,
}

impl std::hash::Hash for ListId {
    /// One word: the two halves of the unit id (random or a counter in their low bytes) and the
    /// list's tag and type, folded. [`IdHasher`] mixes it.
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        let (hi, lo) = self.unit.0.split_at(8);
        let hi = u64::from_le_bytes(hi.try_into().unwrap_or_default());
        let lo = u64::from_le_bytes(lo.try_into().unwrap_or_default());
        state.write_u64(lo ^ hi.rotate_left(23) ^ (u64::from(self.tag) << 56) ^ u64::from(self.ty));
    }
}

/// The adjacency changes of a change set, by list (the staging area of the relations, as
/// `substring_pending` / `text_pending` are for the text indexes): a relation reads the count of
/// a list once, from the transaction, and every later one of the same list in the change set
/// finds it here. Nothing is written before [`ChangeSet::flush_adjacency`], which builds the
/// list keys once and writes the lists in key order.
#[derive(Default)]
pub(crate) struct PendingAdjacency {
    /// The relation types the change set touched, by their interned number.
    types: Vec<Box<str>>,
    type_ids: std::collections::HashMap<Box<str>, u32>,
    /// The type interned last: a bulk load relates with one type over and over.
    last_type: u32,
    /// Position in `lists` of each list.
    index: std::collections::HashMap<ListId, usize, std::hash::BuildHasherDefault<IdHasher>>,
    lists: Vec<(ListId, AdjList)>,
}

impl PendingAdjacency {
    /// The number of the relation type `t`, once it was interned.
    fn type_id(&self, t: &str) -> Option<u32> {
        match self.types.get(self.last_type as usize) {
            Some(last) if **last == *t => Some(self.last_type),
            _ => self.type_ids.get(t).copied(),
        }
    }

    /// The number of the relation type `t`, interning it the first time.
    fn intern(&mut self, t: &str) -> u32 {
        let id = match self.type_id(t) {
            Some(id) => id,
            None => {
                let id = self.types.len() as u32;
                self.types.push(t.into());
                self.type_ids.insert(t.into(), id);
                id
            }
        };
        self.last_type = id;
        id
    }

    /// The list `(unit, dir, relation_type)`, interning the type.
    pub(crate) fn list_id(&mut self, unit: UnitId, dir: Direction, relation_type: &str) -> ListId {
        ListId { unit, tag: tag_of(dir), ty: self.intern(relation_type) }
    }

    /// What the change set keeps of the list `id`, if anything.
    pub(crate) fn get(&self, id: &ListId) -> Option<&AdjList> {
        self.index.get(id).map(|&at| &self.lists[at].1)
    }

    /// What the change set keeps of the list `(unit, tag, relation_type)`, if anything.
    fn find(&self, unit: UnitId, tag: u8, relation_type: &str) -> Option<&AdjList> {
        self.get(&ListId { unit, tag, ty: self.type_id(relation_type)? })
    }

    /// The list `id`, made by `make` when the change set does not keep it yet.
    fn entry(&mut self, id: ListId, make: impl FnOnce() -> AdjList) -> &mut AdjList {
        let lists = &mut self.lists;
        let at = *self.index.entry(id).or_insert_with(|| {
            lists.push((id, make()));
            lists.len() - 1
        });
        &mut self.lists[at].1
    }

    /// The interned types in ascending order of their bytes: the order of their list keys.
    fn types_ascending(&self) -> Vec<u32> {
        let mut order: Vec<u32> = (0..self.types.len() as u32).collect();
        order.sort_unstable_by(|a, b| self.types[*a as usize].cmp(&self.types[*b as usize]));
        order
    }

    /// The rank of each interned type in [`types_ascending`](Self::types_ascending), by number.
    fn type_ranks(&self) -> Vec<u32> {
        let mut rank = vec![0u32; self.types.len()];
        for (r, ty) in self.types_ascending().into_iter().enumerate() {
            rank[ty as usize] = r as u32;
        }
        rank
    }
}

/// The adjacency blocks as [`crate::blocks`] stores them.
struct Adjacency;

impl Block for Adjacency {
    type Val = UnitId;
    const MAX: usize = ADJ_BLOCK_MAX;
    const WHAT: &'static str = "REL_ADJ";
    const ENTRY: usize = 32;

    fn decode(v: &[u8]) -> Result<Vec<(Id, UnitId)>, DecodeError> {
        Ok(AdjBlock::decode(v)?.entries.into_iter().map(|(e, o)| (e.0, o)).collect())
    }

    fn encode(entries: &[(Id, UnitId)]) -> Vec<u8> {
        AdjBlock { entries: entries.iter().map(|(e, o)| (RelationId(*e), *o)).collect() }.encode()
    }

    fn put_val(out: &mut Vec<u8>, other: &UnitId) {
        out.extend_from_slice(&other.0);
    }
}

pub(crate) fn tag_of(dir: Direction) -> u8 {
    match dir {
        Direction::Out => TAG_REL_OUT,
        Direction::In => TAG_REL_IN,
    }
}

/// `tag ‖ unit ‖ type ‖ 0x00`: the prefix of the blocks of one list.
pub(crate) fn list_key(tag: u8, unit: UnitId, relation_type: &str) -> Vec<u8> {
    let mut k = Vec::with_capacity(18 + relation_type.len());
    k.push(tag);
    k.extend_from_slice(&unit.0);
    k.extend_from_slice(relation_type.as_bytes());
    k.push(0);
    k
}

impl<E: TxnEngine> ChangeSet<'_, E> {
    /// Keeps the adjacency entries and the counts of the relation `edge` from `from` to `to`
    /// (`add`: the entries are put, else removed) for [`flush_adjacency`](Self::flush_adjacency).
    /// `plans` are the `(from, Out)` and `(to, In)` lists as
    /// [`plan_relation`](Self::plan_relation) planned them: a list that was empty has no block to
    /// read.
    pub(crate) fn stage_adjacency(
        &mut self,
        edge: RelationId,
        (from, to): (UnitId, UnitId),
        plans: [ListPlan; 2],
        add: bool,
    ) {
        let [out, inn] = plans;
        for (plan, other) in [(out, to), (inn, from)] {
            let list = self.adj_pending.entry(plan.id, || AdjList {
                fresh: add && plan.before == 0,
                count: plan.before,
                dirty: false,
                // Most lists of a bulk load take several entries: no growth from one.
                ops: Vec::with_capacity(4),
            });
            list.count = plan.next;
            list.dirty |= plan.dirty;
            let op = (edge.0, add.then_some(other));
            // Ids are minted ascending: almost always an append.
            match list.ops.last() {
                Some(last) if last.0 >= edge.0 => match list.ops.binary_search_by_key(&edge.0, |o| o.0) {
                    Ok(at) => list.ops[at] = op,
                    Err(at) => list.ops.insert(at, op),
                },
                _ => list.ops.push(op),
            }
        }
    }

    /// Writes the adjacency changes and the counts the change set kept, once per list. Called by
    /// `commit`.
    ///
    /// The blocks first, every list in ascending key order, then the count records in ascending
    /// key order: the transaction gets its writes as a few sorted runs, and a seek of the block
    /// flush never lands below a write of this flush (the counts, tag `0x72`, sort after every
    /// block), so it never has to order the writes before it.
    pub(crate) fn flush_adjacency(&mut self) -> Result<(), UnitError> {
        let pending = std::mem::take(&mut self.adj_pending);
        if pending.lists.is_empty() {
            return Ok(());
        }
        // A list key is `tag ‖ unit ‖ type ‖ 0x00` and a type has no NUL, so the keys sort as
        // `(tag, unit, type)` and the count keys (`0x72 ‖ unit ‖ direction ‖ type ‖ 0x00`) as
        // `(unit, direction, type)`: both orders come from the ids, without comparing keys.
        let rank = pending.type_ranks();
        let (types, mut lists) = (pending.types, pending.lists);
        // Sorted as small keys, the lists staying where they are (`at`): the blocks by
        // `(tag, unit, type rank)`, the counts by `(unit, tag, type rank)`.
        let mut blocks_order: Vec<(u8, u128, u32, u32)> = Vec::with_capacity(lists.len());
        let mut counts_order: Vec<(u128, u8, u32, u32)> = Vec::with_capacity(lists.len());
        for (at, (id, l)) in lists.iter().enumerate() {
            let (unit, ty, at) = (u128::from_be_bytes(id.unit.0), rank[id.ty as usize], at as u32);
            blocks_order.push((id.tag, unit, ty, at));
            if l.dirty {
                counts_order.push((unit, id.tag, ty, at));
            }
        }
        blocks_order.sort_unstable();
        counts_order.sort_unstable();
        for (.., at) in blocks_order {
            let (id, l) = &mut lists[at as usize];
            let list = list_key(id.tag, id.unit, &types[id.ty as usize]);
            let ops = std::mem::take(&mut l.ops);
            blocks::flush::<Adjacency, E>(self, &list, ops, l.fresh, None, None)?;
        }
        for (.., at) in counts_order {
            let (id, l) = &lists[at as usize];
            let dir = if id.tag == TAG_REL_OUT { Direction::Out } else { Direction::In };
            let k = count_key(id.unit, dir, &types[id.ty as usize]);
            self.write_owned(if l.count == 0 { Write::Delete(k) } else { Write::Put(k, encode_value(l.count)) })?;
        }
        Ok(())
    }
}

impl<E: TxnEngine> ChangeSet<'_, E> {
    /// A relation that starts or ends at `unit` as this change set sees it
    /// (the blocks of the transaction and the changes the change set keeps),
    /// if there is one: two cheap scans, one per direction.
    pub(crate) fn relation_of_unit(&self, unit: UnitId) -> Result<Option<RelationId>, UnitError> {
        for tag in [TAG_REL_OUT, TAG_REL_IN] {
            let Some((start, end)) = Key::adjacency_range(tag, unit, None) else {
                continue;
            };
            // An entry the change set adds is a relation, whatever the blocks say (the lists in
            // key order, which is the order of their types).
            let pending = &self.adj_pending;
            let added = pending.types_ascending().into_iter().find_map(|ty| {
                pending
                    .get(&ListId { unit, tag, ty })?
                    .ops
                    .iter()
                    .find_map(|(edge, other)| other.map(|_| RelationId(*edge)))
            });
            if added.is_some() {
                return Ok(added);
            }
            // Else one the blocks hold and the change set does not remove.
            let mut from = start;
            let mut inclusive = true;
            loop {
                let lo = if inclusive { Bound::Included(&from[..]) } else { Bound::Excluded(&from[..]) };
                let page = self.txn()?.scan(lo, Bound::Excluded(&end), PAGE)?;
                for (k, v) in &page {
                    let (Key::RelationOut { relation_type, .. } | Key::RelationIn { relation_type, .. }) =
                        Key::decode(k)?
                    else {
                        return Err(crate::db::corruption(
                            "a key in an adjacency range is not an adjacency key".into(),
                        ));
                    };
                    let removed = self.adj_pending.find(unit, tag, &relation_type);
                    let block = AdjBlock::decode(v).map_err(|e| context(e, "REL_ADJ"))?;
                    if let Some((edge, _)) = block
                        .entries
                        .iter()
                        .find(|(e, _)| removed.is_none_or(|l| l.ops.binary_search_by_key(&e.0, |o| o.0).is_err()))
                    {
                        return Ok(Some(*edge));
                    }
                }
                match page.last() {
                    Some((k, _)) if page.len() == PAGE => {
                        from = k.clone();
                        inclusive = false;
                    }
                    _ => break,
                }
            }
        }
        Ok(None)
    }
}

impl<E: kerno_core::contract::Engine> UnitSnapshot<E> {
    /// The unit at the other end of the relation `edge` in the adjacency list
    /// of `unit` (`dir`, `relation_type`), or `None` when the list has no
    /// entry for it: one seek to the block of the relation.
    pub(crate) fn adjacency_entry(
        &self,
        dir: Direction,
        unit: UnitId,
        relation_type: &str,
        edge: RelationId,
    ) -> Result<Option<UnitId>, UnitError> {
        let list = list_key(tag_of(dir), unit, relation_type);
        // Most lists are one block, the tail: a point read finds it, and when the relation is
        // not below its first entry the tail is the block of the relation. Else seek.
        let decode = |v: &[u8]| AdjBlock::decode(v).map_err(|e| context(e, "REL_ADJ"));
        let tail = match self.snap.get(&blocks::block_key(&list, None))? {
            Some(v) => Some(decode(&v)?).filter(|b| b.entries[0].0 <= edge),
            None => None,
        };
        let block = match tail {
            Some(b) => b,
            None => {
                let (_, end) = crate::schema::prefix_bounds(&list);
                let from = blocks::block_key(&list, Some(&edge.0));
                let page = self.snap.scan(Bound::Included(&from), Bound::Excluded(&end), 1)?;
                let Some((_, v)) = page.into_iter().next() else {
                    return Ok(None);
                };
                decode(&v)?
            }
        };
        Ok(block.entries.binary_search_by_key(&edge, |e| e.0).ok().map(|at| block.entries[at].1))
    }
}

/// One edit of an adjacency list, for [`UnitDb::adjacency_patch`]: the direction, unit and
/// relation type of the list, the relation, and the unit at its other end (`None`: remove it).
#[cfg(any(test, feature = "test-support"))]
pub type AdjEdit<'a> = (Direction, UnitId, &'a str, RelationId, Option<UnitId>);

/// One raw write of [`UnitDb::adjacency_patch`]: a key and its new value (`None`: delete).
#[cfg(any(test, feature = "test-support"))]
pub type RawWrite = (Vec<u8>, Option<Vec<u8>>);

#[cfg(any(test, feature = "test-support"))]
impl<E: TxnEngine> crate::db::UnitDb<E> {
    /// The raw writes that make `edits` to the adjacency lists, as the blocks they fall in
    /// would be rewritten: for tests that forge or repair damage under the writer's back.
    /// Edits of one block are combined into one write; each must keep its block within
    /// [`ADJ_BLOCK_MAX`] entries, and a removal that empties a block deletes it.
    #[doc(hidden)]
    pub fn adjacency_patch(&self, edits: &[AdjEdit<'_>]) -> Result<Vec<RawWrite>, UnitError> {
        let snap = self.raw_snapshot()?;
        let mut blocks_by_key: std::collections::BTreeMap<Vec<u8>, Vec<(RelationId, UnitId)>> = Default::default();
        for (dir, unit, relation_type, edge, other) in edits {
            let list = list_key(tag_of(*dir), *unit, relation_type);
            let (_, end) = crate::schema::prefix_bounds(&list);
            let from = blocks::block_key(&list, Some(&edge.0));
            let stored = snap.scan(Bound::Included(&from), Bound::Excluded(&end), 1)?.into_iter().next();
            // A list that has no block yet starts at its tail, which a later edit finds again.
            let key = stored.as_ref().map_or_else(|| blocks::block_key(&list, None), |(k, _)| k.clone());
            let entries = blocks_by_key.entry(key).or_insert_with(|| match &stored {
                Some((_, v)) => AdjBlock::decode(v).map(|b| b.entries).unwrap_or_default(),
                None => Vec::new(),
            });
            entries.retain(|e| e.0 != *edge);
            if let Some(o) = other {
                entries.push((*edge, *o));
                entries.sort_by_key(|e| e.0);
            }
        }
        Ok(blocks_by_key
            .into_iter()
            .map(|(k, entries)| (k, (!entries.is_empty()).then(|| AdjBlock { entries }.encode())))
            .collect())
    }
}
