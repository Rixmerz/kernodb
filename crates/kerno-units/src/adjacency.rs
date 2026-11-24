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
//! adjacency changes in the change set. [`ChangeSet::flush_adjacency`] writes
//! them at commit: each block a list changes is read once and written once
//! (cut when over the limit, deleted when emptied). A list the change set
//! knows to be empty has no block to read: its blocks are written directly.
//! The change set knows it from the count record of the list, and when that
//! record is missing from the blocks themselves: the writer counts them with
//! one seek (`adj_count::count_entries`), so a list whose record was lost is
//! never taken for an empty one and its tail block is never written over
//! unread.
//!
//! A read inside the change set ([`ChangeSet::relation_of_unit`]) sees both:
//! the blocks of the transaction and what the change set keeps.

use std::collections::BTreeMap;
use std::ops::Bound;

use kerno_core::contract::{Snapshot, Txn, TxnEngine};

use crate::blocks::{self, Block, Id};
use crate::codec::{ADJ_BLOCK_MAX, AdjBlock, Key, TAG_REL_IN, TAG_REL_OUT};
use crate::db::{ChangeSet, UnitSnapshot, context};
use crate::error::{DecodeError, UnitError};
use crate::types::{Direction, RelationId, UnitId};

/// Blocks read per scan call when looking for a surviving entry.
const PAGE: usize = 16;

/// What a change set will change in one adjacency list.
pub(crate) struct AdjList {
    /// The list has no block in the transaction: nothing to read before writing it.
    fresh: bool,
    /// Per relation, the unit at the other end (`Some`: the entry is put) or
    /// `None` (the entry is removed); the last operation of a relation wins.
    ops: BTreeMap<Id, Option<UnitId>>,
}

/// The adjacency changes of a change set, by list key.
pub(crate) type PendingAdjacency = BTreeMap<Vec<u8>, AdjList>;

/// The adjacency blocks as [`crate::blocks`] stores them.
struct Adjacency;

impl Block for Adjacency {
    type Val = UnitId;
    const MAX: usize = ADJ_BLOCK_MAX;
    const WHAT: &'static str = "REL_ADJ";

    fn decode(v: &[u8]) -> Result<Vec<(Id, UnitId)>, DecodeError> {
        Ok(AdjBlock::decode(v)?.entries.into_iter().map(|(e, o)| (e.0, o)).collect())
    }

    fn encode(entries: &[(Id, UnitId)]) -> Vec<u8> {
        AdjBlock { entries: entries.iter().map(|(e, o)| (RelationId(*e), *o)).collect() }.encode()
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
    /// Keeps the adjacency entries of the relation `edge` from `from` to `to`
    /// of type `relation_type` (`add`: they are put, else removed) for
    /// [`flush_adjacency`](Self::flush_adjacency). `before` are the counts of
    /// the `(from, Out)` and `(to, In)` lists as they were before the
    /// operation (what [`adjust_counts`](Self::adjust_counts) returns): a
    /// list that was empty has no block to read.
    pub(crate) fn stage_adjacency(
        &mut self,
        edge: RelationId,
        (from, to): (UnitId, UnitId),
        relation_type: &str,
        add: bool,
        before: (u64, u64),
    ) {
        for (tag, unit, other, count) in [(TAG_REL_OUT, from, to, before.0), (TAG_REL_IN, to, from, before.1)] {
            let list = self
                .adj_pending
                .entry(list_key(tag, unit, relation_type))
                .or_insert_with(|| AdjList { fresh: add && count == 0, ops: BTreeMap::new() });
            list.ops.insert(edge.0, add.then_some(other));
        }
    }

    /// Writes the adjacency changes the change set kept. Called by `commit`.
    pub(crate) fn flush_adjacency(&mut self) -> Result<(), UnitError> {
        for (list, l) in std::mem::take(&mut self.adj_pending) {
            blocks::flush::<Adjacency, E>(self, &list, l.ops, l.fresh)?;
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
            // An entry the change set adds is a relation, whatever the blocks say.
            let added = self
                .adj_pending
                .range(start.clone()..end.clone())
                .find_map(|(_, l)| l.ops.iter().find_map(|(edge, other)| other.map(|_| RelationId(*edge))));
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
                    let removed = self.adj_pending.get(&list_key(tag, unit, &relation_type));
                    let block = AdjBlock::decode(v).map_err(|e| context(e, "REL_ADJ"))?;
                    if let Some((edge, _)) =
                        block.entries.iter().find(|(e, _)| removed.is_none_or(|l| !l.ops.contains_key(&e.0)))
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
        let mut blocks_by_key: BTreeMap<Vec<u8>, Vec<(RelationId, UnitId)>> = BTreeMap::new();
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
