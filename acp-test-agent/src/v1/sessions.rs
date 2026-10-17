//! v1 session lifecycle request handlers.

use agent_client_protocol::schema::v1 as acp;
use agent_client_protocol::{Client, ConnectionTo, Responder};

use super::Demo;
use super::updates::{self, config_options};
use crate::demo::sessions::Error as SessionError;

fn invalid(error: SessionError) -> acp::Error {
    acp::Error::invalid_params().data(error.to_string())
}

/// Handle `session/new`.
pub(super) fn new_session(
    demo: &Demo,
    request: acp::NewSessionRequest,
    responder: Responder<acp::NewSessionResponse>,
) -> Result<(), acp::Error> {
    if let Err(error) = demo.require_authenticated() {
        return responder.respond_with_error(error);
    }
    let session_id = demo.store().create(
        request.cwd,
        request.additional_directories,
        request.mcp_servers.len(),
    );
    let response = {
        let store = demo.store();
        let session = store.get(&session_id).map_err(invalid)?;
        acp::NewSessionResponse::new(session_id.clone())
            .modes(updates::modes(&session.mode))
            .config_options(config_options(&session.config))
    };
    responder.respond(response)
}

/// Handle `session/load`: replay recorded updates, then restore the session.
pub(super) fn load_session(
    demo: &Demo,
    connection: &ConnectionTo<Client>,
    request: acp::LoadSessionRequest,
    responder: Responder<acp::LoadSessionResponse>,
) -> Result<(), acp::Error> {
    let session_id = request.session_id.0.to_string();
    let history = {
        let mut store = demo.store();
        if let Err(error) = store.resume(
            &session_id,
            &request.cwd,
            request.additional_directories,
            request.mcp_servers.len(),
        ) {
            return responder.respond_with_error(invalid(error));
        }
        store
            .get(&session_id)
            .map(|session| session.history.clone())
            .unwrap_or_default()
    };
    for update in history {
        connection.send_notification(acp::SessionNotification::new(
            acp::SessionId::new(session_id.clone()),
            update,
        ))?;
    }
    let (mode, config) = {
        let store = demo.store();
        let session = store.get(&session_id).map_err(invalid)?;
        (session.mode.clone(), session.config.clone())
    };
    responder.respond(
        acp::LoadSessionResponse::new()
            .modes(updates::modes(&mode))
            .config_options(config_options(&config)),
    )
}

/// Handle `session/list` with the configured page size.
pub(super) fn list_sessions(
    demo: &Demo,
    request: acp::ListSessionsRequest,
    responder: Responder<acp::ListSessionsResponse>,
) -> Result<(), acp::Error> {
    let store = demo.store();
    let page = store
        .page(request.cwd.as_deref(), request.cursor.as_deref())
        .map_err(invalid)?;
    responder.respond(
        acp::ListSessionsResponse::new(
            page.sessions
                .iter()
                .map(|session| updates::session_info(session))
                .collect(),
        )
        .next_cursor(page.next_cursor),
    )
}

/// Handle `session/delete`.
pub(super) fn delete_session(
    demo: &Demo,
    request: acp::DeleteSessionRequest,
    responder: Responder<acp::DeleteSessionResponse>,
) -> Result<(), acp::Error> {
    if let Err(error) = demo.store().delete(request.session_id.0.as_ref()) {
        return responder.respond_with_error(invalid(error));
    }
    responder.respond(acp::DeleteSessionResponse::new())
}

/// Handle `session/resume`.
pub(super) fn resume_session(
    demo: &Demo,
    request: acp::ResumeSessionRequest,
    responder: Responder<acp::ResumeSessionResponse>,
) -> Result<(), acp::Error> {
    let session_id = request.session_id.0.as_ref();
    let (mode, config) = {
        let mut store = demo.store();
        if let Err(error) = store.resume(
            session_id,
            &request.cwd,
            request.additional_directories,
            request.mcp_servers.len(),
        ) {
            return responder.respond_with_error(invalid(error));
        }
        let session = store.get(session_id).map_err(invalid)?;
        (session.mode.clone(), session.config.clone())
    };
    responder.respond(
        acp::ResumeSessionResponse::new()
            .modes(updates::modes(&mode))
            .config_options(config_options(&config)),
    )
}

/// Handle `session/close`.
pub(super) fn close_session(
    demo: &Demo,
    request: acp::CloseSessionRequest,
    responder: Responder<acp::CloseSessionResponse>,
) -> Result<(), acp::Error> {
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
) -> Result<(), acp::Error> {
    let fork_id = demo
        .store()
        .fork(
            request.session_id.0.as_ref(),
            request.cwd,
            request.additional_directories,
            request.mcp_servers.len(),
        )
        .map_err(invalid)?;
    let (mode, config) = {
        let store = demo.store();
        let session = store.get(&fork_id).map_err(invalid)?;
        (session.mode.clone(), session.config.clone())
    };
    responder.respond(
        acp::ForkSessionResponse::new(fork_id)
            .modes(updates::modes(&mode))
            .config_options(config_options(&config)),
    )
}

/// Handle `session/set_mode` and announce the new mode.
pub(super) fn set_mode(
    demo: &Demo,
    connection: &ConnectionTo<Client>,
    request: acp::SetSessionModeRequest,
    responder: Responder<acp::SetSessionModeResponse>,
) -> Result<(), acp::Error> {
    let mode = request.mode_id.0.as_ref();
    if !matches!(
        mode,
        crate::demo::MODE_ASK | crate::demo::MODE_CODE | crate::demo::MODE_REVIEW
    ) {
        return responder.respond_with_error(
            acp::Error::invalid_params()
                .data(format!("unknown mode `{mode}`; use ask, code, or review")),
        );
    }
    let session_id = request.session_id.clone();
    {
        let mut store = demo.store();
        let session = match store.get_mut(session_id.0.as_ref()) {
            Ok(session) => session,
            Err(error) => return responder.respond_with_error(invalid(error)),
        };
        session.mode = mode.to_string();
        session.updated_at = crate::demo::sessions::now();
    }
    responder.respond(acp::SetSessionModeResponse::new())?;
    demo.send_update(
        connection,
        &session_id,
        acp::SessionUpdate::CurrentModeUpdate(acp::CurrentModeUpdate::new(mode.to_string())),
    )
}

/// Handle `session/set_config_option` and announce the full option set.
pub(super) fn set_config_option(
    demo: &Demo,
    connection: &ConnectionTo<Client>,
    request: acp::SetSessionConfigOptionRequest,
    responder: Responder<acp::SetSessionConfigOptionResponse>,
) -> Result<(), acp::Error> {
    let session_id = request.session_id.clone();
    let options = {
        let mut store = demo.store();
        let session = match store.get_mut(session_id.0.as_ref()) {
            Ok(session) => session,
            Err(error) => return responder.respond_with_error(invalid(error)),
        };
        apply_config(&mut session.config, &request)
            .map_err(|message| acp::Error::invalid_params().data(message))?;
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

/// Apply one v1 config value to the stored session config.
fn apply_config(
    config: &mut crate::demo::SessionConfig,
    request: &acp::SetSessionConfigOptionRequest,
) -> Result<(), String> {
    let config_id = request.config_id.0.as_ref();
    match config_id {
        crate::demo::MODEL_CONFIG_ID => {
            let value = request
                .value
                .as_value_id()
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
