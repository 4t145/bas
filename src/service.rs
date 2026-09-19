//! The gRPC surface exposed to clients.
//!
//! `basrd` serves [`pb::basr_server`]; `basrctl`, the TUI, and the GUI consume
//! [`pb::basr_client`]. ACP requests and updates use SDK JSON inside
//! [`pb::AcpPayload`]; protobuf defines daemon management and replay envelopes.

mod client;
mod convert;
mod defaults;

pub mod pb {
    //! Generated protobuf messages and service stubs.

    tonic::include_proto!("basr.v1");

    /// Encoded `FileDescriptorSet`, for gRPC reflection servers.
    pub const FILE_DESCRIPTOR_SET: &[u8] = tonic::include_file_descriptor_set!("basr_descriptor");
}

pub use client::*;
pub use convert::*;
pub use defaults::*;

#[cfg(test)]
mod tests;
