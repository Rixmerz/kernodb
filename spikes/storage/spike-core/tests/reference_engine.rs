//! Validates the shared harness against the trivially correct `LogEngine`.

use spike_core::reference::LogEngine;

spike_core::engine_test_suite!(LogEngine, buffered_power_loss_prefix = true);
