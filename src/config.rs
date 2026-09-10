//! Daemon configuration.
//!
//! Everything is read from the environment so that a systemd unit can configure
//! `basrd` without a writable config directory.

mod agents;
mod daemon;

pub use agents::*;
pub use daemon::*;

#[cfg(test)]
mod tests;
