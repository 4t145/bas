//! Daemon-owned discovery and installation; no settings-page state or presentation.

use bas::service::tls::ConnectionOptions as Target;
use std::time::Duration;

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

mod policy;

#[derive(Clone)]
pub(crate) struct Service {
    server: Target,
    runtime: Handle,
}
impl Global for Service {}

pub(super) fn init(server: Target, runtime: Handle, cx: &mut App) {
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
    pub(crate) fn configuration(
        &self,
        save: Option<pb::SaveAgentConfigurationRequest>,
    ) -> impl Future<Output = Result<pb::AgentConfiguration, String>> + use<> {
        let server = self.server.clone();
        let (send, receive) = oneshot::channel();
        let task = AbortOnDropHandle::new(self.runtime.spawn(async move {
            let result = tokio::time::timeout(CATALOG_TIMEOUT, async {
                let mut client = Client::connect_options(&server)
                    .await
                    .map_err(|error| error.to_string())?;
                let response = match save {
                    Some(request) => client.save_agent_configuration(request).await,
                    None => {
                        client
                            .get_agent_configuration(pb::GetAgentConfigurationRequest {})
                            .await
                    }
                };
                response
                    .map(|response| response.into_inner())
                    .map_err(|error| {
                        if error.code() == tonic::Code::Unimplemented {
                            crate::i18n::tr!("agent-config-upgrade").to_owned()
                        } else {
                            error.message().to_owned()
                        }
                    })
            })
            .await
            .unwrap_or_else(|_| Err(crate::i18n::tr!("agent-config-timeout").into()));
            let _ = send.send(result);
        }));
        async move {
            let _task = task;
            receive
                .await
                .unwrap_or_else(|_| Err(crate::i18n::tr!("agent-config-ended").into()))
        }
    }

    pub(crate) fn new(server: Target, runtime: Handle) -> Self {
        Self { server, runtime }
    }

    pub(crate) fn address(&self) -> Target {
        self.server.clone()
    }

    pub(crate) fn runtime(&self) -> &Handle {
        &self.runtime
    }

    pub(crate) fn local(&self) -> bool {
        // A forwarded loopback port does not establish ownership of local files.
        self.server.is_local()
    }

    pub(crate) fn catalog(
        &self,
        refresh: bool,
    ) -> impl Future<Output = Result<Catalog, String>> + use<> {
        let server = self.server.clone();
        let (send, receive) = oneshot::channel();
        let task = AbortOnDropHandle::new(self.runtime.spawn(async move {
            let result = tokio::time::timeout(CATALOG_TIMEOUT, async {
                let mut client = Client::connect_options(&server)
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
        let server = self.server.clone();
        let (send, receiver) = mpsc::unbounded();
        let task = AbortOnDropHandle::new(self.runtime.spawn(async move {
            let result = async {
                let mut client = Client::connect_options(&server)
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
        let server = self.server.clone();
        let (send, receive) = oneshot::channel();
        let task = AbortOnDropHandle::new(self.runtime.spawn(async move {
            let result = async {
                let mut client = Client::connect_options(&server)
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

    pub(crate) fn logout(
        &self,
        spec: AgentSpec,
    ) -> impl Future<Output = Result<(), String>> + use<> {
        let server = self.server.clone();
        let (send, receive) = oneshot::channel();
        let task = AbortOnDropHandle::new(self.runtime.spawn(async move {
            let result = tokio::time::timeout(CATALOG_TIMEOUT, async {
                let mut client = super::check::connect_client(server).await?;
                let expected_spec_json =
                    serde_json::to_vec(&spec).map_err(|error| error.to_string())?;
                client
                    .logout_agent(pb::LogoutAgentRequest {
                        agent: spec.id.to_string(),
                        expected_spec_json,
                    })
                    .await
                    .map_err(|error| {
                        if error.code() == tonic::Code::Unimplemented {
                            crate::i18n::tr!("agent-check-upgrade").into()
                        } else {
                            crate::connection::Failure::from(error).presentation()
                        }
                    })?;
                Ok(())
            })
            .await
            .unwrap_or_else(|_| Err(crate::i18n::tr!("agent-logout-unknown").into()));
            let _ = send.send(result);
        }));
        async move {
            let _task = task;
            receive
                .await
                .unwrap_or_else(|_| Err(crate::i18n::tr!("agent-logout-unknown").into()))
        }
    }
}

#[cfg(all(test, feature = "gui-tests"))]
mod tests;
