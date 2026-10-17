//! Helpers that build and send v2 session updates.

use agent_client_protocol::schema::v2 as acp;
use agent_client_protocol::{Client, Error, V2ConnectionTo};

use super::Demo;
use crate::demo::{self, SessionConfig};

impl Demo {
    /// Send one session update and record it for replay.
    pub(super) fn send_update(
        &self,
        connection: &V2ConnectionTo<Client>,
        session_id: &acp::SessionId,
        update: acp::SessionUpdate,
    ) -> Result<(), Error> {
        self.store().record(session_id.0.as_ref(), update.clone());
        connection.send_notification(acp::UpdateSessionNotification::new(
            session_id.clone(),
            update,
        ))
    }

    /// Send an agent message chunk under `message_id`.
    pub(super) fn send_message_chunk(
        &self,
        connection: &V2ConnectionTo<Client>,
        session_id: &acp::SessionId,
        message_id: &acp::MessageId,
        content: impl Into<acp::ContentBlock>,
    ) -> Result<(), Error> {
        self.send_update(
            connection,
            session_id,
            acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(
                content.into(),
                message_id.clone(),
            )),
        )
    }

    /// Announce that foreground work started.
    pub(super) fn send_running(
        &self,
        connection: &V2ConnectionTo<Client>,
        session_id: &acp::SessionId,
    ) -> Result<(), Error> {
        self.send_update(
            connection,
            session_id,
            acp::SessionUpdate::StateUpdate(acp::StateUpdate::Running(
                acp::RunningStateUpdate::new(),
            )),
        )
    }

    /// Announce that foreground work ended.
    pub(super) fn send_idle(
        &self,
        connection: &V2ConnectionTo<Client>,
        session_id: &acp::SessionId,
        stop_reason: acp::StopReason,
    ) -> Result<(), Error> {
        self.send_update(
            connection,
            session_id,
            acp::SessionUpdate::StateUpdate(acp::StateUpdate::Idle(
                acp::IdleStateUpdate::new().stop_reason(stop_reason),
            )),
        )
    }
}

/// Typed config options for the current session values.
#[must_use]
pub(super) fn config_options(config: &SessionConfig) -> Vec<acp::SessionConfigOption> {
    vec![
        acp::SessionConfigOption::select(
            demo::MODEL_CONFIG_ID,
            "Model",
            config.model.clone(),
            vec![
                acp::SessionConfigSelectOption::new(demo::MODEL_FAST, "Fast"),
                acp::SessionConfigSelectOption::new(demo::MODEL_DEEP, "Deep"),
            ],
        )
        .description("Scripted model selector")
        .category(acp::SessionConfigOptionCategory::Model),
        acp::SessionConfigOption::boolean(demo::THINKING_CONFIG_ID, "Thinking", config.thinking)
            .description("Emit thought chunks before answers")
            .category(acp::SessionConfigOptionCategory::ThoughtLevel),
    ]
}

/// Commands advertised when a session starts and by `/commands`.
#[must_use]
pub(super) fn available_commands() -> Vec<acp::AvailableCommand> {
    demo::COMMANDS
        .iter()
        .map(|descriptor| {
            let command = acp::AvailableCommand::new(descriptor.name, descriptor.description);
            match descriptor.hint {
                Some(hint) => command.input(acp::AvailableCommandInput::Text(
                    acp::TextCommandInput::new(hint),
                )),
                None => command,
            }
        })
        .collect()
}

/// Project a stored session into list metadata.
#[must_use]
pub(super) fn session_info(
    session: &crate::demo::sessions::Session<acp::SessionUpdate>,
) -> acp::SessionInfo {
    acp::SessionInfo::new(session.id.clone(), session.cwd.clone())
        .additional_directories(session.additional_directories.clone())
        .title(session.title.clone())
        .updated_at(session.updated_at.clone())
}
