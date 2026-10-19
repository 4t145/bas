//! Exercise the GUI install action against an actual, controllable gRPC stream.
//! The fixture never downloads or executes a registry distribution.

use std::{
    net::SocketAddr,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use crate::agents::check;
use bas::service::pb;
use futures::{Stream, channel::mpsc};
use gpui_kit::{AnyWindowHandle, Entity, TestAppContext, px, size, test::TestWindowExt};
use tokio_stream::wrappers::TcpListenerStream;
use tokio_util::task::AbortOnDropHandle;
use tonic::{Request, Response, Status as RpcStatus, transport::Server};

use super::*;

type ProgressStream = Pin<Box<dyn Stream<Item = Result<pb::AgentOperation, RpcStatus>> + Send>>;

struct FixtureService {
    catalog: Arc<Mutex<Catalog>>,
    stream: Mutex<Option<mpsc::UnboundedReceiver<Result<pb::AgentOperation, RpcStatus>>>>,
    started: Arc<AtomicBool>,
    probes: Arc<Mutex<Probes>>,
}

#[derive(Default)]
struct Probes {
    authentication_error: Option<RpcStatus>,
    response: Option<pb::CheckAgentResponse>,
    checks: Vec<pb::CheckAgentRequest>,
    logins: Vec<pb::AuthenticateAgentRequest>,
    logouts: Vec<pb::LogoutAgentRequest>,
    logout_error: Option<RpcStatus>,
    answers: Vec<pb::ElicitationAnswer>,
    unrelated: Vec<&'static str>,
}

// Keeping the unrelated RPCs explicit makes this fixture fail compilation when
// the daemon interface changes, without introducing session behavior here.
macro_rules! fixture_service {
    ($($method:ident($request:ty) -> $response:ty;)*) => {
        #[tonic::async_trait]
        impl pb::bas_server::Bas for FixtureService {
            type InstallAgentStream = ProgressStream;
            type AuthenticateAgentInteractiveStream = Pin<Box<dyn Stream<Item = Result<pb::AuthenticationEvent, RpcStatus>> + Send>>;
            type WatchElicitationsStream = Pin<Box<dyn Stream<Item = Result<pb::ElicitationSnapshot, RpcStatus>> + Send>>;
            type AttachStream = Pin<Box<dyn Stream<Item = Result<pb::Event, RpcStatus>> + Send>>;
            type SetForegroundStream = Pin<Box<dyn Stream<Item = Result<pb::ForegroundResponse, RpcStatus>> + Send>>;
            type WatchPermissionsStream = Pin<Box<dyn Stream<Item = Result<pb::PermissionSnapshot, RpcStatus>> + Send>>;

            async fn install_agent(
                &self,
                request: Request<pb::InstallAgentRequest>,
            ) -> Result<Response<Self::InstallAgentStream>, RpcStatus> {
                assert_eq!(request.into_inner().id, "codex");
                let stream = self.stream.lock().expect("fixture stream").take()
                    .ok_or_else(|| RpcStatus::already_exists("installation already started"))?;
                self.started.store(true, Ordering::SeqCst);
                Ok(Response::new(Box::pin(stream)))
            }

            async fn list_agent_catalog(
                &self,
                _: Request<pb::ListAgentCatalogRequest>,
            ) -> Result<Response<pb::ListAgentCatalogResponse>, RpcStatus> {
                let catalog = self.catalog.lock().expect("fixture catalog");
                Ok(Response::new(pb::ListAgentCatalogResponse {
                    catalog_json: serde_json::to_vec(&*catalog).expect("serialize catalog"),
                }))
            }

            async fn check_agent(
                &self,
                request: Request<pb::CheckAgentRequest>,
            ) -> Result<Response<pb::CheckAgentResponse>, RpcStatus> {
                let mut probes = self.probes.lock().expect("fixture probes");
                probes.checks.push(request.into_inner());
                probes.response.clone().map(Response::new)
                    .ok_or_else(|| RpcStatus::unimplemented("unrelated fixture check"))
            }

            async fn authenticate_agent_interactive(
                &self,
                request: Request<tonic::Streaming<pb::AuthenticationInput>>,
            ) -> Result<Response<Self::AuthenticateAgentInteractiveStream>, RpcStatus> {
                if let Some(error) = self.probes.lock().expect("fixture probes").authentication_error.clone() {
                    return Err(error);
                }
                let mut input = request.into_inner();
                let first = input.message().await?.expect("start message");
                let Some(pb::authentication_input::Kind::Start(start)) = first.kind else { panic!("start expected") };
                let method: bas::authentication::Method = serde_json::from_slice(&start.expected_method_json).expect("normalized method");
                assert_eq!(method.id, start.method_id);
                let form = start.method_id == "form-login";
                let terminal = start.method_id == "terminal-login";
                self.probes.lock().expect("probes").logins.push(start);
                if terminal {
                    assert!(matches!(method.execution, bas::authentication::Execution::Terminal(_)));
                    use futures::StreamExt;
                    let stream = tokio_stream::iter([Ok(pb::AuthenticationEvent {
                        kind: Some(pb::authentication_event::Kind::Progress("正在验证认证终端…".into())),
                    })]).chain(futures::stream::pending());
                    return Ok(Response::new(Box::pin(stream)));
                }
                if form {
                    let (send, receive) = tokio::sync::mpsc::channel(4);
                    let probes = self.probes.clone();
                    tokio::spawn(async move {
                        let request = serde_json::json!({"requestId":"auth-request","mode":"form","message":"Configure provider","requestedSchema":{"type":"object","properties":{"account":{"type":"string","minLength":2},"enabled":{"type":"boolean","default":true}},"required":["account","enabled"]}});
                        send.send(Ok(pb::AuthenticationEvent { kind: Some(pb::authentication_event::Kind::Elicitation(pb::PendingElicitation {
                            request_id: "form-1".into(), request: Some(pb::AcpPayload { json: serde_json::to_vec(&request).expect("JSON") }), answered:false
                        })) })).await.expect("form event");
                        let answer = input.message().await.expect("receive reply").expect("reply");
                        let Some(pb::authentication_input::Kind::ElicitationResponse(answer)) = answer.kind else { panic!("form answer") };
                        probes.lock().expect("probes").answers.push(answer);
                        send.send(Ok(pb::AuthenticationEvent { kind: Some(pb::authentication_event::Kind::Check(pb::CheckAgentResponse {
                            status: pb::AgentCheckStatus::Connected as i32, ..Default::default()
                        })) })).await.expect("completion");
                    });
                    return Ok(Response::new(Box::pin(tokio_stream::wrappers::ReceiverStream::new(receive))));
                }
                Ok(Response::new(Box::pin(tokio_stream::iter([Ok(pb::AuthenticationEvent {
                    kind: Some(pb::authentication_event::Kind::Check(pb::CheckAgentResponse {
                        status: pb::AgentCheckStatus::Connected as i32,
                        resolved_program: "/fixture/codex-acp".into(),
                        ..Default::default()
                    }))
                })]))))
            }

            async fn authenticate_agent(
                &self,
                request: Request<pb::AuthenticateAgentRequest>,
            ) -> Result<Response<pb::AuthenticateAgentResponse>, RpcStatus> {
                self.probes.lock().expect("fixture probes").logins.push(request.into_inner());
                Ok(Response::new(pb::AuthenticateAgentResponse {
                    check: Some(pb::CheckAgentResponse {
                        status: pb::AgentCheckStatus::Connected as i32,
                        resolved_program: "/fixture/codex-acp".into(),
                        ..Default::default()
                    }),
                }))
            }

            async fn logout_agent(
                &self, request: Request<pb::LogoutAgentRequest>,
            ) -> Result<Response<pb::LogoutAgentResponse>, RpcStatus> {
                let mut probes = self.probes.lock().expect("fixture probes");
                probes.logouts.push(request.into_inner());
                if let Some(error) = &probes.logout_error { return Err(error.clone()); }
                Ok(Response::new(pb::LogoutAgentResponse {}))
            }

            $(async fn $method(
                &self,
                _: Request<$request>,
            ) -> Result<Response<$response>, RpcStatus> {
                self.probes.lock().expect("fixture probes").unrelated.push(stringify!($method));
                Err(RpcStatus::unimplemented("unrelated fixture RPC"))
            })*
        }
    };
}

fixture_service! {
    list_agent_sessions(pb::ListAgentSessionsRequest) -> pb::ListAgentSessionsResponse;
    import_agent_session(pb::ImportAgentSessionRequest) -> pb::CreateSessionResponse;
    delete_agent_session(pb::DeleteAgentSessionRequest) -> pb::DeleteSessionResponse;
    close_session(pb::GetSessionRequest) -> pb::GetSessionResponse;
    restart_session(pb::GetSessionRequest) -> pb::GetSessionResponse;
    set_session_directories(pb::SetSessionDirectoriesRequest) -> pb::GetSessionResponse;
    list_agents(pb::ListAgentsRequest) -> pb::ListAgentsResponse;
    remove_agent(pb::RemoveAgentRequest) -> pb::RemoveAgentResponse;
    create_session(pb::CreateSessionRequest) -> pb::CreateSessionResponse;
    list_sessions(pb::ListSessionsRequest) -> pb::ListSessionsResponse;
    get_session(pb::GetSessionRequest) -> pb::GetSessionResponse;
    set_archived(pb::SetArchivedRequest) -> pb::SetArchivedResponse;
    delete_session(pb::DeleteSessionRequest) -> pb::DeleteSessionResponse;
    attach(pb::AttachRequest) -> Self::AttachStream;
    set_foreground(tonic::Streaming<pb::ForegroundRequest>) -> Self::SetForegroundStream;
    prompt(pb::PromptSubmission) -> pb::PromptSubmissionResponse;
    get_prompt_acceptance(pb::PromptAcceptanceRequest) -> pb::PromptAcceptanceResponse;
    cancel(pb::AcpPayload) -> pb::CancelResponse;
    set_config_option(pb::AcpPayload) -> pb::AcpPayload;
    set_mode(pb::AcpPayload) -> pb::AcpPayload;
    watch_permissions(pb::WatchPermissionsRequest) -> Self::WatchPermissionsStream;
    watch_elicitations(pb::WatchElicitationsRequest) -> Self::WatchElicitationsStream;
    respond_elicitation(pb::RespondElicitationRequest) -> pb::RespondElicitationResponse;
    respond_permission(pb::RespondPermissionRequest) -> pb::RespondPermissionResponse;
    set_title(pb::SetTitleRequest) -> pb::SetTitleResponse;
    load_history(pb::LoadHistoryRequest) -> pb::LoadHistoryResponse;
}

struct Fixture {
    runtime: tokio::runtime::Runtime,
    address: SocketAddr,
    progress: Option<mpsc::UnboundedSender<Result<pb::AgentOperation, RpcStatus>>>,
    started: Arc<AtomicBool>,
    catalog: Arc<Mutex<Catalog>>,
    probes: Arc<Mutex<Probes>>,
    store: bas::config::Store,
    _server_task: AbortOnDropHandle<()>,
}

impl Fixture {
    fn new(path: std::path::PathBuf) -> Self {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("fixture runtime");
        let listener = runtime
            .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
            .expect("local fixture listener");
        let address = listener.local_addr().expect("fixture address");
        let (progress, stream) = mpsc::unbounded();
        let catalog = Arc::new(Mutex::new(catalog(path.clone())));
        let started = Arc::new(AtomicBool::new(false));
        let probes = Arc::new(Mutex::new(Probes::default()));
        let service = FixtureService {
            catalog: catalog.clone(),
            stream: Mutex::new(Some(stream)),
            started: started.clone(),
            probes: probes.clone(),
        };
        let server_task = AbortOnDropHandle::new(runtime.spawn(async move {
            Server::builder()
                .add_service(pb::bas_server::BasServer::new(service))
                .serve_with_incoming(TcpListenerStream::new(listener))
                .await
                .expect("fixture server");
        }));
        Self {
            runtime,
            address,
            progress: Some(progress),
            started,
            catalog,
            probes,
            store: bas::config::Store::at(path),
            _server_task: server_task,
        }
    }

    fn open(&self, cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Agents>) {
        cx.update(super::super::super::tests::init);
        let mut page = None;
        let window = cx.open_window(size(px(960.), px(720.)), |window, cx| {
            let view = cx.new(|cx| Agents::new(self.store.clone(), window, cx));
            view.update(cx, |page, cx| {
                page.apply_catalog(self.catalog.lock().expect("catalog").clone(), true, cx);
            });
            page = Some(view.clone());
            Root::new(view, window, cx)
        });
        // Initialize after opening the page, so the only first network operation
        // is the install action exercised below.
        cx.update(|cx| cx.set_global(Service::new(self.address, self.runtime.handle().clone())));
        let page = page.expect("fixture page");
        // Installed agents are managed in Settings, not the add window.
        if self.catalog.lock().expect("catalog").entries[0]
            .spec
            .is_some()
        {
            return (window.into(), page);
        }
        let installation = cx.update(|cx| {
            super::super::super::window::open(page.clone(), cx).expect("native installation window")
        });
        let _configuration = window;
        (installation.into(), page)
    }

    fn begin(&self, cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Agents>) {
        let (window, page) = self.open(cx);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find("agents-install-codex").visible());
            window.click("agents-install-codex", cx);
            assert!(page.read(cx).operations["codex"].busy);
        })
        .expect("click registry install");
        // Nothing can emit progress until after the click callback returns.
        assert!(!self.started.load(Ordering::SeqCst));
        self.pump_until(cx, |_| self.started.load(Ordering::SeqCst));
        (window, page)
    }

    fn pump_until(&self, cx: &mut TestAppContext, predicate: impl Fn(&App) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            self.runtime.block_on(async {
                tokio::time::sleep(Duration::from_millis(1)).await;
            });
            cx.run_until_parked();
            if cx.update(|cx| predicate(cx)) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "gRPC fixture condition timed out"
            );
        }
    }

    fn send(&self, update: Result<pb::AgentOperation, RpcStatus>) {
        self.progress
            .as_ref()
            .expect("open progress stream")
            .unbounded_send(update)
            .expect("GUI retains its gRPC stream");
    }
}

