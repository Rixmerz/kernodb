//! Validates the shared harness against the trivially correct `LogEngine`.

kerno_testkit::engine_test_suite!(
    kerno_testkit::reference::LogEngine,
    buffered_power_loss_prefix = true,
    detects_truncation = false
);

kerno_testkit::extended_failure_suite!(kerno_testkit::reference::LogEngine, buffered_power_loss_prefix = true);

kerno_testkit::sync_test_suite!(kerno_testkit::reference::LogEngine);

kerno_testkit::group_commit_test_suite!(kerno_testkit::reference::LogEngine);
