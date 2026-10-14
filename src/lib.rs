//! Bas — a Basic Agent Service.
//!
//! Shared types and client support for `bas`, `bast`, and future frontends:
//!
//! * [`model`] — daemon lifecycle, replay envelopes and client presentation types.
//! * [`service`] — gRPC envelopes, ACP payloads, and client connections.
//! * [`config`] — shared settings, validation, and section-scoped persistence.
//!
//! Backend drivers, session management, runtime configuration, notifications, and the
//! gRPC service implementation belong to the `bas` binary.

pub mod authentication;
pub mod config;
pub mod management;
pub mod model;
pub mod service;

/// ACP wire schema used by external agents and persisted session events.
pub use agent_client_protocol::schema::v1 as acp;

/// ACP v2 presentation types; external-agent connections still negotiate v1.
pub use agent_client_protocol::schema::v2 as acp_v2;
