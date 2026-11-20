//! Runs the shared functional, crash and corruption suites against the LSM engine.

spike_core::engine_test_suite!(spike_lsm::LsmEngine, buffered_power_loss_prefix = true);
