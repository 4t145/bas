//! Daemon configuration.
//!
//! Flags and environment configure the service; a JSON catalog configures backends.

mod agents;
mod daemon;

pub use agents::*;
pub use daemon::*;

#[cfg(test)]
mod tests;
