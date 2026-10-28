//! Runs the shared functional, crash and corruption suites against `KernoDb`.

kerno_testkit::engine_test_suite!(kerno_storage::KernoDb, buffered_power_loss_prefix = true);

/// The same suites with flushes and compactions inline (profile
/// `kerno-inline`, phase1-design.md §5.1).
mod inline {
    kerno_testkit::engine_test_suite!(
        kerno_storage::testing::Profiled<kerno_storage::testing::Inline>,
        buffered_power_loss_prefix = true
    );
}

/// The same suites against the M0 configuration (profile `kerno-m0`): every
/// condition-1 lever off and an unbounded close (phase1-design.md §1 item 4).
mod m0_profile {
    kerno_testkit::engine_test_suite!(
        kerno_storage::testing::Profiled<kerno_storage::testing::M0>,
        buffered_power_loss_prefix = true
    );
}
