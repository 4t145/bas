use anyhow::{Context, Result};
use tracing_journald::Layer;
use tracing_subscriber::{EnvFilter, prelude::*};

use super::Config;

pub(super) fn make_layer(config: &Config) -> std::io::Result<Layer> {
    let mut layer = tracing_journald::layer()?;

    if let Some(identifier) = config.syslog_identifier() {
        layer = layer.with_syslog_identifier(identifier.to_owned());
    }

    Ok(layer)
}

pub(super) fn install(filter: EnvFilter, layer: Layer) -> Result<()> {
    tracing_subscriber::registry()
        .with(filter)
        .with(layer)
        .try_init()
        .context("failed to install the journald tracing subscriber")
}
