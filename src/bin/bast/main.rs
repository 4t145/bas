//! BasT (`bast`) — the Bas management tool.
//!
//! Every command is a thin wrapper around the daemon's gRPC service, so the CLI
//! never needs its own view of sessions.

mod clap;
mod command;
mod render;
mod safe;

use ::clap::Parser;
use anyhow::{Context, Result};
use bas::service::Client;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    match run().await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{}", safe::Safe(format_args!("error: {error:#}")));
            std::process::ExitCode::FAILURE
        }
    }
}

/// # Errors
/// Returns an error if connecting to the daemon or running the command fails.
async fn run() -> Result<()> {
    let args = match clap::Args::try_parse() {
        Ok(args) => args,
        Err(error) => {
            let code = error.exit_code();
            if error.use_stderr() {
                eprintln!("{}", safe::Safe(&error));
            } else {
                safe::safe_println!("{error}");
            }
            std::process::exit(code);
        }
    };
    let mut client = Client::connect_options(&args.connection_options()?)
        .await
        .with_context(|| format!("cannot connect to bas at {}; is it running?", args.server))?;

    args.command.run(&mut client, &args.server).await
}
