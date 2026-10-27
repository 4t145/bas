//! BAS protocol models and authenticated RPC clients.
//!
//! Frontend preferences and daemon settings storage belong to their applications.
//! The BAS daemon enables `server` for its transport endpoints; frontends use the
//! default client-only API. `test-support` enables isolated RPC server fixtures.

pub mod authentication;
pub mod config;
pub mod management;
pub mod model;
pub mod service;

pub use agent_client_protocol::schema::v1 as acp;
pub use agent_client_protocol::schema::v2 as acp_v2;
