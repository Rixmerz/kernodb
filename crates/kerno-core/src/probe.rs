//! Per-thread phase label for test traces.
//!
//! Engine code marks what it is doing (`create`, `session`, `commit`,
//! `rotate`, `flush`, `compact`, `compact_range`, `close`, `checkpoint`,
//! `delete`, `sync`) with [`phase`]; a simulated file system reads the label
//! with [`current_phase`] and records it with every operation it traces, so
//! that crash points can be mapped to the step of the protocol they hit.
//!
//! The cost is one write to a thread-local per guard; nothing is recorded
//! unless a test asks for it.
//!
//! ```
//! use kerno_core::probe;
//!
//! assert_eq!(probe::current_phase(), "");
//! {
//!     let _flush = probe::phase("flush");
//!     assert_eq!(probe::current_phase(), "flush");
//! }
//! assert_eq!(probe::current_phase(), "");
//! ```

use std::cell::Cell;
use std::marker::PhantomData;

thread_local! {
    static PHASE: Cell<&'static str> = const { Cell::new("") };
}

/// Sets the calling thread's phase label to `name` until the returned guard
/// is dropped, which restores the previous label (so phases nest).
#[must_use = "the phase label is reset when the guard is dropped"]
pub fn phase(name: &'static str) -> PhaseGuard {
    let prev = PHASE.with(|p| p.replace(name));
    PhaseGuard { prev, _not_send: PhantomData }
}

/// The calling thread's current phase label (`""` outside any phase).
pub fn current_phase() -> &'static str {
    PHASE.with(Cell::get)
}

/// Restores the previous phase label of its thread when dropped.
///
/// Not `Send`: it must be dropped on the thread that created it.
#[derive(Debug)]
pub struct PhaseGuard {
    prev: &'static str,
    _not_send: PhantomData<*const ()>,
}

impl Drop for PhaseGuard {
    fn drop(&mut self) {
        PHASE.with(|p| p.set(self.prev));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_phase_is_empty() {
        assert_eq!(current_phase(), "");
    }

    #[test]
    fn nested_phases_restore_the_previous_label() {
        let outer = phase("close");
        assert_eq!(current_phase(), "close");
        {
            let _inner = phase("flush");
            assert_eq!(current_phase(), "flush");
        }
        assert_eq!(current_phase(), "close");
        drop(outer);
        assert_eq!(current_phase(), "");
    }

    #[test]
    fn phases_are_per_thread() {
        let _g = phase("commit");
        let other = std::thread::spawn(|| {
            let before = current_phase();
            let _h = phase("compact");
            (before, current_phase())
        })
        .join()
        .unwrap();
        assert_eq!(other, ("", "compact"));
        assert_eq!(current_phase(), "commit");
    }
}
