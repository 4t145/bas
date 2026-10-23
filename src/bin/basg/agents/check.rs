//! The selected daemon, not the GUI's PATH, determines backend availability.

use bas::service::tls::ConnectionOptions as Target;
use std::time::Duration;

use bas::{
    config::AgentSpec,
    service::{Client, pb},
};
use gpui_kit::{App, Global};
use tokio::runtime::Handle;
use tokio_util::task::AbortOnDropHandle;

use super::Status;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(40);
const RELOAD_INTERVAL: Duration = Duration::from_millis(500);
const RELOAD_ATTEMPTS: usize = 6;

#[cfg(test)]
mod tests;

#[derive(Clone)]
pub(crate) struct Service {
    server: Target,
    runtime: Handle,
}

impl Global for Service {}

pub(crate) fn init(server: Target, runtime: Handle, cx: &mut App) {
    cx.set_global(Service { server, runtime });
}

impl Service {
    pub(crate) fn new(server: Target, runtime: Handle) -> Self {
        Self { server, runtime }
    }

    pub(crate) fn address(&self) -> Target {
        self.server.clone()
    }

    pub(crate) fn runtime(&self) -> Handle {
        self.runtime.clone()
    }

    pub(crate) fn request(
        &self,
        spec: AgentSpec,
        connect: bool,
    ) -> impl Future<Output = Status> + use<> {
        let server = self.server.clone();
        let (send, receive) = futures::channel::oneshot::channel();
        let task = AbortOnDropHandle::new(self.runtime.spawn(async move {
            let result = tokio::time::timeout(REQUEST_TIMEOUT, request(server, spec, connect))
                .await
                .unwrap_or_else(|_| Err(crate::i18n::tr!("agent-check-timeout").into()));
            let _ = send.send(result.unwrap_or_else(Status::Failed));
        }));
        async move {
            let _task = task;
            receive
                .await
                .unwrap_or_else(|_| Status::Failed(crate::i18n::tr!("agent-check-ended").into()))
        }
    }
}

/// # Errors
/// Reports connection failures; request RPCs validate compatibility and the expected saved spec.
pub(crate) async fn connect_client(server: Target) -> Result<Client, String> {
    let client = Client::connect_options(&server)
        .await
        .map_err(|_| crate::i18n::tr!("service-connect-failed", server = (server).to_string()))?;
    Ok(client)
}

/// # Errors
/// Rejects stale specs, incompatible daemons and ACP transport failures.
async fn request(server: Target, spec: AgentSpec, connect: bool) -> Result<Status, String> {
    let mut client = connect_client(server).await?;
    let expected_spec_json = serde_json::to_vec(&spec).map_err(|error| error.to_string())?;
    for attempt in 0..RELOAD_ATTEMPTS {
        let result = client
            .check_agent(pb::CheckAgentRequest {
                agent: spec.id.to_string(),
                expected_spec_json: expected_spec_json.clone(),
                connect,
            })
            .await;
        match result {
            Ok(response) => {
                return decode_response(response.into_inner());
            }
            Err(error)
                if matches!(
                    error.code(),
                    tonic::Code::FailedPrecondition | tonic::Code::NotFound
                ) && attempt + 1 < RELOAD_ATTEMPTS =>
            {
                // Give the daemon's configuration watcher time to apply a just-saved registration.
                tokio::time::sleep(RELOAD_INTERVAL).await;
            }
            Err(error) => {
                return Err(match error.code() {
                    tonic::Code::Unimplemented => crate::i18n::tr!("agent-check-upgrade").into(),
                    tonic::Code::FailedPrecondition | tonic::Code::NotFound => {
                        crate::i18n::tr!("agent-config-unloaded").into()
                    }
                    _ => {
                        crate::i18n::tr!("agent-check-error", error = (error.message()).to_string())
                    }
                });
            }
        }
    }
    unreachable!("the last retry returns its result")
}

/// # Errors
/// Rejects missing or malformed ACP authentication data.
pub(crate) fn decode_response(response: pb::CheckAgentResponse) -> Result<Status, String> {
    Ok(match response.status() {
        pb::AgentCheckStatus::Connected => Status::Connected(response.resolved_program),
        pb::AgentCheckStatus::Installed => Status::Installed(response.resolved_program),
        pb::AgentCheckStatus::AuthenticationRequired => {
            let initialization = response
                .initialization
                .ok_or_else(|| crate::i18n::tr!("auth-methods-missing").to_owned())?
                .decode::<serde_json::Value>()
                .map_err(|_| crate::i18n::tr!("auth-invalid-info").to_owned())?;
            Status::AuthenticationRequired {
                program: response.resolved_program,
                methods: authentication_methods(initialization, response.authentication_error)?,
            }
        }
        pb::AgentCheckStatus::MissingExecutable => Status::Missing(response.message),
        _ => Status::Failed(response.message),
    })
}

/// # Errors
/// Rejects malformed native errors or explicit method lists instead of using stale methods.
fn authentication_methods(
    initialization: serde_json::Value,
    error: Option<pb::AcpPayload>,
) -> Result<Vec<bas::authentication::Method>, String> {
    let error = error
        .map(|payload| payload.decode::<serde_json::Value>())
        .transpose()
        .map_err(|_| crate::i18n::tr!("auth-invalid-response").to_owned())?;
    bas::authentication::methods(&initialization, error.as_ref())
        .map_err(|error| crate::i18n::tr!("auth-info-error", error = (error).to_string()))
}
