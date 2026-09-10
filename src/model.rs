//! Domain types shared by the daemon, the management CLI, and every client.
//!
//! This module is the daemon's `d` layer: it is deliberately independent of the
//! agent backend that produced a value, so an embedded agent and an ACP agent
//! look identical to attached frontends.

mod content;
mod event;
mod ids;
mod message;
mod session;
mod time;

pub use content::*;
pub use event::*;
pub use ids::*;
pub use message::*;
pub use session::*;
pub use time::*;

#[cfg(test)]
mod tests;
