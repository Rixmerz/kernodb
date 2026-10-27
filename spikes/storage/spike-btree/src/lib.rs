//! Copy-on-write B+ tree candidate for the Kerno storage spike (Phase 0).
//!
//! LMDB/redb-style shadow paging over a single file of 4 KiB pages
//! (`data.kbt`), written only through the [`spike_core::Vfs`]:
//!
//! * pages 0 and 1 are double-buffered meta pages (a commit writes the slot
//!   not holding the current meta; open picks the valid one with the highest
//!   sequence; a clean close writes both);
//! * the tree is Merkle-checksummed: every branch entry stores its child's
//!   page id and crc32c, the meta stores the root's, leaf entries of large
//!   values store the length and a crc over their overflow chain, and the
//!   persisted free list is a crc-linked chain hanging off the meta;
//! * a commit copies each touched node once, writes new pages bottom-up,
//!   then (with [`spike_core::Durability::Sync`]) `sync_data`, writes the
//!   meta into the other slot and `sync_data` again — two fsyncs per commit;
//! * pages freed by commit `n` are reused only by later commits and only once
//!   no live snapshot is older than `n`; the free set is persisted as a
//!   crc-linked log so reopening never leaks pages;
//! * [`Durability::Buffered`](spike_core::Durability::Buffered) skips both
//!   fsyncs: safe against process crashes, but a power loss yields either a
//!   consistent prefix or a detected corruption error (not a guaranteed
//!   prefix).
//!
//! [`BTreeEngine::verify`] walks the whole file and checks every checksum and
//! that each page is used exactly once (tree, overflow, free-list log or
//! free).
//!
//! See `DESIGN.md` next to this crate for the on-disk format, the recovery
//! procedure and the known limitations.

mod engine;
mod freelist;
mod meta;
mod page;
mod tree;

pub use engine::{BTreeEngine, BTreeSnapshot, VerifyReport};

#[cfg(test)]
mod tests;
