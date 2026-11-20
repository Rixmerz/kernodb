//! `engine_test_suite!` with range deletions in its random workload (the
//! `config` arm), validated on the reference before any engine runs it:
//! `LogEngine` implements them natively, `Emulated<LogEngine>` through point
//! deletions. A separate test target, so that the corruption fingerprint of
//! `reference_engine` (the FP gate) only ever prints the default suite.

use kerno_testkit::harness::SuiteConfig;

/// Thousandths of the random operations that are range deletions.
const RANGES: u32 = 20;

mod native {
    use super::*;

    kerno_testkit::engine_test_suite!(
        kerno_testkit::reference::LogEngine,
        buffered_power_loss_prefix = true,
        detects_truncation = false,
        config = SuiteConfig::with_range_deletes(RANGES)
    );
}

mod emulated {
    use super::*;

    kerno_testkit::engine_test_suite!(
        kerno_testkit::emulated::Emulated<kerno_testkit::reference::LogEngine>,
        buffered_power_loss_prefix = true,
        detects_truncation = false,
        config = SuiteConfig::with_range_deletes(RANGES)
    );
}
