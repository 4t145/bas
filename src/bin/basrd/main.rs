//! `basrd` — the BASR agent service daemon.
//!
//! The daemon owns agent sessions, exposes them over gRPC, and reports
//! completion to the user through desktop notifications. It is built to run
//! under systemd: logs go to journald, readiness is signalled with `sd_notify`,
//! and `WatchdogSec=` is honored.

mod agent;
mod clap;
mod config;
mod notify;
mod service;
mod session;
mod system;
mod tracing;

use std::sync::Arc;

use ::clap::Parser;
use anyhow::{Context, Result};

use crate::{
    agent::{Catalog, LocalHost},
    config::Config,
    notify::Watcher,
    session::Registry,
};

#[tokio::main]
async fn main() -> Result<()> {
    let args = clap::Args::parse();
    let config = args.config()?;

    let backend = args
        .tracing()
        .with_syslog_identifier(tracing::DEFAULT_SYSLOG_IDENTIFIER)
        .init()?;

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
    let shutdown = system::shutdown_signal().context("cannot install shutdown handlers")?;
    let watcher = Watcher::connect(config.notify).await?;
    let mut registry = Registry::new(
        Arc::new(Catalog::from_specs(config.agents)?),
        Arc::new(LocalHost::new(config.permissions)),
        config.limits,
    );
    if let Some(watcher) = &watcher {
        registry = registry.with_completions(watcher.sender.clone());
    }
    let registry = Arc::new(registry);
    let listener = tokio::net::TcpListener::bind(config.listen)
        .await
        .with_context(|| format!("cannot listen on {}", config.listen))?;
    let local = listener
        .local_addr()
        .context("cannot read the bound address")?;
    let stopping = tokio_util::sync::CancellationToken::new();
    let server = tonic::transport::Server::builder()
        .add_service(service::Service::new(Arc::clone(&registry)).into_server())
        .serve_with_incoming_shutdown(
            tokio_stream::wrappers::TcpListenerStream::new(listener),
            stopping.clone().cancelled_owned(),
        );
    tokio::pin!(server);
    let watchdog = system::spawn_watchdog();
    system::ready();
    ::tracing::info!(address = %local, "serving the basr.v1.Basr service");
    let (server_result, signal_result) = tokio::select! {
        result = &mut server => (Some(result), Ok(())),
        result = shutdown => (None, result),
    };
    system::stopping();
    stopping.cancel();
    let cleanup = async {
        let finish_server = async {
            match server_result {
                Some(result) => result,
                None => server.await,
            }
        };
        let (_, result) = tokio::join!(registry.shutdown(), finish_server);
        if let Some(watcher) = watcher {
            watcher.shutdown().await;
        }
        result.context("the gRPC server failed")
    };
    let result = tokio::time::timeout(system::SHUTDOWN_GRACE, cleanup)
        .await
        .context("shutdown grace expired")?;
    if let Some(watchdog) = watchdog {
        watchdog.abort();
        let _ = watchdog.await;
    }
    signal_result.context("shutdown signal failed")?;
    result
}
