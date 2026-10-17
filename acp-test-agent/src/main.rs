use std::process::ExitCode;

use acp_test_agent::cli::Cli;
use clap::Parser;

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match acp_test_agent::run(cli.options()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // stdout belongs to the JSON-RPC transport; diagnostics go to stderr.
            eprintln!("acp-test-agent: {error}");
            ExitCode::FAILURE
        }
    }
}
