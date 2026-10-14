//! `bas` — the Bas agent service daemon.
//!
//! The daemon owns agent sessions, exposes them over gRPC, and reports
//! completion to the user through desktop notifications. It is built to run
//! under systemd: logs go to journald, readiness is signalled with `sd_notify`,
//! and `WatchdogSec=` is honored.

mod agent;
mod clap;
mod config;
mod management;
mod notify;
mod service;
mod session;
mod storage;
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
        "bas starting"
    );

    run(config).await
}

/// Builds the daemon's runtime and serves until a shutdown signal arrives.
async fn run(config: Config) -> Result<()> {
    let shutdown = system::shutdown_signal().context("cannot install shutdown handlers")?;
    let watcher = Watcher::connect(config.notify).await?;
    let storage: Arc<dyn storage::Storage> = match config.localdb_path()? {
        Some(path) => Arc::new(
            storage::LocalDb::open(&path)
                .await
                .with_context(|| format!("cannot open session database {}", path.display()))?,
        ),
        None => Arc::new(storage::Memory::new(config.limits.history_capacity)),
    };
    let mut registry = Registry::with_storage(
        Arc::new(Catalog::from_registrations(config.agents.clone())?),
        Arc::new(LocalHost::new(config.permissions)),
        config.limits,
        storage,
    );
    registry
        .recover()
        .await
        .context("cannot recover stored sessions")?;
    if let Some(watcher) = &watcher {
        registry = registry.with_completions(watcher.sender.clone());
    }
    let registry = Arc::new(registry);
    let manager = Arc::new(management::Manager::new(
        config
            .catalog_source
            .as_ref()
            .and_then(|source| source.settings_store()),
    )?);
    let listener = tokio::net::TcpListener::bind(config.listen)
        .await
        .with_context(|| format!("cannot listen on {}", config.listen))?;
    let local = listener
        .local_addr()
        .context("cannot read the bound address")?;
    let stopping = tokio_util::sync::CancellationToken::new();
    let catalog_watcher = config.catalog_source.map(|source| {
        config::watch_catalog(
            source,
            config.agents,
            Arc::clone(&registry),
            stopping.clone(),
        )
    });
    let server = tonic::transport::Server::builder()
        .add_service(
            service::Service::new(Arc::clone(&registry))
                .with_manager(Arc::clone(&manager))
                .into_server(),
        )
        .serve_with_incoming_shutdown(
            tokio_stream::wrappers::TcpListenerStream::new(listener),
            stopping.clone().cancelled_owned(),
        );
    tokio::pin!(server);
    let watchdog = system::spawn_watchdog();
    system::ready();
    ::tracing::info!(address = %local, "serving the bas.v1.Bas service");
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
        let (_, _, result) = tokio::join!(manager.shutdown(), registry.shutdown(), finish_server);
        if let Some(catalog_watcher) = catalog_watcher {
            catalog_watcher.await.context("catalog watcher failed")?;
        }
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
