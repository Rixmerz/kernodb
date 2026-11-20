#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Serves Kerno databases over an HTTP JSON API and gRPC. See the crate README for the API and
//! `docs/server.md` for running it.
//!
//! One [`Service`] holds one open database; the [`app::App`] holds them all, one per tenant, with
//! the tokens that pick one and the metrics. [`http`] and [`grpc`] are thin transports over it:
//! each turns a request into a call of the service and the answer (or the [`error::ApiError`])
//! back into its own wire format.

pub mod api;
pub mod app;
pub mod args;
pub mod auth;
pub mod error;
pub mod grpc;
pub mod http;
pub mod metrics;
pub mod service;
pub mod tls;

pub use service::Service;

/// The most bytes of one request body, on either transport.
pub const MAX_BODY: usize = 16 << 20;
