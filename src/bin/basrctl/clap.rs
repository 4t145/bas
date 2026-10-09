use std::{net::SocketAddr, path::PathBuf};

use clap::{Parser, Subcommand};

/// Control the BASR agent service.
#[derive(Clone, Debug, Parser)]
#[command(name = "basrctl", version, about, long_about = None)]
pub struct Args {
    /// Address of the daemon's gRPC service.
    #[arg(long, global = true, default_value = basr::service::DEFAULT_ADDRESS)]
    pub server: SocketAddr,

    #[command(subcommand)]
    pub command: Command,
}

/// What to do with the daemon.
#[derive(Clone, Debug, Subcommand)]
pub enum Command {
    /// List the agent backends the daemon can launch.
    Agents,

    /// Discover, install and authenticate agent backends.
    #[command(subcommand)]
    Agent(Agent),

    /// Manage sessions.
    #[command(subcommand)]
    Session(Session),

    /// Attach to a session, print recent events, and follow it live.
    ///
    /// Detaching with `Ctrl-C` leaves the agent running.
    Attach {
        /// Session id.
        session: String,
        /// Resume strictly after this cursor.
        #[arg(long)]
        cursor: Option<u64>,
        /// Replayable events to print before live ones.
        #[arg(long, default_value_t = basr::service::DEFAULT_REPLAY)]
        replay: usize,
    },

    /// Send a user turn to a session.
    Prompt {
        /// Session id.
        session: String,
        /// Prompt text; several words are joined with spaces.
        #[arg(required = true, num_args = 1..)]
        text: Vec<String>,
    },

    /// Ask a session's backend to abandon the running turn.
    Cancel {
        /// Session id.
        session: String,
    },

    /// Print a page of a session's history.
    History {
        /// Session id.
        session: String,
        /// Start strictly after this cursor.
        #[arg(long)]
        after: Option<u64>,
        /// Maximum events to print.
        #[arg(long, default_value_t = 50)]
        limit: u32,
    },
}

#[derive(Clone, Debug, Subcommand)]
pub enum Agent {
    /// Browse the ACP registry and configured agents, including missing programs.
    Catalog {
        #[arg(long)]
        refresh: bool,
        #[arg(long)]
        json: bool,
    },
    /// Install or update a registry agent on the local daemon host.
    Install { id: String },
    /// Check ACP session readiness without sending a model request.
    Check { id: String },
    /// Authenticate using an agent-advertised method in this terminal.
    Login {
        id: String,
        /// Authentication method id; required when several are advertised.
        #[arg(long)]
        method: Option<String>,
    },
    /// Remove a managed registration; retain credentials and version directories.
    Remove { id: String },
}

/// Session administration.
#[derive(Clone, Debug, Subcommand)]
pub enum Session {
    /// Start a session on an agent backend.
    Create {
        /// Backend id; the daemon default is used when omitted.
        #[arg(long, default_value = "")]
        agent: String,
        /// Initial title.
        #[arg(long)]
        title: Option<String>,
        /// Working directory for the agent.
        #[arg(long)]
        cwd: Option<PathBuf>,
    },

    /// List non-archived sessions.
    List {
        /// Include archived sessions.
        #[arg(long)]
        all: bool,
    },

    /// Show one session's metadata.
    Show {
        /// Session id.
        session: String,
    },

    /// Reversibly archive a session without stopping its backend.
    Archive {
        /// Session id.
        session: String,
    },

    /// Restore an archived session to the default list.
    Restore {
        /// Session id.
        session: String,
    },

    /// Permanently stop a session's backend and forget it (not recoverable archive).
    Remove {
        /// Session id.
        session: String,
    },

    /// Rename a session.
    Rename {
        /// Session id.
        session: String,
        /// New title.
        title: String,
    },
}

#[cfg(test)]
mod tests;
