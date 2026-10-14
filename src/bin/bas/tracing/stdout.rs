use anyhow::{Result, anyhow};
use tracing_subscriber::EnvFilter;

use super::Config;

pub(super) fn install(config: &Config, filter: EnvFilter) -> Result<()> {
    tracing_subscriber::fmt()
        .compact()
        .with_env_filter(filter)
        .with_ansi(config.ansi())
        .with_target(config.includes_target())
        .try_init()
        .map_err(|error| anyhow!("failed to install the stdout tracing subscriber: {error}"))
}
