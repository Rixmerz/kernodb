//! The shared suites with range deletions in their random workload, and the
//! range deletion suite, against `KernoDb` and its profiles. A separate test
//! target from `shared.rs`: the generated modules have fixed names.

use kerno_testkit::harness::SuiteConfig;

/// Thousandths of the random operations that are range deletions.
const RANGES: u32 = 20;

mod threads {
    use super::*;

    kerno_testkit::engine_test_suite!(
        kerno_storage::KernoDb,
        buffered_power_loss_prefix = true,
        config = SuiteConfig::with_range_deletes(RANGES)
    );
    kerno_testkit::range_delete_test_suite!(kerno_storage::KernoDb);
}

/// Flushes and compactions inline (profile `kerno-inline`).
mod inline {
    use super::*;

    kerno_testkit::engine_test_suite!(
        kerno_storage::testing::Profiled<kerno_storage::testing::Inline>,
        buffered_power_loss_prefix = true,
        config = SuiteConfig::with_range_deletes(RANGES)
    );
    kerno_testkit::range_delete_test_suite!(kerno_storage::testing::Profiled<kerno_storage::testing::Inline>);
}

/// The M0 compaction configuration (profile `kerno-m0`): no flush split, no
/// partial L0 compactions, no trivial moves.
mod m0_profile {
    kerno_testkit::range_delete_test_suite!(kerno_storage::testing::Profiled<kerno_storage::testing::M0>);
}
