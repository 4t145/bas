//! ACP protocol v2 implementation of the scripted test agent.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use agent_client_protocol::schema::v2 as acp;
use agent_client_protocol::{Agent, Client, Error, Responder, V2ConnectionTo};

use crate::demo::sessions::Store;
use crate::demo::{self, Options};

mod client_ops;
mod prompt;
mod sessions;
mod updates;

/// Authentication method id advertised by the v2 agent.
pub(super) const AUTH_METHOD_ID: &str = "demo-login";

/// State shared by every v2 request handler of one connection.
pub(super) struct Demo {
    options: Options,
    sessions: Mutex<Store<acp::SessionUpdate>>,
    authenticated: AtomicBool,
    next_message: AtomicU64,
    next_tool: AtomicU64,
}

impl Demo {
    fn new(options: Options) -> Self {
        Self {
            sessions: Mutex::new(Store::new(options.page_size)),
            options,
            authenticated: AtomicBool::new(false),
            next_message: AtomicU64::new(0),
            next_tool: AtomicU64::new(0),
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

    pub(super) fn next_message_id(&self, prefix: &str) -> acp::MessageId {
        let value = self.next_message.fetch_add(1, Ordering::AcqRel) + 1;
        acp::MessageId::new(format!("{prefix}-{value}"))
    }

    pub(super) fn next_tool_id(&self) -> acp::ToolCallId {
        let value = self.next_tool.fetch_add(1, Ordering::AcqRel) + 1;
        acp::ToolCallId::new(format!("tool-{value}"))
    }

    pub(super) fn next_terminal_id(&self) -> acp::TerminalId {
        let value = self.next_tool.fetch_add(1, Ordering::AcqRel) + 1;
        acp::TerminalId::new(format!("terminal-{value}"))
    }

    pub(super) fn require_authenticated(&self) -> Result<(), Error> {
        if !self.options.require_auth || self.is_authenticated() {
            return Ok(());
        }
        Err(Error::auth_required().data(format!(
            "auth/login with `{AUTH_METHOD_ID}` before creating a session"
        )))
    }

    fn initialize(
        &self,
        request: acp::InitializeRequest,
        responder: Responder<acp::InitializeResponse>,
    ) -> Result<(), Error> {
        let capabilities = acp::AgentCapabilities::new().session(
            acp::SessionCapabilities::new()
                .prompt(
                    acp::PromptCapabilities::new()
                        .image(acp::PromptImageCapabilities::new())
                        .audio(acp::PromptAudioCapabilities::new())
                        .embedded_context(acp::PromptEmbeddedContextCapabilities::new()),
                )
                .delete(acp::SessionDeleteCapabilities::new())
                .additional_directories(acp::SessionAdditionalDirectoriesCapabilities::new())
                .fork(acp::SessionForkCapabilities::new()),
        );
        responder.respond(
            acp::InitializeResponse::new(
                request.protocol_version,
                acp::Implementation::new(demo::AGENT_NAME, env!("CARGO_PKG_VERSION"))
                    .title(demo::AGENT_TITLE),
            )
            .capabilities(capabilities)
            .auth_methods(vec![acp::AuthMethod::Agent(
                acp::AuthMethodAgent::new(AUTH_METHOD_ID, "Demo sign in")
                    .description("Marks the connection authenticated for --require-auth"),
            )]),
        )
    }

    fn login(
        &self,
        request: acp::LoginAuthRequest,
        responder: Responder<acp::LoginAuthResponse>,
    ) -> Result<(), Error> {
        if request.method_id.0.as_ref() != AUTH_METHOD_ID {
            return responder.respond_with_error(Error::invalid_params().data(format!(
                "unknown auth method `{}`; try `{AUTH_METHOD_ID}`",
                request.method_id
            )));
        }
        self.authenticated.store(true, Ordering::Release);
        responder.respond(acp::LoginAuthResponse::new())
    }

    fn logout(&self, responder: Responder<acp::LogoutAuthResponse>) -> Result<(), Error> {
        self.authenticated.store(false, Ordering::Release);
        responder.respond(acp::LogoutAuthResponse::new())
    }

    fn cancel(
        &self,
        notification: acp::CancelSessionNotification,
        _connection: V2ConnectionTo<Client>,
    ) -> Result<(), Error> {
        self.store().cancel(notification.session_id.0.as_ref());
        Ok(())
    }
}

/// Build the v2 protocol implementation.
pub fn component(options: Options) -> impl agent_client_protocol::ConnectTo<Client> {
    let demo = Arc::new(Demo::new(options));

    Agent
        .v2()
        .name("acp-test-agent/v2")
        .on_receive_request(
            {
                let demo = demo.clone();
                async move |request: acp::InitializeRequest,
                            responder: Responder<acp::InitializeResponse>,
                            _connection: V2ConnectionTo<Client>| {
                    demo.initialize(request, responder)
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            {
                let demo = demo.clone();
                async move |request: acp::LoginAuthRequest,
                            responder: Responder<acp::LoginAuthResponse>,
                            _connection: V2ConnectionTo<Client>| {
                    demo.login(request, responder)
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            {
                let demo = demo.clone();
                async move |_request: acp::LogoutAuthRequest,
                            responder: Responder<acp::LogoutAuthResponse>,
                            _connection: V2ConnectionTo<Client>| {
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
                            connection: V2ConnectionTo<Client>| {
                    sessions::new_session(&demo, &connection, request, responder)
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            {
                let demo = demo.clone();
                async move |request: acp::ListSessionsRequest,
                            responder: Responder<acp::ListSessionsResponse>,
                            _connection: V2ConnectionTo<Client>| {
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
                            _connection: V2ConnectionTo<Client>| {
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
                            connection: V2ConnectionTo<Client>| {
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
                            _connection: V2ConnectionTo<Client>| {
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
                            _connection: V2ConnectionTo<Client>| {
                    sessions::fork_session(&demo, request, responder)
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            {
                let demo = demo.clone();
                async move |request: acp::SetSessionConfigOptionRequest,
                            responder: Responder<acp::SetSessionConfigOptionResponse>,
                            connection: V2ConnectionTo<Client>| {
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
                            connection: V2ConnectionTo<Client>| {
                    prompt::handle_prompt(&demo, connection, request, responder)
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_notification(
            {
                let demo = demo.clone();
                async move |notification: acp::CancelSessionNotification,
                            connection: V2ConnectionTo<Client>| {
                    demo.cancel(notification, connection)
                }
            },
            agent_client_protocol::on_receive_notification!(),
        )
}
