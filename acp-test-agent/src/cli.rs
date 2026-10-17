//! Command-line interface for the stdio agent.

use clap::{Parser, ValueEnum};

use crate::demo::Options;

/// A scripted ACP agent used to exercise clients.
#[derive(Debug, Clone, Parser)]
#[command(
    name = "acp-test-agent",
    version,
    about = "Scripted ACP v1/v2 agent for exercising clients",
    long_about = "Runs an Agent Client Protocol agent over stdio. It negotiates v1 or v2 \
                  from the initialize request and answers prompts with scripted demos: run \
                  /help in a session to list them."
)]
pub struct Cli {
    /// Number of sessions returned on each session/list page.
    #[arg(long, default_value_t = 2)]
    pub page_size: usize,

    /// Reject session creation until an authenticate/auth-login call succeeds.
    #[arg(long)]
    pub require_auth: bool,

    /// Protocol versions offered during initialize negotiation.
    #[arg(long, value_enum, default_value_t = Protocol::Both)]
    pub protocol: Protocol,
}

/// Protocol versions the agent offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Protocol {
    /// Negotiate v2 when the client asks for it, otherwise v1.
    #[value(name = "both")]
    Both,
    /// Only serve clients that can use v1.
    #[value(name = "v1")]
    V1,
    /// Only serve clients that can use v2.
    #[value(name = "v2")]
    V2,
}

impl Cli {
    /// Convert parsed arguments into agent options.
    #[must_use]
    pub fn options(&self) -> Options {
        let (v1, v2) = match self.protocol {
            Protocol::Both => (true, true),
            Protocol::V1 => (true, false),
            Protocol::V2 => (false, true),
        };
        Options {
            page_size: self.page_size.max(1),
            require_auth: self.require_auth,
            v1,
            v2,
        }
    }
}
