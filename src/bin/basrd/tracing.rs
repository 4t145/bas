//! Logging setup for the daemon.
//!
//! The module installs one global `tracing` subscriber. `Stdout` is suitable
//! both for local development and for systemd's default stdout-to-journal
//! collection. `Journald` writes structured fields directly to journald.

mod stdout;

#[cfg(target_os = "linux")]
mod journald;

#[cfg(test)]
mod tests;

use std::{env, error::Error, fmt, str::FromStr};

#[cfg(not(target_os = "linux"))]
use anyhow::bail;
use anyhow::{Context, Result};
use tracing_subscriber::EnvFilter;

/// Environment variable used by [`Config::from_env`] to choose a backend.
pub const BACKEND_ENV: &str = "BASRD_LOG_BACKEND";

/// Identifier the daemon reports to journald by default.
pub const DEFAULT_SYSLOG_IDENTIFIER: &str = "basrd";

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
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParseBackendError {
    value: String,
}

impl fmt::Display for ParseBackendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "unknown tracing backend {:?}; expected auto, stdout, or journald",
            self.value
        )
    }
}

impl Error for ParseBackendError {}

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

/// The builder covers every logging option even though the daemon sets only a
/// few; this module's tests exercise the whole facade.
#[allow(dead_code)]
impl Config {
    /// Creates a configuration for an explicitly selected backend.
    pub fn new(backend: Backend) -> Self {
        Self {
            backend,
            ..Self::default()
        }
    }

    /// Reads `BASRD_LOG_BACKEND` and `RUST_LOG`, falling back to the defaults
    /// when either variable is absent.
    pub fn from_env() -> Result<Self> {
        let mut config = Self::default();

        if let Some(value) = read_env(BACKEND_ENV)? {
            config.backend = value
                .parse()
                .with_context(|| format!("invalid {BACKEND_ENV}"))?;
        }

        if let Some(value) = read_env(EnvFilter::DEFAULT_ENV)? {
            config.filter = value;
        }

        // Report a bad filter as configuration failure before installing a
        // process-global subscriber.
        config.env_filter()?;
        Ok(config)
    }

    pub fn with_filter(mut self, filter: impl Into<String>) -> Self {
        self.filter = filter.into();
        self
    }

    pub fn with_backend(mut self, backend: Backend) -> Self {
        self.backend = backend;
        self
    }

    pub fn with_ansi(mut self, enabled: bool) -> Self {
        self.ansi = enabled;
        self
    }

    pub fn with_target(mut self, enabled: bool) -> Self {
        self.include_target = enabled;
        self
    }

    pub fn with_syslog_identifier(mut self, identifier: impl Into<String>) -> Self {
        self.syslog_identifier = Some(identifier.into());
        self
    }

    pub fn backend(&self) -> Backend {
        self.backend
    }

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
}

/// Installs the configured process-global subscriber and returns the backend
/// that was actually selected.
///
/// This function must be called at most once per process. In `Auto` mode,
/// failure to open the journald socket causes a fallback to stdout. An
/// explicitly requested `Journald` backend returns that failure to the caller.
pub fn init(config: Config) -> Result<Backend> {
    let filter = config.env_filter()?;

    match config.backend {
        Backend::Auto => init_auto(&config, filter),
        Backend::Stdout => {
            stdout::install(&config, filter)?;
            Ok(Backend::Stdout)
        }
        Backend::Journald => init_journald(&config, filter),
    }
}

fn init_auto(config: &Config, filter: EnvFilter) -> Result<Backend> {
    let selected = resolve_auto_backend(
        env::var_os("JOURNAL_STREAM").is_some(),
        cfg!(target_os = "linux"),
    );

    if selected == Backend::Stdout {
        stdout::install(config, filter)?;
        return Ok(Backend::Stdout);
    }

    #[cfg(target_os = "linux")]
    {
        match journald::make_layer(config) {
            Ok(layer) => {
                journald::install(filter, layer)?;
                Ok(Backend::Journald)
            }
            Err(error) => {
                stdout::install(config, filter)?;
                ::tracing::warn!(
                    error = %error,
                    "journald is unavailable; using the stdout tracing backend"
                );
                Ok(Backend::Stdout)
            }
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        // `resolve_auto_backend` selects stdout on this platform, so this is
        // unreachable. Keeping the branch explicit makes the module portable.
        unreachable!("auto selected journald on an unsupported platform")
    }
}

fn init_journald(config: &Config, filter: EnvFilter) -> Result<Backend> {
    #[cfg(target_os = "linux")]
    {
        let layer = journald::make_layer(config).context("failed to open the journald socket")?;
        journald::install(filter, layer)?;
        Ok(Backend::Journald)
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = (config, filter);
        bail!("the journald tracing backend is only supported on Linux")
    }
}

fn resolve_auto_backend(journal_stream_present: bool, journald_supported: bool) -> Backend {
    if journal_stream_present && journald_supported {
        Backend::Journald
    } else {
        Backend::Stdout
    }
}

fn read_env(name: &str) -> Result<Option<String>> {
    match env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(error).with_context(|| format!("{name} is not valid UTF-8")),
    }
}
