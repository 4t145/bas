//! Domain types shared by the daemon, the management CLI, and every client.
//!
//! Daemon lifecycle and replay envelopes wrap the shared ACP schema.
//! Content, tools and stop reasons are re-exported from [`crate::acp`].

mod approval;
mod content;
mod event;
mod ids;
mod message;
pub mod presentation;
mod session;
mod submission;
mod time;

pub use approval::*;
pub use content::*;
pub use event::*;
pub use ids::*;
pub use message::*;
pub use session::*;
pub use submission::*;
pub use time::*;

#[cfg(test)]
mod tests;
