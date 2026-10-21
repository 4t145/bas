//! Exercise the GUI install action against an actual, controllable gRPC stream.
//! The fixture never downloads or executes a registry distribution.

use std::{
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use bas::service::{Target, pb, transport::Listener};
use futures::{Stream, channel::mpsc};
use gpui_kit::{AnyWindowHandle, Entity, TestAppContext, px, size, test::TestWindowExt};
use tokio_util::task::AbortOnDropHandle;
use tonic::{Request, Response, Status as RpcStatus, transport::Server};

use super::*;

type ProgressStream = Pin<Box<dyn Stream<Item = Result<pb::AgentOperation, RpcStatus>> + Send>>;

struct FixtureService {
    store: bas::config::Store,
    catalog: Arc<Mutex<Catalog>>,
    stream: Mutex<Option<mpsc::UnboundedReceiver<Result<pb::AgentOperation, RpcStatus>>>>,
    started: Arc<AtomicBool>,
    probes: Arc<Mutex<Probes>>,
}

#[derive(Default)]
struct Probes {
    configuration_error: Option<RpcStatus>,
    catalog_error: Option<RpcStatus>,
    authentication_error: Option<RpcStatus>,
    response: Option<pb::CheckAgentResponse>,
    checks: Vec<pb::CheckAgentRequest>,
    logins: Vec<pb::AuthenticateAgentRequest>,
    logouts: Vec<pb::LogoutAgentRequest>,
    removals: Vec<String>,
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
            async fn get_agent_configuration(
                &self, _: Request<pb::GetAgentConfigurationRequest>,
            ) -> Result<Response<pb::AgentConfiguration>, RpcStatus> {
                if let Some(error) = self.probes.lock().expect("probes").configuration_error.clone() {
                    return Err(error);
                }
                let document = self.store.load().expect("fixture settings");
                let registrations_json = serde_json::to_vec(&document.settings.daemon.agents).expect("agents");
                Ok(Response::new(pb::AgentConfiguration {
                    profile: self.store.path().display().to_string(),
                    revision: String::from_utf8(registrations_json.clone()).expect("JSON"),
                    registrations_json,
                }))
            }

            async fn save_agent_configuration(
                &self, request: Request<pb::SaveAgentConfigurationRequest>,
            ) -> Result<Response<pb::AgentConfiguration>, RpcStatus> {
                let request = request.into_inner();
                let baseline = self.get_agent_configuration(Request::new(pb::GetAgentConfigurationRequest {})).await?.into_inner();
                if request.profile != baseline.profile || request.revision != baseline.revision {
                    return Err(RpcStatus::aborted("configuration changed"));
                }
                let mut document = self.store.load().expect("fixture document");
                let agents = serde_json::from_slice(&request.registrations_json).expect("registrations");
                self.store.save_agents(&mut document, agents).expect("fixture save");
                self.get_agent_configuration(Request::new(pb::GetAgentConfigurationRequest {})).await
            }

            async fn remove_agent(
                &self, request: Request<pb::RemoveAgentRequest>,
            ) -> Result<Response<pb::RemoveAgentResponse>, RpcStatus> {
                let id = request.into_inner().id;
                let mut document = self.store.load().expect("fixture document");
                if !document.settings.daemon.agents.iter()
                    .any(|agent| agent.id().as_str() == id && agent.managed().is_some()) {
                    return Err(RpcStatus::failed_precondition("not a managed registration"));
                }
                let mut agents = document.settings.daemon.agents.clone();
                agents.retain(|agent| agent.id().as_str() != id);
                if agents.is_empty() {
                    agents.push(bas::config::AgentSpec::embedded().into());
                }
                self.store.save_agents(&mut document, agents).expect("fixture remove");
                self.probes.lock().expect("probes").removals.push(id);
                Ok(Response::new(pb::RemoveAgentResponse {}))
            }

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
                if let Some(error) = self.probes.lock().expect("probes").catalog_error.clone() {
                    return Err(error);
                }
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
    load_history_window(pb::LoadHistoryWindowRequest) -> pb::LoadHistoryWindowResponse;
}

struct Fixture {
    runtime: tokio::runtime::Runtime,
    address: Target,
    progress: Option<mpsc::UnboundedSender<Result<pb::AgentOperation, RpcStatus>>>,
    started: Arc<AtomicBool>,
    catalog: Arc<Mutex<Catalog>>,
    probes: Arc<Mutex<Probes>>,
    store: bas::config::Store,
    _server_task: AbortOnDropHandle<()>,
    _endpoint_directory: tempfile::TempDir,
}

impl Fixture {
    fn new(path: std::path::PathBuf) -> Self {
        Self::with_transport(path, false)
    }

    fn with_transport(path: std::path::PathBuf, tcp: bool) -> Self {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("fixture runtime");
        let endpoint_directory = tempfile::tempdir().expect("endpoint directory");
        #[cfg(unix)]
        let address = {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                endpoint_directory.path(),
                std::fs::Permissions::from_mode(0o700),
            )
            .expect("private socket directory");
            Target::Unix(
                endpoint_directory
                    .path()
                    .canonicalize()
                    .expect("canonical endpoint directory")
                    .join("service.sock"),
            )
        };
        #[cfg(windows)]
        let address = Target::Pipe(format!(
            "bas-gui-{}-{}",
            std::process::id(),
            endpoint_directory
                .path()
                .file_name()
                .expect("unique directory")
                .to_string_lossy()
        ));
        let address = if tcp {
            "tcp://127.0.0.1:0".parse().expect("TCP fixture")
        } else {
            address
        };
        let listener = runtime
            .block_on(Listener::bind(&address))
            .expect("local fixture listener");
        let address = listener.target().clone();
        let (progress, stream) = mpsc::unbounded();
        let catalog = Arc::new(Mutex::new(catalog(path.clone())));
        let started = Arc::new(AtomicBool::new(false));
        let probes = Arc::new(Mutex::new(Probes::default()));
        let service = FixtureService {
            store: bas::config::Store::at(path.clone()),
            catalog: catalog.clone(),
            stream: Mutex::new(Some(stream)),
            started: started.clone(),
            probes: probes.clone(),
        };
        let server_task = AbortOnDropHandle::new(runtime.spawn(async move {
            Server::builder()
                .add_service(pb::bas_server::BasServer::new(service))
                .serve_with_incoming(listener.incoming())
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
            _endpoint_directory: endpoint_directory,
        }
    }

    fn open(&self, cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Agents>) {
        cx.update(super::super::super::tests::init);
        cx.update(|cx| {
            crate::agents::init(
                self.address.clone().into(),
                self.runtime.handle().clone(),
                cx,
            );
        });
        let mut page = None;
        let window = cx.open_window(size(px(960.), px(720.)), |window, cx| {
            let view = cx.new(|cx| Agents::new(self.store.clone(), window, cx));
            view.update(cx, |page, cx| {
                let agents = self
                    .store
                    .load()
                    .expect("fixture settings")
                    .settings
                    .daemon
                    .agents;
                let registrations_json = serde_json::to_vec(&agents).expect("registrations");
                page.accept_configuration(pb::AgentConfiguration {
                    profile: self.store.path().display().to_string(),
                    revision: String::from_utf8(registrations_json.clone()).expect("JSON"),
                    registrations_json,
                })
                .expect("configuration");
                page.configuration_task = None;
                page.registry.load = Load::Idle;
                page.apply_catalog(self.catalog.lock().expect("catalog").clone(), cx);
            });
            page = Some(view.clone());
            Root::new(view, window, cx)
        });
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
fn fetched_registrations_survive_remote_catalog_failure(cx: &mut TestAppContext) {
    use bas::config::{AgentSpec, agent::Command};
    let directory = tempfile::tempdir().expect("settings");
    let fixture = Fixture::with_transport(directory.path().join("server.json"), true);
    let custom = AgentSpec::acp("custom", Command::new("/server/custom", []));
    let managed_spec = AgentSpec::acp("managed", Command::new("/server/managed", []));
    let managed = super::super::super::tests::managed_fixture(directory.path(), &managed_spec);
    // The registration remains readable, but neither the server catalog nor the client
    // can resolve this descriptor. Configuration recovery must not require it.
    std::fs::remove_file(&managed.managed().expect("managed").installation.path)
        .expect("missing descriptor");
    let mut document = fixture.store.load().expect("settings");
    fixture
        .store
        .save_agents(&mut document, vec![custom.clone().into(), managed.clone()])
        .expect("registrations");
    fixture.probes.lock().expect("probes").catalog_error =
        Some(RpcStatus::unavailable("managed descriptor unavailable"));
    cx.update(super::super::super::tests::init);
    cx.update(|cx| {
        crate::agents::init(
            fixture.address.clone().into(),
            fixture.runtime.handle().clone(),
            cx,
        )
    });
    let mut page = None;
    let window = cx.open_window(size(px(960.), px(720.)), |window, cx| {
        // This store is deliberately unrelated to the daemon's settings.
        let view = cx.new(|cx| {
            Agents::new(
                bas::config::Store::at(directory.path().join("client.json")),
                window,
                cx,
            )
        });
        page = Some(view.clone());
        Root::new(view, window, cx)
    });
    let page = page.expect("page");
    fixture.pump_until(cx, |cx| {
        let page = page.read(cx);
        page.configuration.is_some()
            && !page.registry.loading()
            && page.message.contains("managed descriptor unavailable")
    });
    cx.update_window(window.into(), |_, _, cx| {
        page.update(cx, |page, cx| {
            assert!(page.registry.editable());
            assert!(page.registry.catalog.is_none());
            let entries = page.configuration_entries();
            assert_eq!(entries.len(), 2);
            assert_eq!(entries[0].id, "managed");
            assert_eq!(entries[0].status, Status::Missing);
            assert!(entries[0].spec.is_none());
            assert_eq!(entries[1].spec.as_ref(), Some(&custom));

            let mut cached = catalog("/server/settings.json".into());
            cached.can_install = false;
            cached.entries = vec![entries[0].clone()];
            cached.entries[0].name = "Managed display name".into();
            page.apply_catalog(cached, cx);
            assert!(
                page.registry.editable(),
                "editing does not require install capability"
            );
            page.refresh_registry(false, cx);
        });
    })
    .expect("fallback cards");
    fixture.pump_until(cx, |cx| !page.read(cx).registry.loading());
    cx.update_window(window.into(), |_, window, cx| {
        page.update(cx, |page, cx| {
            assert_eq!(page.configuration_entries()[0].name, "Managed display name");
            page.edit(custom.clone(), Some(custom.id.clone()), window, cx);
            page.editor.mark_delete();
            page.save(cx);
        });
    })
    .expect("remove custom configuration despite catalog outage");
    fixture.pump_until(cx, |cx| page.read(cx).configuration_task.is_none());
    assert_eq!(
        fixture.store.load().expect("saved").settings.daemon.agents,
        vec![managed]
    );
    cx.update(|cx| page.update(cx, |page, cx| page.remove_managed("managed".into(), cx)));
    fixture.pump_until(cx, |_| {
        fixture.probes.lock().expect("probes").removals == ["managed"]
    });
    assert_eq!(
        fixture
            .store
            .load()
            .expect("removed")
            .settings
            .daemon
            .agents,
        vec![AgentSpec::embedded().into()]
    );
    assert!(!directory.path().join("client.json").exists());
}

#[gpui_kit::test]
fn old_daemon_configuration_error_has_no_local_write_fallback(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().expect("settings");
    let fixture = Fixture::with_transport(directory.path().join("server.json"), true);
    let (_, page) = fixture.open(cx);
    fixture.probes.lock().expect("probes").configuration_error =
        Some(RpcStatus::unimplemented("older service"));
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.configuration = None;
            page.document = None;
            page.fetch_configuration(cx);
        })
    });
    fixture.pump_until(cx, |cx| page.read(cx).configuration_task.is_none());
    cx.update(|cx| {
        assert!(page.read(cx).document.is_none());
        assert_eq!(
            page.read(cx).message,
            crate::i18n::tr!("agent-config-upgrade")
        );
    });
    assert!(
        !fixture.store.path().exists(),
        "no client-local fallback file"
    );
}

