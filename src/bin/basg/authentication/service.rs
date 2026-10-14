//! Owns the daemon authentication stream; dropping it cancels its ACP process.
use std::path::PathBuf;

use bas::{
    authentication::{Invocation, Method, elicitation::Request},
    config::AgentSpec,
    service::pb,
};
use futures::channel::mpsc as futures_mpsc;
use tokio::{runtime::Handle, sync::mpsc};
use tokio_util::task::AbortOnDropHandle;

use crate::agents::{Status, check};

mod failure;
pub(super) use failure::Failure;

pub(super) enum Event {
    Progress(String),
    Check(Status),
    Elicitation {
        request_id: String,
        request: Result<Request, Failure>,
    },
    ElicitationComplete(Option<serde_json::Value>),
    ElicitationCancelled(String),
    Terminal(Result<Invocation, Failure>),
    Unknown,
}

pub(super) enum Control {
    ElicitationResponse {
        request_id: String,
        response: serde_json::Value,
    },
    TerminalResult {
        exit_code: i32,
    },
}

fn encode(control: Control) -> pb::AuthenticationInput {
    use pb::authentication_input::Kind;
    let kind = match control {
        Control::ElicitationResponse {
            request_id,
            response,
        } => Kind::ElicitationResponse(pb::ElicitationAnswer {
            request_id,
            response: Some(pb::AcpPayload {
                json: serde_json::to_vec(&response).expect("JSON value serializes"),
            }),
        }),
        Control::TerminalResult { exit_code } => {
            Kind::TerminalResult(pb::TerminalAuthenticationResult { exit_code })
        }
    };
    pb::AuthenticationInput { kind: Some(kind) }
}

fn decode(event: pb::AuthenticationEvent) -> Event {
    use pb::authentication_event::Kind;
    match event.kind {
        Some(Kind::Progress(message)) => Event::Progress(message),
        Some(Kind::Check(response)) => {
            Event::Check(check::decode_response(response).unwrap_or_else(Status::Failed))
        }
        Some(Kind::Elicitation(pending)) => {
            let request = pending
                .request
                .as_ref()
                .ok_or(Failure::MissingRequest)
                .and_then(|payload| {
                    payload
                        .decode::<serde_json::Value>()
                        .map_err(|error| Failure::InvalidPayload(error.to_string()))
                })
                .and_then(|raw| {
                    bas::authentication::elicitation::parse_request(&raw)
                        .map_err(|error| Failure::InvalidPayload(error.to_string()))
                });
            Event::Elicitation {
                request_id: pending.request_id,
                request,
            }
        }
        Some(Kind::ElicitationComplete(payload)) => {
            Event::ElicitationComplete(payload.decode::<serde_json::Value>().ok())
        }
        Some(Kind::ElicitationCancelled(request_id)) => Event::ElicitationCancelled(request_id),
        Some(Kind::Terminal(payload)) => Event::Terminal(
            payload
                .decode::<Invocation>()
                .map_err(|error| Failure::InvalidPayload(error.to_string())),
        ),
        None => Event::Unknown,
    }
}

fn next_event(
    next: Result<Option<pb::AuthenticationEvent>, tonic::Status>,
) -> Result<Event, Failure> {
    next.map_err(|error| Failure::StreamRpc(error.into()))?
        .map(decode)
        .ok_or(Failure::StreamEnded)
}

fn start_input(spec: &AgentSpec, method: &Method) -> Result<pb::AuthenticationInput, Failure> {
    Ok(pb::AuthenticationInput {
        kind: Some(pb::authentication_input::Kind::Start(
            pb::AuthenticateAgentRequest {
                agent: spec.id.to_string(),
                expected_spec_json: serde_json::to_vec(spec)
                    .map_err(|error| Failure::External(error.to_string()))?,
                method_id: method.id.clone(),
                expected_method_json: serde_json::to_vec(method)
                    .map_err(|error| Failure::External(error.to_string()))?,
            },
        )),
    })
}

pub(super) struct Stream {
    pub input: mpsc::UnboundedSender<Control>,
    pub events: futures_mpsc::UnboundedReceiver<Result<Event, Failure>>,
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
    mut input: mpsc::UnboundedReceiver<Control>,
    events: futures_mpsc::UnboundedSender<Result<Event, Failure>>,
) -> Result<(), Failure> {
    let mut client = check::verified_client(service.address(), &path)
        .await
        .map_err(Failure::External)?;
    let saved = bas::config::Store::at(path)
        .load()
        .map_err(|_| Failure::ConfigUnreadable)?;
    if !matches_registration(&saved.settings.daemon.agents, &spec) {
        return Err(Failure::RegistrationChanged);
    }
    let request = start_input(&spec, &method)?;
    let (send, receive) = mpsc::channel(16);
    send.send(request).await.map_err(|_| Failure::InputEnded)?;
    let mut stream = client
        .authenticate_agent_interactive(tokio_stream::wrappers::ReceiverStream::new(receive))
        .await
        .map_err(|error| Failure::StartRpc(error.into()))?
        .into_inner();
    loop {
        tokio::select! {
            next = input.recv() => {
                let Some(next) = next else { return Ok(()); };
                send.send(encode(next)).await.map_err(|_| Failure::InputClosed)?;
            }
            next = stream.message() => {
                let event = next_event(next)?;
                let complete = matches!(event, Event::Check(_));
                if events.unbounded_send(Ok(event)).is_err() { return Ok(()); }
                if complete { return Ok(()); }
            }
        }
    }
}

pub(super) fn matches_registration(
    registrations: &[bas::config::AgentRegistration],
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

pub(super) fn terminal(
    runtime: &Handle,
    plan: bas::authentication::Invocation,
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

#[cfg(all(test, feature = "gui-tests"))]
mod tests;
