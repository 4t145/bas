//! Daemon-owned discovery and installation; no settings-page state or presentation.

use std::{net::SocketAddr, path::Path, time::Duration};

use bas::{
    config::AgentSpec,
    management::Catalog,
    service::{Client, pb},
};
use futures::{
    StreamExt,
    channel::{mpsc, oneshot},
};
use gpui_kit::{App, Global};
use tokio::runtime::Handle;
use tokio_util::task::AbortOnDropHandle;

const CATALOG_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Clone)]
pub(crate) struct Service {
    server: SocketAddr,
    runtime: Handle,
}
impl Global for Service {}

pub(super) fn init(server: SocketAddr, runtime: Handle, cx: &mut App) {
    cx.set_global(Service { server, runtime });
}

pub(crate) struct Progress {
    pub message: String,
    pub complete: bool,
    pub spec: Option<AgentSpec>,
}

fn decode(operation: pb::AgentOperation) -> Progress {
    let spec = if operation.complete {
        serde_json::from_slice(&operation.spec_json).ok()
    } else {
        None
    };
    Progress {
        message: operation.message,
        complete: operation.complete,
        spec,
    }
}

pub(crate) struct Updates {
    receiver: mpsc::UnboundedReceiver<Result<Progress, String>>,
    _task: AbortOnDropHandle<()>,
}

impl Updates {
    pub(crate) async fn next(&mut self) -> Option<Result<Progress, String>> {
        self.receiver.next().await
    }
}

impl Service {
    pub(crate) fn new(server: SocketAddr, runtime: Handle) -> Self {
        Self { server, runtime }
    }

    pub(crate) fn address(&self) -> SocketAddr {
        self.server
    }

    pub(crate) fn local(&self) -> bool {
        self.server.ip().is_loopback()
    }

    pub(crate) fn catalog(
        &self,
        refresh: bool,
    ) -> impl Future<Output = Result<Catalog, String>> + use<> {
        let server = self.server;
        let (send, receive) = oneshot::channel();
        let task = AbortOnDropHandle::new(self.runtime.spawn(async move {
            let result = tokio::time::timeout(CATALOG_TIMEOUT, async {
                let mut client = Client::connect_at(server)
                    .await
                    .map_err(|error| error.to_string())?;
                let response = client
                    .list_agent_catalog(pb::ListAgentCatalogRequest { refresh })
                    .await
                    .map_err(|error| error.message().to_owned())?
                    .into_inner();
                serde_json::from_slice(&response.catalog_json).map_err(|error| error.to_string())
            })
            .await
            .unwrap_or_else(|_| Err(crate::i18n::tr!("catalog-timeout").into()));
            let _ = send.send(result);
        }));
        async move {
            let _task = task;
            receive
                .await
                .unwrap_or_else(|_| Err(crate::i18n::tr!("catalog-ended").into()))
        }
    }

    pub(crate) fn install(&self, id: String) -> Updates {
        let server = self.server;
        let (send, receiver) = mpsc::unbounded();
        let task = AbortOnDropHandle::new(self.runtime.spawn(async move {
            let result = async {
                if !server.ip().is_loopback() {
                    return Err(crate::i18n::tr!("agent-install-remote").into());
                }
                let mut client = Client::connect_at(server)
                    .await
                    .map_err(|error| error.to_string())?;
                let mut stream = client
                    .install_agent(pb::InstallAgentRequest { id })
                    .await
                    .map_err(|error| error.message().to_owned())?
                    .into_inner();
                while let Some(operation) = stream
                    .message()
                    .await
                    .map_err(|error| error.message().to_owned())?
                {
                    let complete = operation.complete;
                    if send.unbounded_send(Ok(decode(operation))).is_err() {
                        return Ok(());
                    }
                    if complete {
                        return Ok(());
                    }
                }
                Err(crate::i18n::tr!("agent-install-incomplete").to_owned())
            }
            .await;
            if let Err(error) = result {
                let _ = send.unbounded_send(Err(error));
            }
        }));
        Updates {
            receiver,
            _task: task,
        }
    }

    pub(crate) fn remove(&self, id: String) -> impl Future<Output = Result<(), String>> + use<> {
        let server = self.server;
        let (send, receive) = oneshot::channel();
        let task = AbortOnDropHandle::new(self.runtime.spawn(async move {
            let result = async {
                if !server.ip().is_loopback() {
                    return Err(crate::i18n::tr!("agent-remove-remote").into());
                }
                let mut client = Client::connect_at(server)
                    .await
                    .map_err(|error| error.to_string())?;
                client
                    .remove_agent(pb::RemoveAgentRequest { id })
                    .await
                    .map_err(|error| error.message().to_owned())?;
                Ok(())
            }
            .await;
            let _ = send.send(result);
        }));
        async move {
            let _task = task;
            receive
                .await
                .unwrap_or_else(|_| Err(crate::i18n::tr!("agent-remove-ended").into()))
        }
    }
}

/// Both sides must name the same file before the GUI writes local configuration.
pub(crate) fn settings_match(catalog: &Catalog, path: &Path) -> bool {
    catalog.settings_path.as_ref().is_some_and(|daemon_path| {
        let canonical =
            |path: &Path| std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
        canonical(daemon_path) == canonical(path)
    })
}

#[cfg(all(test, feature = "gui-tests"))]
mod tests;
