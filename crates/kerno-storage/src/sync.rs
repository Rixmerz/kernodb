//! Synchronization primitives of the commit queue.
//!
//! The commit queue ([`crate::commit_queue`]) is written against this module
//! so that its protocol can be explored exhaustively: normally these are the
//! `parking_lot` types the rest of the engine uses, and with
//! `RUSTFLAGS="--cfg kerno_loom"` they are `loom`'s, wrapped to the same
//! interface (`lock()` returns the guard, `wait` takes the guard by
//! reference, no poisoning).

#[cfg(not(kerno_loom))]
pub(crate) use parking_lot::{Condvar, Mutex};

#[cfg(not(kerno_loom))]
pub(crate) use std::sync::Arc;

#[cfg(kerno_loom)]
pub(crate) use loom::sync::Arc;

#[cfg(kerno_loom)]
pub(crate) use loom_adapter::{Condvar, Mutex};

#[cfg(kerno_loom)]
mod loom_adapter {
    use std::ops::{Deref, DerefMut};

    pub(crate) struct Mutex<T>(loom::sync::Mutex<T>);

    /// `None` only while [`Condvar::wait`] holds the inner guard.
    pub(crate) struct MutexGuard<'a, T>(Option<loom::sync::MutexGuard<'a, T>>);

    impl<T> Mutex<T> {
        pub(crate) fn new(value: T) -> Self {
            Mutex(loom::sync::Mutex::new(value))
        }

        pub(crate) fn lock(&self) -> MutexGuard<'_, T> {
            // loom's mutex only fails if a model thread panicked, which
            // fails the model anyway.
            MutexGuard(Some(self.0.lock().expect("loom mutex poisoned")))
        }
    }

    impl<T> Deref for MutexGuard<'_, T> {
        type Target = T;

        fn deref(&self) -> &T {
            self.0.as_ref().expect("guard held")
        }
    }

    impl<T> DerefMut for MutexGuard<'_, T> {
        fn deref_mut(&mut self) -> &mut T {
            self.0.as_mut().expect("guard held")
        }
    }

    pub(crate) struct Condvar(loom::sync::Condvar);

    impl Condvar {
        pub(crate) fn new() -> Self {
            Condvar(loom::sync::Condvar::new())
        }

        pub(crate) fn wait<T>(&self, guard: &mut MutexGuard<'_, T>) {
            let inner = guard.0.take().expect("guard held");
            guard.0 = Some(self.0.wait(inner).expect("loom mutex poisoned"));
        }

        pub(crate) fn notify_one(&self) {
            self.0.notify_one();
        }

        pub(crate) fn notify_all(&self) {
            self.0.notify_all();
        }
    }
}
