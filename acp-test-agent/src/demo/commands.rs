//! Parsing of the scripted prompt commands.
//!
//! A prompt that does not start with `/` is treated as `/echo <text>`, which
//! keeps the agent usable as a plain echo backend.

/// Description of one advertised command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Descriptor {
    /// Command name without a leading slash.
    pub name: &'static str,
    /// Human-readable description for clients.
    pub description: &'static str,
    /// Optional hint shown while the client composes command input.
    pub hint: Option<&'static str>,
}

/// A parsed prompt command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Show the command catalog.
    Help,
    /// Stream text back as message chunks.
    Echo { text: String },
    /// Stream one block of every supported content type.
    Rich,
    /// Stream a thought chunk before the answer.
    Thought,
    /// Publish and finish a tool call.
    Tool,
    /// Run an execution plan, waiting between steps.
    Plan { millis: u64 },
    /// Remove the demo plan.
    ClearPlan,
    /// Ask the client for permission.
    Permission,
    /// Send a form elicitation.
    Elicitation,
    /// Send a URL elicitation and complete it once the client responds.
    ElicitationUrl,
    /// Exercise client file-system callbacks (v1 only).
    FileSystem,
    /// Exercise client terminals (v1) or agent-owned terminals (v2).
    Terminal,
    /// Wait before answering.
    Sleep { millis: u64 },
    /// Change a session config option and publish an update.
    Config { setting: Option<String> },
    /// Re-send available commands.
    Commands,
    /// Send usage and cost updates.
    Usage,
    /// Send a user-facing notice.
    Notice,
    /// Send a context compaction update.
    Compaction,
    /// Update session metadata.
    Info,
    /// Send custom extension traffic.
    Extension,
    /// Fail the turn.
    Fail,
    /// End the turn with a specific stop reason.
    Stop { reason: StopReason },
    /// Unknown slash command; the string is the command name as typed.
    Unknown(String),
    /// Known command with invalid arguments; the string is a human-readable reason.
    Invalid { reason: String },
}

/// Stop reasons the scripted agent can produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// Successful completion.
    EndTurn,
    /// Token budget exhausted.
    MaxTokens,
    /// Turn request budget exhausted.
    MaxTurnRequests,
    /// The agent refused to continue.
    Refusal,
    /// Work was cancelled.
    Cancelled,
    /// v2 failed stop reason; v1 maps this to an RPC error.
    Error,
}

/// Default delay used by `/sleep` when no argument is given.
pub const DEFAULT_SLEEP_MILLIS: u64 = 3_000;

/// Default interval between demo plan steps, leaving time to test cancellation.
pub const DEFAULT_PLAN_MILLIS: u64 = 5_000;

impl StopReason {
    /// Parse the value accepted by `/stop`.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "end_turn" | "endTurn" => Some(Self::EndTurn),
            "max_tokens" | "maxTokens" => Some(Self::MaxTokens),
            "max_turn_requests" | "maxTurnRequests" => Some(Self::MaxTurnRequests),
            "refusal" => Some(Self::Refusal),
            "cancelled" | "canceled" => Some(Self::Cancelled),
            "error" => Some(Self::Error),
            _ => None,
        }
    }

    /// Wire-compatible label used in user-facing text.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::EndTurn => "end_turn",
            Self::MaxTokens => "max_tokens",
            Self::MaxTurnRequests => "max_turn_requests",
            Self::Refusal => "refusal",
            Self::Cancelled => "cancelled",
            Self::Error => "error",
        }
    }
}

#[cfg(test)]
mod tests;

/// Parse one prompt as a command.
#[must_use]
pub fn parse(text: &str) -> Command {
    let text = text.trim();
    let Some(rest) = text.strip_prefix('/') else {
        return Command::Echo {
            text: text.to_string(),
        };
    };
    let (name, argument) = match rest.split_once(char::is_whitespace) {
        Some((name, argument)) => (name, argument.trim()),
        None => (rest, ""),
    };
    match name.to_ascii_lowercase().as_str() {
        "help" => Command::Help,
        "echo" => Command::Echo {
            text: argument.to_string(),
        },
        "rich" => Command::Rich,
        "thought" => Command::Thought,
        "tool" => Command::Tool,
        "plan" if argument == "clear" => Command::ClearPlan,
        "plan" => match parse_millis(argument, DEFAULT_PLAN_MILLIS) {
            Ok(millis) => Command::Plan { millis },
            Err(reason) => Command::Invalid { reason },
        },
        "permission" => Command::Permission,
        "elicit" => Command::Elicitation,
        "elicit-url" | "elicit_url" => Command::ElicitationUrl,
        "fs" => Command::FileSystem,
        "terminal" => Command::Terminal,
        "sleep" => parse_sleep(argument),
        "config" => Command::Config {
            setting: (!argument.is_empty()).then(|| argument.to_string()),
        },
        "commands" => Command::Commands,
        "usage" => Command::Usage,
        "notice" => Command::Notice,
        "compact" => Command::Compaction,
        "info" => Command::Info,
        "extension" => Command::Extension,
        "fail" => Command::Fail,
        "stop" => parse_stop(argument),
        other => Command::Unknown(other.to_string()),
    }
}

fn parse_sleep(argument: &str) -> Command {
    match parse_millis(argument, DEFAULT_SLEEP_MILLIS) {
        Ok(millis) => Command::Sleep { millis },
        Err(reason) => Command::Invalid { reason },
    }
}

/// # Errors
/// Rejects arguments that are not an unsigned millisecond count.
fn parse_millis(argument: &str, default: u64) -> Result<u64, String> {
    if argument.is_empty() {
        Ok(default)
    } else {
        argument
            .parse()
            .map_err(|_| format!("`{argument}` is not a millisecond count"))
    }
}

fn parse_stop(argument: &str) -> Command {
    if argument.is_empty() {
        return Command::Stop {
            reason: StopReason::EndTurn,
        };
    }
    match StopReason::parse(argument) {
        Some(reason) => Command::Stop { reason },
        None => Command::Invalid {
            reason: format!(
                "`{argument}` is not a stop reason; use end_turn, max_tokens, \
                 max_turn_requests, refusal, cancelled, or error"
            ),
        },
    }
}
