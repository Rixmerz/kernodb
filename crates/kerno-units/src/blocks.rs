//! Posting blocks, the mechanics of the adjacency index ([`crate::codec`], `REL_OUT` /
//! `REL_IN`, ADR-0026). The trigram index and the keyword index shared them until layer
//! version 11, where they became posting runs ([`crate::runs`], ADR-0027).
//!
//! A *list* is a sorted run of entries `(id, value)` kept under one key
//! prefix (the list key), cut in blocks of at most [`Block::MAX`] entries, one
//! engine record each:
//!
//! * a bounded block: `list ‖ 0x00 ‖ bound[16]`, holding the entries whose id
//!   is above the bound of the block before it and at most its own bound;
//! * the tail block: `list ‖ 0x01`, the last one, with no bound.
//!
//! The block of an id is therefore the first key at or after
//! `list ‖ 0x00 ‖ id`: one forward seek. The tail holds every id above the last
//! bound, so when its first entry is at or below an id the tail is the block of
//! that id, and a point read of the tail key finds it without the seek. The list
//! key must end where no byte of the list's own name can be mistaken for the
//! marker (a fixed length, or a name with no NUL followed by a NUL).
//!
//! A change set keeps what it changes per list ([`flush`]) and writes each
//! block it touches once, at commit.

use std::ops::Bound;

use kerno_core::contract::{Txn, TxnEngine};

use crate::db::{ChangeSet, Write, context};
use crate::error::{DecodeError, UnitError, UnitRule};
use crate::schema::prefix_bounds;

/// An entry id: a unit id or a relation id.
pub(crate) type Id = [u8; 16];

/// The byte after the list key of a bounded block, and of the tail.
pub(crate) const BOUNDED: u8 = 0x00;
pub(crate) const TAIL: u8 = 0x01;

/// What a family of blocks stores and how it encodes it.
pub(crate) trait Block {
    /// What an entry carries after its id (`()` for none).
    type Val: Copy;
    /// Most entries in one block.
    const MAX: usize;
    /// The record family, for the context of a decode error.
    const WHAT: &'static str;
    /// Bytes of one entry in a block value: its id, then its value.
    const ENTRY: usize;
    /// The entries of a block value: at least one, at most [`MAX`](Self::MAX), strictly ascending.
    fn decode(v: &[u8]) -> Result<Vec<(Id, Self::Val)>, DecodeError>;
    /// The block value of `entries`, ascending.
    fn encode(entries: &[(Id, Self::Val)]) -> Vec<u8>;
    /// Appends the bytes of an entry's value, as [`encode`](Self::encode) writes them after
    /// the id (`ENTRY - 16` bytes).
    fn put_val(out: &mut Vec<u8>, val: &Self::Val);
    /// Whether `bytes` (`ENTRY - 16` of them) are a value [`decode`](Self::decode) accepts.
    fn val_ok(_bytes: &[u8]) -> bool {
        true
    }
}

/// The id of the last entry of the block value `v` when `v` is a block [`Block::decode`]
/// accepts, judged without building its entries: the envelope, a whole number of entries
/// between 1 and [`Block::MAX`], ids strictly ascending, every value valid. `None` otherwise.
fn last_id_if_valid<B: Block>(v: &[u8]) -> Option<Id> {
    let entries = body(v, B::WHAT).ok()?;
    if entries.is_empty() || !entries.len().is_multiple_of(B::ENTRY) || entries.len() / B::ENTRY > B::MAX {
        return None;
    }
    let mut prev: Option<&[u8]> = None;
    for e in entries.chunks_exact(B::ENTRY) {
        let (id, val) = e.split_at(16);
        if prev.is_some_and(|p| p >= id) || !B::val_ok(val) {
            return None;
        }
        prev = Some(id);
    }
    prev.and_then(|id| id.try_into().ok())
}

