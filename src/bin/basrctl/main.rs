//! `basrctl` — the BASR management tool.
//!
//! Every command is a thin wrapper around the daemon's gRPC service, so the CLI
//! never needs its own view of sessions.

mod clap;
mod command;
mod render;

use ::clap::Parser;
use anyhow::{Context, Result};
use basr::service::Client;

#[tokio::main]
async fn main() -> Result<()> {
    let args = clap::Args::parse();
    let mut client = connect(args.server).await?;

    command::run(args.command, &mut client).await
}

/// Connects to the daemon, turning a transport failure into a usable hint.
///
/// # Errors
///
/// Returns an error when the daemon cannot be reached.
async fn connect(server: std::net::SocketAddr) -> Result<Client> {
    basr::service::connect(server)
        .await
        .with_context(|| format!("cannot connect to basrd at {server}; is it running?"))
}
