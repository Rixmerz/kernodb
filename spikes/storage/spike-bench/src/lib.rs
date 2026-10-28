//! Benchmark workloads, metrics and reference-engine adapters for the Kerno
//! Phase 0 storage spike. The `spike-bench` binary drives them; the
//! differential test reuses the adapters.

pub mod adapters;
pub mod metrics;
pub mod workloads;
