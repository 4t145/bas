//! Session lifecycle: creation, attach and detach, replay, and history.
//!
//! A session outlives its clients. Detaching only drops an [`Attachment`]; the
//! backend keeps producing [`Event`](crate::model::Event)s into the session's
//! replay window, so a later attach can be told the newest N messages and can
//! page backwards with a cursor.

mod attachment;
mod conversation;
mod history;
mod registry;

pub use attachment::*;
pub use conversation::*;
pub use history::*;
pub use registry::*;

#[cfg(test)]
mod tests;
