//! The shared functional, crash and corruption suites from `spike-core`.
//!
//! Buffered commits are not fsynced and a copy-on-write tree has no ordering
//! barrier between its pages and its meta page in that mode, so after a power
//! loss a buffered database may report corruption instead of a consistent
//! prefix (see DESIGN.md). Buffered mode is therefore only crash-tested
//! against process kills.

use spike_btree::BTreeEngine;

spike_core::engine_test_suite!(BTreeEngine, buffered_power_loss_prefix = false);
