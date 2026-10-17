//! Protocol-neutral demo data shared by the v1 and v2 implementations.
//!
//! Everything here is about the test agent's scripted behavior: which commands
//! exist, what session state is kept, and how prompts map to demos. Typed ACP
//! messages stay in [`crate::v1`] and [`crate::v2`].

pub mod commands;
pub mod sessions;

#[cfg(test)]
mod tests;

/// Name advertised in `initialize` and used for `.name(...)` debugging.
pub const AGENT_NAME: &str = "acp-test-agent";

/// Human-readable title advertised in `initialize`.
pub const AGENT_TITLE: &str = "ACP Test Agent";

/// Select option used to demonstrate model switching.
pub const MODEL_CONFIG_ID: &str = "model";
/// Default model value.
pub const MODEL_FAST: &str = "fast";
/// Alternative model value.
pub const MODEL_DEEP: &str = "deep";

/// Boolean option used to demonstrate toggles and config updates.
pub const THINKING_CONFIG_ID: &str = "thinking";

/// Mode used by the v1 session mode demo.
pub const MODE_ASK: &str = "ask";
/// Alternative v1 mode.
pub const MODE_CODE: &str = "code";
/// Alternative v1 mode.
pub const MODE_REVIEW: &str = "review";

/// Prompt commands understood by the agent.
///
/// This catalog is also advertised as ACP available commands, so names must not
/// include a leading slash.
pub const COMMANDS: &[commands::Descriptor] = &[
    commands::Descriptor {
        name: "help",
        description: "List the scripted feature demos",
        hint: None,
    },
    commands::Descriptor {
        name: "echo",
        description: "Echo text back as streamed message chunks",
        hint: Some("text to echo"),
    },
    commands::Descriptor {
        name: "rich",
        description: "Stream text, image, audio, resource, and resource link content",
        hint: None,
    },
    commands::Descriptor {
        name: "thought",
        description: "Stream a reasoning chunk before the answer",
        hint: None,
    },
    commands::Descriptor {
        name: "tool",
        description: "Show a tool call lifecycle with content and a diff",
        hint: None,
    },
    commands::Descriptor {
        name: "plan",
        description: "Publish an execution plan",
        hint: None,
    },
    commands::Descriptor {
        name: "permission",
        description: "Ask the client for tool permission",
        hint: None,
    },
    commands::Descriptor {
        name: "elicit",
        description: "Request structured input with a form elicitation",
        hint: None,
    },
    commands::Descriptor {
        name: "elicit-url",
        description: "Request URL elicitation, then complete it",
        hint: None,
    },
    commands::Descriptor {
        name: "fs",
        description: "Exercise the v1 client file-system callbacks",
        hint: None,
    },
    commands::Descriptor {
        name: "terminal",
        description: "Exercise client terminals (v1) or agent-owned terminals (v2)",
        hint: None,
    },
    commands::Descriptor {
        name: "sleep",
        description: "Wait before answering so cancellation can be tested",
        hint: Some("milliseconds, default 3000"),
    },
    commands::Descriptor {
        name: "config",
        description: "Change a session config option and announce the update",
        hint: Some("model=fast|deep or on|off"),
    },
    commands::Descriptor {
        name: "commands",
        description: "Re-send the available commands update",
        hint: None,
    },
    commands::Descriptor {
        name: "usage",
        description: "Send context usage and cost updates",
        hint: None,
    },
    commands::Descriptor {
        name: "notice",
        description: "Send a user-facing notice when the client supports it",
        hint: None,
    },
    commands::Descriptor {
        name: "compact",
        description: "Send a context compaction summary and completion",
        hint: None,
    },
    commands::Descriptor {
        name: "info",
        description: "Update session title and activity timestamp",
        hint: None,
    },
    commands::Descriptor {
        name: "extension",
        description: "Send custom _demo extension requests and notifications",
        hint: None,
    },
    commands::Descriptor {
        name: "fail",
        description: "End the turn with an error",
        hint: None,
    },
    commands::Descriptor {
        name: "stop",
        description: "End the turn with a chosen stop reason",
        hint: Some("end_turn|max_tokens|max_turn_requests|refusal|cancelled"),
    },
];

/// Runtime options chosen on the command line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    /// Number of sessions returned per `session/list` page.
    pub page_size: usize,
    /// Reject session creation until a client authenticates.
    pub require_auth: bool,
    /// Offer the v1 protocol implementation.
    pub v1: bool,
    /// Offer the v2 protocol implementation.
    pub v2: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            page_size: 2,
            require_auth: false,
            v1: true,
            v2: true,
        }
    }
}

/// Session config values that both protocol versions expose as config options.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionConfig {
    /// Selected model value.
    pub model: String,
    /// Whether the thinking demo is enabled.
    pub thinking: bool,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            model: MODEL_FAST.to_string(),
            thinking: false,
        }
    }
}

/// Human-readable list used by `/help`.
#[must_use]
pub fn help_text() -> String {
    let mut text = String::from("ACP test agent demos:\n\n");
    for command in COMMANDS {
        text.push_str(&format!(
            "- `/{name}` — {description}\n",
            name = command.name,
            description = command.description
        ));
    }
    text
}

/// Apply the `/config` argument to a session config.
///
/// # Errors
/// Returns a human-readable reason when the setting name or value is not
/// recognized.
pub fn apply_config_setting(
    config: &mut SessionConfig,
    setting: Option<&str>,
) -> Result<(), String> {
    match setting {
        None | Some("toggle") => {
            config.thinking = !config.thinking;
            Ok(())
        }
        Some(value @ ("on" | "off")) => {
            config.thinking = value == "on";
            Ok(())
        }
        Some(value) => {
            let value = value.strip_prefix("model=").unwrap_or(value);
            if matches!(value, MODEL_FAST | MODEL_DEEP) {
                config.model = value.to_string();
                Ok(())
            } else {
                Err(format!(
                    "`{value}` is not a config value; use on, off, model=fast, or model=deep"
                ))
            }
        }
    }
}

/// Embedded text used by the resource content demo.
pub const SAMPLE_RESOURCE: &str = include_str!("demo/assets/sample.md");

/// A 1x1 transparent PNG used by the image content demo.
pub const SAMPLE_IMAGE_BASE64: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";

/// A short silent WAV payload used by the audio content demo.
pub const SAMPLE_AUDIO_BASE64: &str =
    "UklGRiQAAABXQVZFZm10IBAAAAABAAEAQB8AAEAfAAABAAgAZGF0YQAAAAA=";