#[gpui_kit::test]
fn remote_configuration_save_stays_with_its_backend_after_switching(cx: &mut TestAppContext) {
    use bas::config::{AgentSpec, agent::Command};
    let directory = tempfile::tempdir().expect("settings");
    let fixture = Fixture::with_transport(directory.path().join("server.json"), true);
    let (window, page) = fixture.open(cx);
    cx.update_window(window, |_, window, cx| {
        page.update(cx, |page, cx| {
            page.edit(
                AgentSpec::acp("remote-custom", Command::new("/server/agent", [])),
                None,
                window,
                cx,
            );
        });
        crate::agents::init(
            "tcp://192.0.2.1:4554".parse().expect("other backend"),
            fixture.runtime.handle().clone(),
            cx,
        );
        page.update(cx, |page, cx| page.save(cx));
    })
    .expect("save through remote editor");
    fixture.pump_until(cx, |cx| page.read(cx).configuration_task.is_none());
    assert!(
        fixture
            .store
            .load()
            .expect("daemon-side persisted settings")
            .settings
            .daemon
            .agents
            .iter()
            .any(|agent| agent.id().as_str() == "remote-custom")
    );
    cx.update(|cx| {
        assert_eq!(
            page.read(cx).target().expect("owned target").target,
            fixture.address
        );
        assert!(page.read(cx).message.is_empty());
    });
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
        // Management can switch before this backend's panel is even created.
        crate::agents::init(
            "192.0.2.1:4554".parse().expect("other target"),
            fixture.runtime.handle().clone(),
            cx,
        );
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
