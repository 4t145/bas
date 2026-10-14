//! Daemon implementation of the shared `bas.v1.Bas` protocol.

mod authentication_error;
mod pagination;
mod server;
mod status;

pub use server::*;

#[cfg(test)]
mod tests;