/// The key of the block of `list` with the bound `bound` (`None`: the tail).
pub(crate) fn block_key(list: &[u8], bound: Option<&Id>) -> Vec<u8> {
    let mut k = Vec::with_capacity(list.len() + 17);
    k.extend_from_slice(list);
    match bound {
        Some(id) => {
            k.push(BOUNDED);
            k.extend_from_slice(id);
        }
        None => k.push(TAIL),
    }
    k
}

/// The bound of the block at `key` of the list `list` (`None`: the tail).
pub(crate) fn bound_of(list: &[u8], key: &[u8]) -> Result<Option<Id>, DecodeError> {
    match key.get(list.len()..) {
        Some([TAIL]) => Ok(None),
        Some([BOUNDED, rest @ ..]) if rest.len() == 16 => {
            let mut id = [0u8; 16];
            id.copy_from_slice(rest);
            Ok(Some(id))
        }
        _ => Err(DecodeError::new(UnitRule::Unit3, format!("block key of {} bytes has no valid bound", key.len()))),
    }
}

/// Checks a block value's envelope and returns its body. `what` names the
/// family in a message. The version comes first (a newer one is a newer
/// layer, not damage), then the flags.
pub(crate) fn body<'a>(v: &'a [u8], what: &str) -> Result<&'a [u8], DecodeError> {
    let e = |rule, m: &str| DecodeError::new(rule, format!("{what}: {m}"));
    match v {
        [] | [_] => Err(e(UnitRule::Unit9, "fewer bytes than its envelope")),
        [0, ..] => Err(e(UnitRule::Unit6, "envelope_version 0")),
        [1, 0, ..] => Ok(&v[2..]),
        [1, _, ..] => Err(e(UnitRule::Unit8, "reserved_flags set")),
        [_, ..] => Err(e(UnitRule::Unit7, "a newer envelope_version")),
    }
}

/// Writes the changes `ops` to the list `list`: per id, `Some(value)` puts the
/// entry (or replaces its value) and `None` removes it. Each block an id
/// falls in is read once, changed for every id of `ops` in it, then written
/// back, cut when over [`Block::MAX`] entries (from the low end, each bounded
/// by its last id; the rest keeps the key it had) and deleted when emptied.
///
/// `ops` are ascending by id, one per id. `fresh`: the caller knows the list
/// has no block, so none is read.
pub(crate) fn flush<B: Block, E: TxnEngine>(
    cs: &mut ChangeSet<'_, E>,
    list: &[u8],
    ops: impl IntoIterator<Item = (Id, Option<B::Val>)>,
    fresh: bool,
) -> Result<(), UnitError> {
    if fresh {
        let units: Vec<(Id, B::Val)> = ops.into_iter().filter_map(|(id, v)| v.map(|v| (id, v))).collect();
        return write_chunks::<B, E>(cs, list, units, block_key(list, None));
    }
    let tail_key = block_key(list, None);
    let mut ops = ops.into_iter().peekable();
    let Some(&(first, _)) = ops.peek() else {
        return Ok(());
    };
    // The tail holds every id above the last bound, so when its first entry is at or below the
    // lowest id to change, it is the block of all of them: one point read finds it where the seek
    // below walks the engine's levels. Ids are minted in ascending order, so the entries a change
    // set adds are almost always in the tail.
    let stored_tail = cs.txn()?.get(&tail_key)?;
    let (_, end) = prefix_bounds(list);
    let mut tail = match stored_tail {
        // Every id to change is above the last entry of the tail (the ids ascend), so above
        // every entry of the list: the puts are appended to the tail's bytes and a removal
        // has nothing to remove. This writes what the general path below writes (the tail's
        // entries, then the new ones, cut the same way) without building the entries.
        Some(v) if last_id_if_valid::<B>(&v).is_some_and(|last| last < first) => {
            return append::<B, E>(cs, list, v, ops, tail_key);
        }
        // A tail that does not decode is not used here: the seek reads it if the change needs
        // it, and reports it then.
        Some(v) => B::decode(&v).ok(),
        None => None,
    };
    while let Some((first, _)) = ops.peek() {
        let (block, mut units, bound) = match tail.take_if(|u| u.first().is_some_and(|e| e.0 <= *first)) {
            Some(units) => (Some(tail_key.clone()), units, None),
            None => {
                let from = block_key(list, Some(first));
                let page = cs.txn()?.scan(Bound::Included(&from), Bound::Excluded(&end), 1)?;
                match page.into_iter().next() {
                    Some((k, v)) => {
                        let units = B::decode(&v).map_err(|e| context(e, B::WHAT))?;
                        let bound = bound_of(list, &k).map_err(|e| context(e, B::WHAT))?;
                        (Some(k), units, bound)
                    }
                    None => (None, Vec::new(), None),
                }
            }
        };
        while let Some((id, op)) = ops.next_if(|(u, _)| bound.is_none_or(|b| *u <= b)) {
            match (units.binary_search_by_key(&id, |e| e.0), op) {
                (Err(at), Some(v)) => units.insert(at, (id, v)),
                (Ok(at), Some(v)) => units[at].1 = v,
                (Ok(at), None) => {
                    units.remove(at);
                }
                (Err(_), None) => {}
            }
        }
        let key = block.clone().unwrap_or_else(|| block_key(list, bound.as_ref()));
        if units.is_empty() {
            if block.is_some() {
                cs.write_owned(Write::Delete(key))?;
            }
            continue;
        }
        write_chunks::<B, E>(cs, list, units, key)?;
    }
    Ok(())
}

