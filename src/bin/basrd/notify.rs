//! Desktop notifications and the daemon-side watchers that trigger them.
//!
//! The design document calls for telling the user when a task is finished. The
//! transport is D-Bus (`org.freedesktop.Notifications`), reached through the
//! session bus. The daemon can disable this worker in headless environments.

mod dbus;
mod notification;
mod watcher;

pub use dbus::*;
pub use notification::*;
pub use watcher::*;

#[cfg(test)]
mod tests;
