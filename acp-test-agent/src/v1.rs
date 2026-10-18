//! ACP protocol v1 implementation of the scripted test agent.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use agent_client_protocol::schema::v1 as acp;
use agent_client_protocol::{Agent, Client, ConnectTo, ConnectionTo, Responder};

use crate::demo::sessions::Store;
use crate::demo::{self, Options};

mod client_ops;
mod prompt;
mod sessions;
mod updates;

/// Authentication method id advertised by the v1 agent.
pub(super) const AUTH_METHOD_ID: &str = "demo-login";

/// State shared by every v1 request handler of one connection.
pub(super) struct Demo {
    options: Options,
    sessions: Mutex<Store<acp::SessionUpdate>>,
    authenticated: AtomicBool,
    client_capabilities: Mutex<Option<acp::ClientCapabilities>>,
    next_message: AtomicU64,
    next_tool: AtomicU64,
    resources: demo::resources::Resources,
}

impl Demo {
    fn new(options: Options) -> Self {
        Self {
            sessions: Mutex::new(Store::new(options.page_size)),
            options,
            authenticated: AtomicBool::new(false),
            client_capabilities: Mutex::new(None),
            next_message: AtomicU64::new(0),
            next_tool: AtomicU64::new(0),
            resources: demo::resources::Resources::default(),
        }
    }

    pub(super) fn store(&self) -> MutexGuard<'_, Store<acp::SessionUpdate>> {
        self.sessions
            .lock()
            .expect("session store mutex is never poisoned")
    }

    pub(super) fn is_authenticated(&self) -> bool {
        self.authenticated.load(Ordering::Acquire)
    }

    pub(super) fn client_capabilities(&self) -> Option<acp::ClientCapabilities> {
        self.client_capabilities
            .lock()
            .expect("client capability mutex is never poisoned")
            .clone()
    }

    pub(super) fn next_message_id(&self, prefix: &str) -> acp::MessageId {
        let value = self.next_message.fetch_add(1, Ordering::AcqRel) + 1;
        acp::MessageId::new(format!("{prefix}-{value}"))
    }

    pub(super) fn next_tool_id(&self) -> acp::ToolCallId {
        let value = self.next_tool.fetch_add(1, Ordering::AcqRel) + 1;
        acp::ToolCallId::new(format!("tool-{value}"))
    }

    pub(super) fn require_authenticated(&self) -> Result<(), acp::Error> {
        if !self.options.require_auth || self.is_authenticated() {
            return Ok(());
        }
        Err(acp::Error::auth_required().data(format!(
            "authenticate with `{AUTH_METHOD_ID}` before creating a session"
        )))
    }

    fn initialize(
        &self,
        request: acp::InitializeRequest,
        responder: Responder<acp::InitializeResponse>,
    ) -> Result<(), acp::Error> {
        *self
            .client_capabilities
            .lock()
            .expect("client capability mutex is never poisoned") =
            Some(request.client_capabilities);

        let capabilities = acp::AgentCapabilities::new()
            .load_session(true)
            .prompt_capabilities(
                acp::PromptCapabilities::new()
                    .image(true)
                    .audio(true)
                    .embedded_context(true),
            )
            .session_capabilities(
                acp::SessionCapabilities::new()
                    .list(acp::SessionListCapabilities::new())
                    .delete(acp::SessionDeleteCapabilities::new())
                    .resume(acp::SessionResumeCapabilities::new())
                    .close(acp::SessionCloseCapabilities::new())
                    .additional_directories(acp::SessionAdditionalDirectoriesCapabilities::new())
                    .fork(acp::SessionForkCapabilities::new()),
            )
            .auth(acp::AgentAuthCapabilities::new().logout(acp::LogoutCapabilities::new()));
        responder.respond(
            acp::InitializeResponse::new(request.protocol_version)
                .agent_capabilities(capabilities)
                .auth_methods(vec![acp::AuthMethod::Agent(
                    acp::AuthMethodAgent::new(AUTH_METHOD_ID, "Demo sign in")
                        .description("Marks the connection authenticated for --require-auth"),
                )])
                .agent_info(
                    acp::Implementation::new(demo::AGENT_NAME, env!("CARGO_PKG_VERSION"))
                        .title(demo::AGENT_TITLE),
                ),
        )
    }

    fn authenticate(
        &self,
        request: acp::AuthenticateRequest,
        responder: Responder<acp::AuthenticateResponse>,
    ) -> Result<(), acp::Error> {
        if request.method_id.0.as_ref() != AUTH_METHOD_ID {
            return responder.respond_with_error(acp::Error::invalid_params().data(format!(
                "unknown auth method `{}`; try `{AUTH_METHOD_ID}`",
                request.method_id
            )));
        }
        self.authenticated.store(true, Ordering::Release);
        responder.respond(acp::AuthenticateResponse::new())
    }

    fn logout(&self, responder: Responder<acp::LogoutResponse>) -> Result<(), acp::Error> {
        self.authenticated.store(false, Ordering::Release);
        responder.respond(acp::LogoutResponse::new())
    }

    fn cancel(
        &self,
        notification: acp::CancelNotification,
        _connection: ConnectionTo<Client>,
    ) -> Result<(), acp::Error> {
        self.store().cancel(notification.session_id.0.as_ref());
        Ok(())
    }
}

