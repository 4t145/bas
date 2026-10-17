//! v2 session lifecycle request handlers.

use agent_client_protocol::schema::v2 as acp;
use agent_client_protocol::{Client, Error, Responder, V2ConnectionTo};

use super::Demo;
use super::updates::{self, config_options};
use crate::demo::sessions::Error as SessionError;

fn invalid(error: SessionError) -> Error {
    Error::invalid_params().data(error.to_string())
}

/// Handle `session/new` and announce the ready state.
pub(super) fn new_session(
    demo: &Demo,
    connection: &V2ConnectionTo<Client>,
    request: acp::NewSessionRequest,
    responder: Responder<acp::NewSessionResponse>,
) -> Result<(), Error> {
    if let Err(error) = demo.require_authenticated() {
        return responder.respond_with_error(error);
    }
    let session_id = demo.store().create(
        request.cwd.into_inner(),
        request
            .additional_directories
            .into_iter()
            .map(acp::AbsolutePath::into_inner)
            .collect(),
        request.mcp_servers.len(),
    );
    let (config, commands) = {
        let store = demo.store();
        let session = store.get(&session_id).map_err(invalid)?;
        (
            config_options(&session.config),
            updates::available_commands(),
        )
    };
    responder.respond(
        acp::NewSessionResponse::new(session_id.clone())
            .config_options(config)
            .available_commands(commands),
    )?;
    // v2 signals readiness separately from the prompt response. A client can
    // wait for this idle update before it starts foreground work.
    demo.send_idle(
        connection,
        &acp::SessionId::new(session_id),
        acp::StopReason::EndTurn,
    )
}

/// Handle `session/list` with the configured page size.
pub(super) fn list_sessions(
    demo: &Demo,
    request: acp::ListSessionsRequest,
    responder: Responder<acp::ListSessionsResponse>,
) -> Result<(), Error> {
    let cwd = request.cwd.map(acp::AbsolutePath::into_inner);
    let cursor = request.cursor.as_ref().map(|cursor| cursor.0.to_string());
    let store = demo.store();
    let page = store
        .page(cwd.as_deref(), cursor.as_deref())
        .map_err(invalid)?;
    responder.respond(
        acp::ListSessionsResponse::new(
            page.sessions
                .iter()
                .map(|session| updates::session_info(session))
                .collect(),
        )
        .next_cursor(page.next_cursor.map(acp::SessionListCursor::new)),
    )
}

/// Handle `session/delete`.
pub(super) fn delete_session(
    demo: &Demo,
    request: acp::DeleteSessionRequest,
    responder: Responder<acp::DeleteSessionResponse>,
) -> Result<(), Error> {
    if let Err(error) = demo.store().delete(request.session_id.0.as_ref()) {
        return responder.respond_with_error(invalid(error));
    }
    responder.respond(acp::DeleteSessionResponse::new())
}

