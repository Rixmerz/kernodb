//! Runs the shared functional, crash and corruption suites against `KernoDb`.

kerno_testkit::engine_test_suite!(kerno_storage::KernoDb, buffered_power_loss_prefix = true);
