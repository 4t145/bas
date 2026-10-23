//! Subcommand dispatch.

mod agents;
mod attach;
mod history;
mod prompt;
mod session;

use anyhow::Result;
use bas::service::Client;

use crate::clap::Command;

impl Command {
    /// Runs one `bast` invocation against a connected daemon.
    ///
    /// # Errors
    ///
    /// Returns an error when the daemon rejects the request or the connection
    /// fails mid-call.
    pub async fn run(self, client: &mut Client, server: &bas::service::Target) -> Result<()> {
        match self {
            Command::Agents => agents::list(client).await,
            Command::Agent(command) => command.run(client, server).await,
            Command::Session(command) => command.run(client).await,
            Command::Attach {
                session,
                cursor,
                replay,
            } => attach::run(client, &session, cursor, replay).await,
            Command::Prompt {
                session,
                submission,
                text,
            } => prompt::send(client, server, &session, submission, text).await,
            Command::PromptStatus { session, turn } => prompt::status(client, session, turn).await,
            Command::Cancel { session } => prompt::cancel(client, &session).await,
            Command::History {
                session,
                after,
                limit,
            } => history::page(client, &session, after, limit).await,
        }
    }
}
