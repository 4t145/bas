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
        listen = ?config.listen,
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
    let mut listeners = Vec::new();
    let mut bound = Vec::new();
    let stopping = tokio_util::sync::CancellationToken::new();
    let tls = config.tls.as_ref().map(|tls| tls.load()).transpose()?;
    for target in &config.listen {
        let mut server = tonic::transport::Server::builder();
        let bind_target = match target {
            bas::service::Target::Mtls(address) => {
                server = server.tls_config(
                    tls.clone()
                        .context("mTLS listener requires server TLS configuration")?,
                )?;
                bas::service::Target::Tcp(address.clone())
            }
            _ => target.clone(),
        };
        let listener = bas::service::transport::Listener::bind(&bind_target)
            .await
            .with_context(|| format!("cannot listen on {target}"))?;
        bound.push(match (target, listener.target()) {
            (bas::service::Target::Mtls(_), bas::service::Target::Tcp(address)) => {
                bas::service::Target::Mtls(address.clone())
            }
            _ => listener.target().clone(),
        });
        listeners.push(
            server
                .add_service(
                    service::Service::new(Arc::clone(&registry))
                        .with_manager(Arc::clone(&manager))
                        .into_server(),
                )
                .serve_with_incoming_shutdown(
                    listener.incoming(),
                    stopping.clone().cancelled_owned(),
                ),
        );
    }
    let catalog_watcher = config.catalog_source.map(|source| {
        config::watch_catalog(
            source,
            config.agents,
            Arc::clone(&registry),
            stopping.clone(),
        )
    });
    let server = async { futures::future::try_join_all(listeners).await.map(|_| ()) };
    tokio::pin!(server);
    let watchdog = system::spawn_watchdog();
    system::ready();
    for target in &bound {
        ::tracing::info!(address = %target, "serving the bas.v1.Bas service");
    }
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
