//! Runs the extended failure suite (process kill that keeps unsynced writes
//! pending, Linux `fsync` failures, I/O errors in `Buffered` mode,
//! per-directory durability and a full disk) against `KernoDb`, with
//! background threads and with the inline profile.

/// The default configuration: background flush and compaction threads.
mod threads {
    kerno_testkit::extended_failure_suite!(kerno_storage::KernoDb, buffered_power_loss_prefix = true);
}

/// Flushes and compactions inline (profile `kerno-inline`).
mod inline {
    kerno_testkit::extended_failure_suite!(
        kerno_storage::testing::Profiled<kerno_storage::testing::Inline>,
        buffered_power_loss_prefix = true
    );
}