/// Handle `session/resume` and replay history when requested.
pub(super) fn resume_session(
    demo: &Demo,
    connection: &V2ConnectionTo<Client>,
    request: acp::ResumeSessionRequest,
    responder: Responder<acp::ResumeSessionResponse>,
) -> Result<(), Error> {
    let session_id = request.session_id.0.to_string();
    let cwd = request.cwd.into_inner();
    let additional_directories = request
        .additional_directories
        .into_iter()
        .map(acp::AbsolutePath::into_inner)
        .collect();
    let replay_history = match request.replay_from {
        None => false,
        Some(acp::ReplayFrom::Start(_)) => true,
        Some(other) => {
            return responder.respond_with_error(
                Error::invalid_params().data(format!("unsupported replay cursor {other:?}")),
            );
        }
    };
    let history = {
        let mut store = demo.store();
        if let Err(error) = store.resume(
            &session_id,
            &cwd,
            additional_directories,
            request.mcp_servers.len(),
        ) {
            return responder.respond_with_error(invalid(error));
        }
        if replay_history {
            store
                .get(&session_id)
                .map(|session| {
                    // Foreground state is live, not retained history: a
                    // resumed session starts idle regardless of the last
                    // state recorded before it was closed.
                    session
                        .history
                        .iter()
                        .filter(|update| !matches!(update, acp::SessionUpdate::StateUpdate(_)))
                        .cloned()
                        .collect()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        }
    };
    for update in history {
        connection.send_notification(acp::UpdateSessionNotification::new(
            session_id.clone(),
            update,
        ))?;
    }
    let (config, commands) = {
        let store = demo.store();
        let session = store.get(&session_id).map_err(invalid)?;
        (
            config_options(&session.config),
            updates::available_commands(),
        )
    };
    responder.respond(
        acp::ResumeSessionResponse::new()
            .config_options(config)
            .available_commands(commands),
    )
}

/// Handle `session/close`.
pub(super) fn close_session(
    demo: &Demo,
    request: acp::CloseSessionRequest,
    responder: Responder<acp::CloseSessionResponse>,
) -> Result<(), Error> {
    if let Err(error) = demo.store().close(request.session_id.0.as_ref()) {
        return responder.respond_with_error(invalid(error));
    }
    responder.respond(acp::CloseSessionResponse::new())
}

/// Handle `session/fork`.
pub(super) fn fork_session(
    demo: &Demo,
    request: acp::ForkSessionRequest,
    responder: Responder<acp::ForkSessionResponse>,
) -> Result<(), Error> {
    let fork_id = demo
        .store()
        .fork(
            request.session_id.0.as_ref(),
            request.cwd.into_inner(),
            request
                .additional_directories
                .into_iter()
                .map(acp::AbsolutePath::into_inner)
                .collect(),
            request.mcp_servers.len(),
        )
        .map_err(invalid)?;
    let (config, commands) = {
        let store = demo.store();
        let session = store.get(&fork_id).map_err(invalid)?;
        (
            config_options(&session.config),
            updates::available_commands(),
        )
    };
    responder.respond(
        acp::ForkSessionResponse::new(fork_id)
            .config_options(config)
            .available_commands(commands),
    )
}

/// Handle `session/set_config_option` and announce the full option set.
pub(super) fn set_config_option(
    demo: &Demo,
    connection: &V2ConnectionTo<Client>,
    request: acp::SetSessionConfigOptionRequest,
    responder: Responder<acp::SetSessionConfigOptionResponse>,
) -> Result<(), Error> {
    let session_id = request.session_id.clone();
    let options = {
        let mut store = demo.store();
        let session = match store.get_mut(session_id.0.as_ref()) {
            Ok(session) => session,
            Err(error) => return responder.respond_with_error(invalid(error)),
        };
        apply_config(&mut session.config, &request)
            .map_err(|message| Error::invalid_params().data(message))?;
        session.updated_at = crate::demo::sessions::now();
        config_options(&session.config)
    };
    responder.respond(acp::SetSessionConfigOptionResponse::new(options.clone()))?;
    demo.send_update(
        connection,
        &session_id,
        acp::SessionUpdate::ConfigOptionUpdate(acp::ConfigOptionUpdate::new(options)),
    )
}

/// Apply one v2 config value to the stored session config.
fn apply_config(
    config: &mut crate::demo::SessionConfig,
    request: &acp::SetSessionConfigOptionRequest,
) -> Result<(), String> {
    let config_id = request.config_id.0.as_ref();
    match config_id {
        crate::demo::MODEL_CONFIG_ID => {
            let value = request
                .value
                .as_id()
                .map(|value| value.0.to_string())
                .ok_or_else(|| "model expects an id value".to_string())?;
            if !matches!(
                value.as_str(),
                crate::demo::MODEL_FAST | crate::demo::MODEL_DEEP
            ) {
                return Err(format!("unknown model `{value}`; use fast or deep"));
            }
            config.model = value;
            Ok(())
        }
        crate::demo::THINKING_CONFIG_ID => {
            config.thinking = request
                .value
                .as_bool()
                .ok_or_else(|| "thinking expects a boolean value".to_string())?;
            Ok(())
        }
        _ => Err(format!("unknown config option `{config_id}`")),
    }
}
