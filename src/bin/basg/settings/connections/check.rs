use super::*;
use bas::service::{Client, Target, pb, tls::ConnectionOptions};
use std::{error::Error as StdError, io::ErrorKind, time::Duration};
use tokio_util::task::AbortOnDropHandle;

const CHECK_TIMEOUT: Duration = Duration::from_secs(5);

#[cfg(test)]
pub(super) mod tests;

#[derive(Debug, thiserror::Error)]
pub(super) enum Error {
    #[error("connection check timed out")]
    Timeout,
    #[error("network runtime is unavailable")]
    Runtime,
    #[error(transparent)]
    Connect(#[from] bas::service::tls::Error),
    #[error(transparent)]
    Rpc(#[from] tonic::Status),
}

impl Error {
    pub(super) fn message(&self, target: &Target) -> String {
        use bas::service::tls::Error as TlsError;
        match self {
            Self::Timeout => crate::i18n::tr!("connections-check-timeout").into(),
            Self::Runtime => crate::i18n::tr!("connections-check-unavailable").into(),
            Self::Connect(TlsError::Configuration | TlsError::Empty) => {
                crate::i18n::tr!("connections-check-tls-settings").into()
            }
            Self::Connect(TlsError::Read { path, .. }) => {
                crate::i18n::tr!("connections-check-file", path = path.display().to_string())
            }
            Self::Connect(error) => transport_message(error, target),
            Self::Rpc(status) => match status.code() {
                tonic::Code::DeadlineExceeded => {
                    crate::i18n::tr!("connections-check-timeout").into()
                }
                tonic::Code::PermissionDenied | tonic::Code::Unauthenticated => {
                    crate::i18n::tr!("connections-check-denied").into()
                }
                tonic::Code::Unimplemented => {
                    crate::i18n::tr!("connections-check-incompatible").into()
                }
                tonic::Code::Unavailable | tonic::Code::Unknown => {
                    transport_message(status, target)
                }
                _ => crate::i18n::tr!("connections-check-response").into(),
            },
        }
    }
}

fn transport_message(error: &(dyn StdError + 'static), target: &Target) -> String {
    let mut cause = Some(error);
    while let Some(error) = cause {
        if error.is::<rustls::Error>() {
            return crate::i18n::tr!("connections-check-tls").into();
        }
        if let Some(error) = error.downcast_ref::<std::io::Error>() {
            match error.kind() {
                ErrorKind::ConnectionRefused => {
                    return crate::i18n::tr!("connections-check-refused").into();
                }
                ErrorKind::TimedOut => return crate::i18n::tr!("connections-check-timeout").into(),
                ErrorKind::PermissionDenied => {
                    return crate::i18n::tr!("connections-check-denied").into();
                }
                ErrorKind::Unsupported => {
                    return crate::i18n::tr!("connections-check-unsupported").into();
                }
                ErrorKind::NotFound if target.is_local() => {
                    return crate::i18n::tr!("connections-check-local-missing").into();
                }
                _ => {}
            }
        }
        cause = error.source();
    }
    if matches!(target, Target::Mtls(_)) {
        crate::i18n::tr!("connections-check-mtls-failed").into()
    } else {
        crate::i18n::tr!("connections-check-failed").into()
    }
}

/// # Errors
/// Reports connection, authentication, certificate-file, RPC and total deadline failures.
pub(super) async fn probe(options: ConnectionOptions, timeout: Duration) -> Result<(), Error> {
    tokio::time::timeout(timeout, async {
        let mut client = Client::connect_options(&options).await?;
        client.list_agents(pb::ListAgentsRequest {}).await?;
        Ok(())
    })
    .await
    .map_err(|_| Error::Timeout)?
}

pub(super) enum State {
    Checking,
    Complete(Result<(), Error>),
}

pub(super) struct Entry {
    options: ConnectionOptions,
    generation: u64,
    pub(super) state: State,
    _task: gpui_kit::Task<()>,
}

impl Page {
    pub(super) fn prune_checks(&mut self, cx: &App) {
        let profiles = self.profiles.read(cx).entries();
        self.checks.retain(|id, entry| {
            profiles
                .iter()
                .any(|profile| profile.id == *id && profile.options() == entry.options)
        });
    }

    pub(super) fn check_state(&self, profile: &ConnectionProfile) -> Option<&State> {
        self.checks
            .get(&profile.id)
            .filter(|entry| entry.options == profile.options())
            .map(|entry| &entry.state)
    }

    pub(super) fn check_connection(&mut self, profile: &ConnectionProfile, cx: &mut Context<Self>) {
        if self.editor.is_some()
            || matches!(self.check_state(profile), Some(State::Checking))
            || !self.profiles.read(cx).entries().contains(profile)
        {
            return;
        }
        let options = profile.options();
        let request = cx
            .try_global::<crate::agents::catalog::Service>()
            .map(|service| {
                AbortOnDropHandle::new(
                    service
                        .runtime()
                        .spawn(probe(options.clone(), CHECK_TIMEOUT)),
                )
            });
        self.begin_check(
            profile.id.clone(),
            options,
            async move {
                match request {
                    Some(task) => task.await.unwrap_or(Err(Error::Runtime)),
                    None => Err(Error::Runtime),
                }
            },
            cx,
        );
    }

    fn begin_check(
        &mut self,
        id: String,
        options: ConnectionOptions,
        request: impl Future<Output = Result<(), Error>> + 'static,
        cx: &mut Context<Self>,
    ) {
        self.check_generation += 1;
        let generation = self.check_generation;
        let checked_id = id.clone();
        let task = cx.spawn(async move |page, cx| {
            let result = request.await;
            let _ = page.update(cx, |page, cx| {
                page.finish_check(&checked_id, generation, result, cx);
            });
        });
        self.checks.insert(
            id,
            Entry {
                options,
                generation,
                state: State::Checking,
                _task: task,
            },
        );
        cx.notify();
    }

    fn finish_check(
        &mut self,
        id: &str,
        generation: u64,
        result: Result<(), Error>,
        cx: &mut Context<Self>,
    ) {
        self.prune_checks(cx);
        if let Some(entry) = self.checks.get_mut(id)
            && entry.generation == generation
        {
            match &result {
                Ok(()) => tracing::info!(
                    target: "basg::connections",
                    endpoint = %entry.options.target,
                    "Connection check succeeded"
                ),
                Err(error) => tracing::warn!(
                    target: "basg::connections",
                    endpoint = %entry.options.target,
                    ?error,
                    "Connection check failed"
                ),
            }
            entry.state = State::Complete(result);
            cx.notify();
        }
    }
}
