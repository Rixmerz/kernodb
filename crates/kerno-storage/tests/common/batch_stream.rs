//! A deterministic stream of batches, `batch_for(1)`, `batch_for(2)`, …, for
//! the tests that run a writer in another process (`process_kill.rs`,
//! `lazyfs.rs`): the writer commits batch `n` as sequence `n`, and the
//! checker rebuilds the expected contents after any prefix from the numbers
//! alone, so no state has to travel between the two processes.
//!
//! Included with `#[path]` by each target; none uses all of it.
#![allow(dead_code)]

use kerno_core::WriteBatch;
use kerno_core::contract::{Durability, Options};
use kerno_storage::Config;
use kerno_testkit::rng::Rng;

/// Distinct keys: few enough that overwrites, deletes and compactions happen.
pub const KEYS: u64 = 512;

pub fn key(i: u64) -> Vec<u8> {
    format!("k{i:04}").into_bytes()
}

/// The batch committed with sequence `seq`: 1 to 4 puts or deletes, and now
/// and then a range deletion.
pub fn batch_for(seq: u64) -> WriteBatch {
    let mut rng = Rng::new(seq);
    let mut b = WriteBatch::new();
    for _ in 0..rng.range(1, 4) {
        let k = key(rng.below(KEYS));
        if rng.chance(1, 5) {
            b.delete(k);
        } else {
            let len = rng.range(40, 240) as usize;
            b.put(k, rng.bytes(len));
        }
    }
    if rng.chance(1, 40) {
        let start = rng.below(KEYS);
        b.delete_range(key(start), key((start + rng.range(1, 24)).min(KEYS)));
    }
    b
}

/// Tiny buffers, so that a writer's lifetime includes flushes and compactions.
pub fn config(durability: Durability) -> Config {
    Config::from(&Options { durability, cache_bytes: 64 << 10, write_buffer_bytes: 16 << 10 })
}

pub fn mode_name(durability: Durability) -> &'static str {
    match durability {
        Durability::Sync => "sync",
        Durability::Buffered => "buffered",
    }
}

pub fn mode_from(name: &str) -> Durability {
    match name {
        "sync" => Durability::Sync,
        "buffered" => Durability::Buffered,
        other => panic!("unknown durability mode {other:?}"),
    }
}
