//! A scripted ACP test agent for exercising client implementations.
//!
//! The agent has no model behind it. Prompt text selects a deterministic demo,
//! which lets tests and manual client sessions drive individual protocol
//! features. One process serves one client, using ACP v1 or v2 depending on the
//! `initialize` request.

pub mod cli;
pub mod demo;
pub mod extensions;
pub mod v1;
pub mod v2;

use agent_client_protocol::{Agent, ConnectTo, Stdio};

use crate::demo::Options;

/// Serve one ACP connection over stdio, routing v1 and v2 clients to their
/// respective implementations.
///
/// # Errors
///
/// Returns an error when the transport fails or when a protocol implementation
/// stops with an error.
pub async fn run(options: Options) -> agent_client_protocol::Result<()> {
    let mut router = Agent.protocol_router();
    if options.v1 {
        router = router.with_v1(v1::component(options.clone()));
    }
    if options.v2 {
        router = router.with_v2(v2::component(options.clone()));
    }
    router.connect_to(Stdio::new()).await
}
