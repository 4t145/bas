//! Subcommand dispatch.

mod agents;
mod attach;
mod history;
mod prompt;
mod session;

use anyhow::Result;
use basr::service::Client;

use crate::clap::Command;

impl Command {
    /// Runs one `basrctl` invocation against a connected daemon.
    ///
    /// # Errors
    ///
    /// Returns an error when the daemon rejects the request or the connection
    /// fails mid-call.
    pub async fn run(self, client: &mut Client) -> Result<()> {
        match self {
            Command::Agents => agents::list(client).await,
            Command::Session(command) => command.run(client).await,
            Command::Attach {
                session,
                cursor,
                replay,
            } => attach::run(client, &session, cursor, replay).await,
            Command::Prompt { session, text } => prompt::send(client, &session, text).await,
            Command::Cancel { session } => prompt::cancel(client, &session).await,
            Command::History {
                session,
                after,
                limit,
            } => history::page(client, &session, after, limit).await,
        }
    }
}
