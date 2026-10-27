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

pub mod config;
pub use bas_client::{acp, acp_v2, authentication, management, model, service};
