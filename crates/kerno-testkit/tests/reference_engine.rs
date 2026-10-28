//! Validates the shared harness against the trivially correct `LogEngine`.

kerno_testkit::engine_test_suite!(
    kerno_testkit::reference::LogEngine,
    buffered_power_loss_prefix = true,
    detects_truncation = false
);
