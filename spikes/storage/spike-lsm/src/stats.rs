//! Engine counters reported through `Engine::stats`.

use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

macro_rules! counters {
    ($($name:ident),* $(,)?) => {
        #[derive(Default, Debug)]
        pub struct Stats {
            $(pub $name: AtomicU64,)*
        }

        impl Stats {
            pub fn report(&self) -> Vec<(String, u64)> {
                vec![$((stringify!($name).to_string(), self.$name.load(Relaxed)),)*]
            }
        }
    };
}

counters!(
    commits,
    wal_bytes,
    wal_syncs,
    memtable_rotations,
    flushes,
    flush_bytes_written,
    compactions,
    trivial_moves,
    compaction_bytes_read,
    compaction_bytes_written,
    tombstones_dropped,
    write_stalls,
    write_stall_micros,
    write_slowdowns,
    bloom_checks,
    bloom_negatives,
    bloom_false_positives,
    block_cache_hits,
    block_cache_misses,
    manifest_writes,
    obsolete_files_deleted,
);

pub fn bump(c: &AtomicU64, n: u64) {
    c.fetch_add(n, Relaxed);
}