/// Appends the puts of `ops`, all above the last entry of the tail value `tail`, to it, and
/// writes the result as [`write_chunks`] does: full blocks from the low end, each bounded by
/// its last id, and the rest under `tail_key`.
fn append<B: Block, E: TxnEngine>(
    cs: &mut ChangeSet<'_, E>,
    list: &[u8],
    mut tail: Vec<u8>,
    ops: impl Iterator<Item = (Id, Option<B::Val>)>,
    tail_key: Vec<u8>,
) -> Result<(), UnitError> {
    let before = tail.len();
    tail.reserve(ops.size_hint().0 * B::ENTRY);
    for (id, op) in ops {
        if let Some(val) = op {
            tail.extend_from_slice(&id);
            B::put_val(&mut tail, &val);
        }
    }
    if tail.len() == before {
        // Only removals of entries the list does not have.
        return Ok(());
    }
    let block_len = B::MAX * B::ENTRY;
    if tail.len() - 2 <= block_len {
        return cs.write_owned(Write::Put(tail_key, tail));
    }
    let (envelope, entries) = tail.split_at(2);
    let mut chunks = entries.chunks(block_len).peekable();
    let mut tail_key = Some(tail_key);
    while let Some(chunk) = chunks.next() {
        let key = if chunks.peek().is_some() {
            let last = &chunk[chunk.len() - B::ENTRY..][..16];
            block_key(list, last.try_into().ok())
        } else {
            tail_key.take().unwrap_or_default()
        };
        let mut v = Vec::with_capacity(2 + chunk.len());
        v.extend_from_slice(envelope);
        v.extend_from_slice(chunk);
        cs.write_owned(Write::Put(key, v))?;
    }
    Ok(())
}

/// Writes `units` as blocks of at most [`Block::MAX`]: full blocks from the
/// low end, each bounded by its last id, and the rest under `last_key`.
fn write_chunks<B: Block, E: TxnEngine>(
    cs: &mut ChangeSet<'_, E>,
    list: &[u8],
    units: Vec<(Id, B::Val)>,
    mut last_key: Vec<u8>,
) -> Result<(), UnitError> {
    let mut chunks = units.chunks(B::MAX).peekable();
    while let Some(chunk) = chunks.next() {
        let k = if chunks.peek().is_some() {
            block_key(list, chunk.last().map(|e| &e.0))
        } else {
            std::mem::take(&mut last_key)
        };
        cs.write_owned(Write::Put(k, B::encode(chunk)))?;
    }
    Ok(())
}
