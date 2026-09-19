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
    let mut client = Client::connect_at(args.server)
        .await
        .with_context(|| format!("cannot connect to basrd at {}; is it running?", args.server))?;

    args.command.run(&mut client).await
}
