//! The gRPC surface exposed to clients.
//!
//! `basrd` serves [`pb::basr_server`]; `basrctl`, the TUI, and the GUI consume
//! [`pb::basr_client`]. Everything crossing the wire is a [`crate::model`]
//! value, converted by [`convert`].

mod client;
mod convert;
mod server;

pub mod pb {
    //! Generated protobuf messages and service stubs.

    tonic::include_proto!("basr.v1");

    /// Encoded `FileDescriptorSet`, for gRPC reflection servers.
    pub const FILE_DESCRIPTOR_SET: &[u8] = tonic::include_file_descriptor_set!("basr_descriptor");
}

pub use client::*;
pub use convert::*;
pub use server::*;

#[cfg(test)]
mod tests;
