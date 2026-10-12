//! Owns the daemon authentication stream; dropping it cancels its ACP process.
use std::path::PathBuf;

use basr::{authentication::Method, config::AgentSpec, service::pb};
use futures::channel::mpsc as futures_mpsc;
use tokio::{runtime::Handle, sync::mpsc};
use tokio_util::task::AbortOnDropHandle;

use crate::settings::agents::check;

pub(super) struct Stream {
    pub input: mpsc::UnboundedSender<pb::AuthenticationInput>,
    pub events: futures_mpsc::UnboundedReceiver<Result<pb::AuthenticationEvent, String>>,
    pub task: AbortOnDropHandle<()>,
}

pub(super) fn start(
    service: check::Service,
    spec: AgentSpec,
    method: Method,
    path: PathBuf,
) -> Stream {
    let (input, receive) = mpsc::unbounded_channel();
    let (events, receiver) = futures_mpsc::unbounded();
    let runtime = service.runtime();
    let task = AbortOnDropHandle::new(runtime.spawn(async move {
        let result = run(service, spec, method, path, receive, events.clone()).await;
        if let Err(error) = result {
            let _ = events.unbounded_send(Err(error));
        }
    }));
    Stream {
        input,
        events: receiver,
        task,
    }
}

async fn run(
    service: check::Service,
    spec: AgentSpec,
    method: Method,
    path: PathBuf,
    mut input: mpsc::UnboundedReceiver<pb::AuthenticationInput>,
    events: futures_mpsc::UnboundedSender<Result<pb::AuthenticationEvent, String>>,
) -> Result<(), String> {
    let mut client = check::verified_client(service.address(), &path).await?;
    let saved = basr::config::Store::at(path)
        .load()
        .map_err(|_| crate::i18n::tr!("auth-check-config").to_owned())?;
    if !matches_registration(&saved.settings.daemon.agents, &spec) {
        return Err(crate::i18n::tr!("auth-config-changed").into());
    }
    let request = pb::AuthenticationInput {
        kind: Some(pb::authentication_input::Kind::Start(
            pb::AuthenticateAgentRequest {
                agent: spec.id.to_string(),
                expected_spec_json: serde_json::to_vec(&spec).map_err(|error| error.to_string())?,
                method_id: method.id.clone(),
                expected_method_json: serde_json::to_vec(&method)
                    .map_err(|error| error.to_string())?,
            },
        )),
    };
    let (send, receive) = mpsc::channel(16);
    send.send(request)
        .await
        .map_err(|_| crate::i18n::tr!("auth-ended").to_owned())?;
    let mut stream = client
        .authenticate_agent_interactive(tokio_stream::wrappers::ReceiverStream::new(receive))
        .await
        .map_err(|error| rpc_error(crate::i18n::tr!("auth-start-failed"), &error))?
        .into_inner();
    loop {
        tokio::select! {
            next = input.recv() => {
                let Some(next) = next else { return Ok(()); };
                send.send(next).await.map_err(|_| crate::i18n::tr!("auth-connection-closed").to_owned())?;
            }
            next = stream.message() => {
                let event = next.map_err(|error| rpc_error(crate::i18n::tr!("auth-failed"), &error))?
                    .ok_or_else(|| crate::i18n::tr!("auth-disconnected").to_owned())?;
                let complete = matches!(event.kind, Some(pb::authentication_event::Kind::Check(_)));
                if events.unbounded_send(Ok(event)).is_err() { return Ok(()); }
                if complete { return Ok(()); }
            }
        }
    }
}

pub(super) fn matches_registration(
    registrations: &[basr::config::AgentRegistration],
    spec: &AgentSpec,
) -> bool {
    let mut matching = registrations
        .iter()
        .filter(|registration| registration.id() == &spec.id);
    let Some(registration) = matching.next() else {
        return false;
    };
    matching.next().is_none()
        && registration
            .resolve()
            .is_ok_and(|resolved| resolved == *spec)
}

fn rpc_error(action: &str, error: &tonic::Status) -> String {
    if error.code() == tonic::Code::Unimplemented {
        return crate::i18n::tr!("auth-upgrade").into();
    }
    let message = error.message().trim();
    if message.is_empty() {
        crate::i18n::tr!(
            "auth-rpc-error",
            action = (action).to_string(),
            code = format!("{:?}", error.code())
        )
    } else {
        format!("{action}：{message}")
    }
}

pub(super) fn terminal(
    runtime: &Handle,
    plan: basr::authentication::Invocation,
) -> impl Future<Output = Result<i32, String>> + use<> {
    let (send, receive) = futures::channel::oneshot::channel();
    let task = AbortOnDropHandle::new(runtime.spawn(async move {
        let result = async {
            let invocation =
                super::terminal::Invocation::from_plan(plan).map_err(|error| error.to_string())?;
            let login = super::terminal::launch(invocation).map_err(|error| error.to_string())?;
            login.complete_code().await
        }
        .await;
        let _ = send.send(result);
    }));
    async move {
        let _task = task;
        receive
            .await
            .unwrap_or_else(|_| Err(crate::i18n::tr!("auth-terminal-ended").into()))
    }
}
