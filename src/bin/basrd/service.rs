//! Daemon implementation of the shared `basr.v1.Basr` protocol.

mod authentication_error;
mod pagination;
mod server;
mod status;

pub use server::*;

#[cfg(test)]
mod tests;
