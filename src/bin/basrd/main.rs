//! `basrd` — the BASR agent service daemon.
//!
//! The daemon owns agent sessions, exposes them over gRPC, and reports
//! completion to the user through desktop notifications. It is built to run
//! under systemd: logs go to journald, readiness is signalled with `sd_notify`,
//! and `WatchdogSec=` is honored.

mod clap;
mod system;
mod tracing;

use std::sync::Arc;

use ::clap::Parser;
use anyhow::{Context, Result};
use basr::{
    agent::{Catalog, Driver, Kind, LocalHost, PermissionPolicy, acp, embedded},
    config::{Config, Notify},
    notify::{self, Dbus, Notifier},
    service,
    session::Registry,
};

#[tokio::main]
async fn main() -> Result<()> {
    let args = clap::Args::parse();
    let config = args.config()?;

    let backend = tracing::init(
        args.tracing()?
            .with_syslog_identifier(tracing::DEFAULT_SYSLOG_IDENTIFIER),
    )?;

    ::tracing::info!(
        backend = %backend,
        listen = %config.listen,
        agents = config.agents.len(),
        "basrd starting"
    );

    run(config).await
}

/// Builds the daemon's runtime and serves until a shutdown signal arrives.
async fn run(config: Config) -> Result<()> {
    let catalog = Arc::new(catalog(&config)?);
    let registry = Arc::new(Registry::new(
        catalog,
        Arc::new(LocalHost::new(PermissionPolicy::Allow)),
        config.limits,
    ));

    let watcher = match notifier(config.notify).await {
        Some(notifier) => Some(notify::watch_completions(Arc::clone(&registry), notifier)),
        None => {
            ::tracing::info!("desktop notifications are disabled");
            None
        }
    };

    let listener = tokio::net::TcpListener::bind(config.listen)
        .await
        .with_context(|| format!("cannot listen on {}", config.listen))?;
    let local = listener
        .local_addr()
        .context("cannot read the bound address")?;

    let watchdog = system::spawn_watchdog();
    system::ready();
    ::tracing::info!(address = %local, "serving the basr.v1.Basr service");

    let result = tonic::transport::Server::builder()
        .add_service(service::Service::new(registry).into_server())
        .serve_with_incoming_shutdown(
            tokio_stream::wrappers::TcpListenerStream::new(listener),
            async {
                let _ = system::shutdown_signal().await;
                // Attached clients hold long-lived streams, so bound the drain.
                tokio::spawn(async {
                    tokio::time::sleep(system::SHUTDOWN_GRACE).await;
                    ::tracing::warn!("shutdown grace expired; exiting");
                    std::process::exit(0);
                });
            },
        )
        .await
        .context("the gRPC server failed");

    system::stopping();
    if let Some(watchdog) = watchdog {
        watchdog.abort();
    }
    if let Some(watcher) = watcher {
        watcher.abort();
    }

    result
}

/// Builds the catalog of launchable agent backends.
fn catalog(config: &Config) -> Result<Catalog> {
    let mut catalog = Catalog::new();

    for spec in &config.agents {
        let driver: Arc<dyn Driver> = match spec.kind {
            Kind::Embedded => Arc::new(embedded::Driver::new()),
            Kind::Acp => Arc::new(acp::Driver::new()),
        };

        catalog
            .insert(spec.clone(), driver)
            .with_context(|| format!("cannot configure agent {}", spec.id))?;
    }

    Ok(catalog)
}

/// Connects the notifier named by the configuration, if any.
async fn notifier(policy: Notify) -> Option<Arc<dyn Notifier>> {
    match policy {
        Notify::Never => None,
        Notify::Auto | Notify::Always => match Dbus::connect().await {
            Ok(notifier) => Some(Arc::new(notifier)),
            Err(error) => {
                ::tracing::warn!(%error, "no desktop session bus; notifications are off");
                None
            }
        },
    }
}
