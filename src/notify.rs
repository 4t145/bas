//! Desktop notifications and the daemon-side watchers that trigger them.
//!
//! The design document calls for telling the user when a task is finished. The
//! transport is D-Bus (`org.freedesktop.Notifications`), reached through the
//! session bus; [`Null`] keeps the daemon working in environments without one,
//! such as an installer or a headless systemd unit.

mod dbus;
mod notification;
mod watcher;

pub use dbus::*;
pub use notification::*;
pub use watcher::*;

#[cfg(test)]
mod tests;
