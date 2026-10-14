//! Daemon configuration.
//!
//! Flags configure the service; unified settings configure reloadable backends.

mod agents;
mod daemon;
mod reload;

pub use agents::*;
pub use daemon::*;
pub use reload::*;

#[cfg(test)]
mod tests;
