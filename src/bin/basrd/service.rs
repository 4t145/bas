//! Daemon implementation of the shared `basr.v1.Basr` protocol.

mod pagination;
mod server;

pub use server::*;

#[cfg(test)]
mod tests;