#[gpui_kit::test]
fn grpc_transport_failure_ends_the_gui_operation(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().expect("settings directory");
    let fixture = Fixture::new(directory.path().join("settings.json"));
    let (window, page) = fixture.begin(cx);
    fixture.send(Err(RpcStatus::unavailable(
        "fixture connection interrupted",
    )));
    fixture.pump_until(cx, |cx| !page.read(cx).operations["codex"].busy);
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            page.read(cx).operations["codex"]
                .message
                .contains("fixture connection interrupted")
        );
        assert!(window.try_find("agents-install-cancel-codex").is_none());
        assert!(window.try_find("agents-install-codex").is_some());
    })
    .expect("transport failure enables retry");
}

#[gpui_kit::test]
fn authentication_form_round_trips_over_the_owned_interactive_stream(cx: &mut TestAppContext) {
    use bas::config::{AgentSpec, agent::Command};
    let directory = tempfile::tempdir().expect("settings");
    let fixture = Fixture::new(directory.path().join("settings.json"));
    let spec = AgentSpec::acp("codex", Command::new("/fixture/agent", []));
    let mut document = fixture.store.load().expect("document");
    fixture
        .store
        .save_agents(&mut document, vec![spec.clone().into()])
        .expect("configured fixture");
    {
        let mut catalog = fixture.catalog.lock().expect("catalog");
        catalog.entries[0].spec = Some(spec.clone());
        catalog.entries[0].status = Status::Installed;
    }
    let (window, page) = fixture.open(cx);
    cx.update(|cx| {
        check::init(fixture.address, fixture.runtime.handle().clone(), cx);
        let raw = serde_json::json!({"protocolVersion":1,"authMethods":[{"id":"form-login","name":"Configure provider"}]});
        let methods = bas::authentication::methods(&raw, None).expect("method");
        page.update(cx, |page, cx| {
            page.checks.insert(spec.id.clone(), super::super::super::Check {
                spec:spec.clone(), status:crate::agents::Status::AuthenticationRequired { program:"/fixture/agent".into(), methods }, completed_at:None, _task:None
            });
            cx.notify();
        });
    });
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("authentication-method-0").label(),
            Some("Configure provider")
        );
        window.click("authentication-method-0", cx);
    })
    .expect("select authentication method");
    fixture.pump_until(cx, |cx| {
        page.read(cx).authentications[&spec.id]
            .read(cx)
            .has_pending_forms()
    });
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.click("elicitation-submit", cx);
        assert!(fixture.probes.lock().expect("probes").answers.is_empty());
        window.click("elicitation-input-account", cx);
        window.input("Ada", cx);
        window.render_frame(cx);
        window.click("elicitation-submit", cx);
    })
    .expect("submit typed form");
    fixture.pump_until(cx, |cx| {
        page.read(cx)
            .status(&spec)
            .is_some_and(crate::agents::Status::connected)
    });
    let probes = fixture.probes.lock().expect("probes");
    assert_eq!(probes.answers.len(), 1);
    assert_eq!(probes.answers[0].request_id, "form-1");
    assert_eq!(
        probes.answers[0]
            .response
            .as_ref()
            .expect("payload")
            .decode::<serde_json::Value>()
            .expect("response"),
        serde_json::json!({"action":"accept","content":{"account":"Ada","enabled":true}})
    );
    assert!(probes.unrelated.is_empty());
}
