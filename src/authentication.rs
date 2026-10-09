//! Authentication descriptors normalized from the original ACP JSON payloads.
//!
//! Execution comes from protocol fields and documented compatibility contracts;
//! method names and descriptions remain presentation data.

use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub mod elicitation;
mod normalization;

pub use normalization::methods;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Source {
    Initialize,
    AuthenticationError,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Method {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub raw: Value,
    pub source: Source,
    pub execution: Execution,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Execution {
    Agent,
    Terminal(Terminal),
    Unsupported { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Terminal {
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub arguments: Arguments,
    /// Legacy descriptor identity, checked against the configured executable or entry point.
    pub command: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Arguments {
    /// Standard ACP: append to the complete configured invocation.
    Append,
    /// Verified legacy application descriptor: retain the launcher and replace application arguments.
    ReplaceApplication,
    /// Complete legacy command descriptor: its args include the complete invocation.
    ReplaceInvocation,
}

/// A terminal launch plan validated by the daemon against the current Agent configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invocation {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid authentication descriptor: {0}")]
    Descriptor(String),
    #[error("unsupported elicitation request: {0}")]
    Elicitation(String),
    #[error("invalid elicitation response: {0}")]
    Response(String),
}

#[cfg(test)]
mod tests;
