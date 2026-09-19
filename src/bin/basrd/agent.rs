//! Agent backends and the abstraction that hides their differences.
//!
//! Two kinds of backend are supported:
//!
//! * [`Kind::Embedded`] runs an agent implemented in this crate, in the daemon's
//!   own process (see [`embedded`]).
//! * [`Kind::Acp`] drives an external agent over the Agent Client Protocol,
//!   spawning it as a child process (see [`acp`]).
//!
//! Both produce [`Update`] values through the same channel, so the session layer
//! and every frontend stay backend-agnostic.

mod catalog;
mod driver;
mod spec;

pub mod acp;
pub mod embedded;

pub use catalog::*;
pub use driver::*;
pub use spec::*;

#[cfg(test)]
mod tests;