/// Build the v1 protocol implementation.
pub fn component(options: Options) -> impl ConnectTo<Client> {
    let demo = Arc::new(Demo::new(options));

    Agent
        .builder()
        .name("acp-test-agent/v1")
        .on_receive_request(
            {
                let demo = demo.clone();
                async move |request: acp::InitializeRequest,
                            responder: Responder<acp::InitializeResponse>,
                            _connection: ConnectionTo<Client>| {
                    demo.initialize(request, responder)
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            {
                let demo = demo.clone();
                async move |request: acp::AuthenticateRequest,
                            responder: Responder<acp::AuthenticateResponse>,
                            _connection: ConnectionTo<Client>| {
                    demo.authenticate(request, responder)
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            {
                let demo = demo.clone();
                async move |_request: acp::LogoutRequest,
                            responder: Responder<acp::LogoutResponse>,
                            _connection: ConnectionTo<Client>| {
                    demo.logout(responder)
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            {
                let demo = demo.clone();
                async move |request: acp::NewSessionRequest,
                            responder: Responder<acp::NewSessionResponse>,
                            connection: ConnectionTo<Client>| {
                    sessions::new_session(&demo, &connection, request, responder)
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            {
                let demo = demo.clone();
                async move |request: acp::LoadSessionRequest,
                            responder: Responder<acp::LoadSessionResponse>,
                            connection: ConnectionTo<Client>| {
                    sessions::load_session(&demo, &connection, request, responder)
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            {
                let demo = demo.clone();
                async move |request: acp::ListSessionsRequest,
                            responder: Responder<acp::ListSessionsResponse>,
                            _connection: ConnectionTo<Client>| {
                    sessions::list_sessions(&demo, request, responder)
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            {
                let demo = demo.clone();
                async move |request: acp::DeleteSessionRequest,
                            responder: Responder<acp::DeleteSessionResponse>,
                            _connection: ConnectionTo<Client>| {
                    sessions::delete_session(&demo, request, responder)
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            {
                let demo = demo.clone();
                async move |request: acp::ResumeSessionRequest,
                            responder: Responder<acp::ResumeSessionResponse>,
                            connection: ConnectionTo<Client>| {
                    sessions::resume_session(&demo, &connection, request, responder)
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            {
                let demo = demo.clone();
                async move |request: acp::CloseSessionRequest,
                            responder: Responder<acp::CloseSessionResponse>,
                            _connection: ConnectionTo<Client>| {
                    sessions::close_session(&demo, request, responder)
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            {
                let demo = demo.clone();
                async move |request: acp::ForkSessionRequest,
                            responder: Responder<acp::ForkSessionResponse>,
                            connection: ConnectionTo<Client>| {
                    sessions::fork_session(&demo, &connection, request, responder)
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            {
                let demo = demo.clone();
                async move |request: acp::SetSessionModeRequest,
                            responder: Responder<acp::SetSessionModeResponse>,
                            connection: ConnectionTo<Client>| {
                    sessions::set_mode(&demo, &connection, request, responder)
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            {
                let demo = demo.clone();
                async move |request: acp::SetSessionConfigOptionRequest,
                            responder: Responder<acp::SetSessionConfigOptionResponse>,
                            connection: ConnectionTo<Client>| {
                    sessions::set_config_option(&demo, &connection, request, responder)
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            {
                let demo = demo.clone();
                async move |request: acp::PromptRequest,
                            responder: Responder<acp::PromptResponse>,
                            connection: ConnectionTo<Client>| {
                    prompt::handle_prompt(&demo, connection, request, responder)
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_notification(
            {
                let demo = demo.clone();
                async move |notification: acp::CancelNotification,
                            connection: ConnectionTo<Client>| {
                    demo.cancel(notification, connection)
                }
            },
            agent_client_protocol::on_receive_notification!(),
        )
}
