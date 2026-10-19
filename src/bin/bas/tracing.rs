//! Logging setup for the daemon.
//!
//! The module installs one global `tracing` subscriber. `Stdout` is suitable
//! both for local development and for systemd's default stdout-to-journal
//! collection. `Journald` writes structured fields directly to journald.

mod stdout;

#[cfg(target_os = "linux")]
mod journald;

#[cfg(target_os = "linux")]
mod journal_stream;

#[cfg(test)]
mod tests;

use std::{fmt, str::FromStr};

#[cfg(not(target_os = "linux"))]
use anyhow::bail;
use anyhow::{Context, Result};
use tracing_subscriber::EnvFilter;

/// Environment variable used by the command-line parser to choose a backend.
pub const BACKEND_ENV: &str = "BAS_LOG_BACKEND";

/// Identifier the daemon reports to journald by default.
pub const DEFAULT_SYSLOG_IDENTIFIER: &str = "bas";

/// A destination for tracing events.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Backend {
    /// Use journald when stdout is connected to the journal; otherwise use
    /// stdout. If the journald socket cannot be opened, initialization falls
    /// back to stdout.
    #[default]
    Auto,
    /// Write human-readable, compact log lines to stdout.
    Stdout,
    /// Write structured events directly to journald.
    Journald,
}

impl fmt::Display for Backend {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Auto => "auto",
            Self::Stdout => "stdout",
            Self::Journald => "journald",
        })
    }
}

impl FromStr for Backend {
    type Err = ParseBackendError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "stdout" => Ok(Self::Stdout),
            "journal" | "journald" => Ok(Self::Journald),
            _ => Err(ParseBackendError {
                value: value.to_owned(),
            }),
        }
    }
}

/// Error returned when a log backend name is unknown.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("unknown tracing backend {value:?}; expected auto, stdout, or journald")]
pub struct ParseBackendError {
    value: String,
}

/// Configuration shared by all logging backends.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Config {
    backend: Backend,
    filter: String,
    ansi: bool,
    include_target: bool,
    syslog_identifier: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            backend: Backend::Auto,
            filter: "info".to_owned(),
            // Escape sequences are undesirable in journal output. Callers can
            // explicitly enable them for interactive development.
            ansi: false,
            include_target: true,
            // tracing-journald derives this from the executable by default.
            syslog_identifier: None,
        }
    }
}

impl Config {
    /// Creates a configuration for an explicitly selected backend.
    pub fn new(backend: Backend) -> Self {
        Self {
            backend,
            ..Self::default()
        }
    }

    pub fn with_filter(mut self, filter: impl Into<String>) -> Self {
        self.filter = filter.into();
        self
    }

    pub fn with_syslog_identifier(mut self, identifier: impl Into<String>) -> Self {
        self.syslog_identifier = Some(identifier.into());
        self
    }

    #[cfg(test)]
    pub fn backend(&self) -> Backend {
        self.backend
    }

    #[cfg(test)]
    pub fn filter(&self) -> &str {
        &self.filter
    }

    pub fn ansi(&self) -> bool {
        self.ansi
    }

    pub fn includes_target(&self) -> bool {
        self.include_target
    }

    pub fn syslog_identifier(&self) -> Option<&str> {
        self.syslog_identifier.as_deref()
    }

    fn env_filter(&self) -> Result<EnvFilter> {
        EnvFilter::try_new(&self.filter)
            .with_context(|| format!("invalid tracing filter {:?}", self.filter))
    }

    /// Installs the configured process-global subscriber and returns the backend
    /// that was actually selected.
    ///
    /// This function must be called at most once per process. In `Auto` mode,
    /// failure to open the journald socket causes a fallback to stdout. An
    /// explicitly requested `Journald` backend returns that failure to the caller.
    ///
    /// # Errors
    /// Returns an error if the filter is invalid, the subscriber is already installed,
    /// or a required backend is unavailable.
    pub fn init(self) -> Result<Backend> {
        let filter = self.env_filter()?;

        match self.backend {
            Backend::Auto => self.init_auto(filter),
            Backend::Stdout => {
                stdout::install(&self, filter)?;
                Ok(Backend::Stdout)
            }
            Backend::Journald => self.init_journald(filter),
        }
    }

    /// # Errors
    /// Returns an error if installing the selected subscriber fails.
    fn init_auto(&self, filter: EnvFilter) -> Result<Backend> {
        #[cfg(target_os = "linux")]
        if journal_stream::stdout_is_journal() {
            return match journald::make_layer(self) {
                Ok(layer) => {
                    journald::install(filter, layer)?;
                    Ok(Backend::Journald)
                }
                Err(error) => {
                    stdout::install(self, filter)?;
                    ::tracing::warn!(
                        error = %error,
                        "journald is unavailable; using the stdout tracing backend"
                    );
                    Ok(Backend::Stdout)
                }
            };
        }

        stdout::install(self, filter)?;
        Ok(Backend::Stdout)
    }

    /// # Errors
    /// Returns an error if journald is unsupported, unavailable, or cannot be installed.
    fn init_journald(&self, filter: EnvFilter) -> Result<Backend> {
        #[cfg(target_os = "linux")]
        {
            let layer = journald::make_layer(self).context("failed to open the journald socket")?;
            journald::install(filter, layer)?;
            Ok(Backend::Journald)
        }

        #[cfg(not(target_os = "linux"))]
        {
            let _ = (self, filter);
            bail!("the journald tracing backend is only supported on Linux")
        }
    }
}
