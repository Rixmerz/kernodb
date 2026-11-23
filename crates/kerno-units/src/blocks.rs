//! Posting blocks, the mechanics shared by the trigram index
//! ([`crate::substring`], ADR-0025), the keyword index ([`crate::fulltext`])
//! and the adjacency index ([`crate::codec`], `REL_OUT` / `REL_IN`), ADR-0026.
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
//! `list ‖ 0x00 ‖ id`: one forward seek. The list key must end where no byte
//! of the list's own name can be mistaken for the marker (a fixed length, or
//! a name with no NUL followed by a NUL).
//!
//! A change set keeps what it changes per list ([`flush`]) and writes each
//! block it touches once, at commit.

use std::collections::BTreeMap;
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
    /// The entries of a block value: at least one, at most [`MAX`](Self::MAX), strictly ascending.
    fn decode(v: &[u8]) -> Result<Vec<(Id, Self::Val)>, DecodeError>;
    /// The block value of `entries`, ascending.
    fn encode(entries: &[(Id, Self::Val)]) -> Vec<u8>;
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

/// Checks that `ids` are strictly ascending.
pub(crate) fn ascending<T>(entries: &[(Id, T)], what: &str) -> Result<(), DecodeError> {
    if entries.windows(2).any(|w| w[0].0 >= w[1].0) {
        return Err(DecodeError::new(UnitRule::Unit13, format!("{what}: ids not strictly ascending")));
    }
    Ok(())
}

/// Writes the changes `ops` to the list `list`: per id, `Some(value)` puts the
/// entry (or replaces its value) and `None` removes it. Each block an id
/// falls in is read once, changed for every id of `ops` in it, then written
/// back, cut when over [`Block::MAX`] entries (from the low end, each bounded
/// by its last id; the rest keeps the key it had) and deleted when emptied.
///
/// `fresh`: the caller knows the list has no block, so none is read.
pub(crate) fn flush<B: Block, E: TxnEngine>(
    cs: &mut ChangeSet<'_, E>,
    list: &[u8],
    ops: BTreeMap<Id, Option<B::Val>>,
    fresh: bool,
) -> Result<(), UnitError> {
    if fresh {
        let units: Vec<(Id, B::Val)> = ops.into_iter().filter_map(|(id, v)| v.map(|v| (id, v))).collect();
        return write_chunks::<B, E>(cs, list, units, block_key(list, None));
    }
    let (_, end) = prefix_bounds(list);
    let mut ops = ops.into_iter().peekable();
    while let Some((first, _)) = ops.peek() {
        let from = block_key(list, Some(first));
        let page = cs.txn()?.scan(Bound::Included(&from), Bound::Excluded(&end), 1)?;
        let (block, mut units, bound) = match page.into_iter().next() {
            Some((k, v)) => {
                let units = B::decode(&v).map_err(|e| context(e, B::WHAT))?;
                let bound = bound_of(list, &k).map_err(|e| context(e, B::WHAT))?;
                (Some(k), units, bound)
            }
            None => (None, Vec::new(), None),
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

/// Writes `units` as blocks of at most [`Block::MAX`]: full blocks from the
/// low end, each bounded by its last id, and the rest under `last_key`.
fn write_chunks<B: Block, E: TxnEngine>(
    cs: &mut ChangeSet<'_, E>,
    list: &[u8],
    units: Vec<(Id, B::Val)>,
    last_key: Vec<u8>,
) -> Result<(), UnitError> {
    let mut chunks = units.chunks(B::MAX).peekable();
    while let Some(chunk) = chunks.next() {
        let k = if chunks.peek().is_some() { block_key(list, chunk.last().map(|e| &e.0)) } else { last_key.clone() };
        cs.write_owned(Write::Put(k, B::encode(chunk)))?;
    }
    Ok(())
}
