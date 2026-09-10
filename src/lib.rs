//! BASR — a Basic Agent Service in Rust.
//!
//! This crate is the shared implementation behind the `basrd` daemon and the
//! `basrctl` management tool:
//!
//! * [`model`] — backend-neutral domain types (the `d` layer).
//! * [`agent`] — agent backends and the drivers that start them.
//! * [`session`] — session lifecycle, attach/detach, replay, and history.
//! * [`service`] — the gRPC surface exposed to clients.
//! * [`notify`] — desktop notifications and completion watchers.
//! * [`config`] — daemon configuration read from the environment.

pub mod agent;
pub mod config;
pub mod model;
pub mod notify;
pub mod service;
pub mod session;
