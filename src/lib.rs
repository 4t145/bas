//! BASR — a Basic Agent Service in Rust.
//!
//! Shared types and client support for `basrd`, `basrctl`, and future frontends:
//!
//! * [`model`] — daemon lifecycle, replay envelopes and client presentation types.
//! * [`service`] — gRPC envelopes, ACP payloads, and client connections.
//! * [`config`] — shared settings, validation, and section-scoped persistence.
//!
//! Backend drivers, session management, runtime configuration, notifications, and the
//! gRPC service implementation belong to the `basrd` binary.

pub mod config;
pub mod model;
pub mod service;

/// ACP schema supplied by agent-client-protocol 2.x (the negotiated v1 protocol).
pub use agent_client_protocol::schema::v1 as acp;
