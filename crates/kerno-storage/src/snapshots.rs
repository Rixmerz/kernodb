//! The registry of live snapshots (phase1-design.md §5.7): what the
//! `snapshot_*` metrics report.
//!
//! The registry is shared (`Arc`) by the engine and by every snapshot it
//! hands out, so a snapshot that outlives the handle still deregisters itself
//! on drop. It is a leaf lock: nothing is taken while it is held.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

#[derive(Default)]
struct Live {
    next_id: u64,
    /// `(sequence, id)` to the moment the snapshot was taken. Sequences never
    /// decrease over time and ids increase, so the first entry is also the
    /// snapshot that has been alive the longest.
    taken: BTreeMap<(u64, u64), Instant>,
}

#[derive(Default)]
pub(crate) struct SnapshotRegistry {
    live: Mutex<Live>,
}

/// The oldest live snapshot, and how many there are.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Summary {
    pub live: u64,
    /// Sequence of the oldest snapshot (0 when there is none).
    pub oldest_seq: u64,
    /// How long it has been alive (zero when there is none).
    pub oldest_age: Duration,
}

/// Deregisters its snapshot when dropped.
pub(crate) struct SnapshotGuard {
    registry: Arc<SnapshotRegistry>,
    key: (u64, u64),
}

impl SnapshotRegistry {
    /// Registers a snapshot that observes `seq`.
    pub fn register(self: &Arc<Self>, seq: u64) -> SnapshotGuard {
        let mut live = self.live.lock();
        let key = (seq, live.next_id);
        live.next_id += 1;
        live.taken.insert(key, Instant::now());
        SnapshotGuard { registry: self.clone(), key }
    }

    pub fn summary(&self) -> Summary {
        let live = self.live.lock();
        match live.taken.iter().next() {
            Some((&(oldest_seq, _), taken)) => {
                Summary { live: live.taken.len() as u64, oldest_seq, oldest_age: taken.elapsed() }
            }
            None => Summary::default(),
        }
    }
}

impl Drop for SnapshotGuard {
    fn drop(&mut self) {
        self.registry.live.lock().taken.remove(&self.key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracks_live_count_and_the_oldest_by_sequence() {
        let registry = Arc::new(SnapshotRegistry::default());
        assert_eq!(registry.summary(), Summary::default());
        let a = registry.register(5);
        let b = registry.register(5);
        let c = registry.register(9);
        let s = registry.summary();
        assert_eq!((s.live, s.oldest_seq), (3, 5));
        drop(a);
        // The second snapshot at the same sequence takes over.
        assert_eq!((registry.summary().live, registry.summary().oldest_seq), (2, 5));
        drop(b);
        assert_eq!((registry.summary().live, registry.summary().oldest_seq), (1, 9));
        drop(c);
        assert_eq!(registry.summary(), Summary::default());
    }

    #[test]
    fn a_guard_outlives_the_registry_owner() {
        let registry = Arc::new(SnapshotRegistry::default());
        let guard = registry.register(1);
        let weak = Arc::downgrade(&registry);
        drop(registry);
        assert!(weak.upgrade().is_some(), "the guard keeps the registry alive");
        drop(guard);
        assert!(weak.upgrade().is_none(), "and releases it when it goes");
    }

    #[test]
    fn age_grows_while_the_snapshot_lives() {
        let registry = Arc::new(SnapshotRegistry::default());
        let _g = registry.register(1);
        std::thread::sleep(Duration::from_millis(5));
        assert!(registry.summary().oldest_age >= Duration::from_millis(5));
    }
}
