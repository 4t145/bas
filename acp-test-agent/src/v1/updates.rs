//! Helpers that build and send v1 session updates.

use agent_client_protocol::schema::v1 as acp;
use agent_client_protocol::{Client, ConnectionTo};

use super::Demo;
use crate::demo::{self, SessionConfig};

impl Demo {
    /// Send one session update and record it for later replay.
    pub(super) fn send_update(
        &self,
        connection: &ConnectionTo<Client>,
        session_id: &acp::SessionId,
        update: acp::SessionUpdate,
    ) -> Result<(), acp::Error> {
        self.store().record(session_id.0.as_ref(), update.clone());
        connection.send_notification(acp::SessionNotification::new(session_id.clone(), update))
    }

    /// Send an agent message chunk under `message_id`.
    pub(super) fn send_message_chunk(
        &self,
        connection: &ConnectionTo<Client>,
        session_id: &acp::SessionId,
        message_id: &acp::MessageId,
        text: impl Into<String>,
    ) -> Result<(), acp::Error> {
        self.send_update(
            connection,
            session_id,
            acp::SessionUpdate::AgentMessageChunk(
                acp::ContentChunk::new(acp::ContentBlock::from(text))
                    .message_id(message_id.clone()),
            ),
        )
    }

    /// Send a thought chunk under `message_id`.
    pub(super) fn send_thought_chunk(
        &self,
        connection: &ConnectionTo<Client>,
        session_id: &acp::SessionId,
        message_id: &acp::MessageId,
        text: impl Into<String>,
    ) -> Result<(), acp::Error> {
        self.send_update(
            connection,
            session_id,
            acp::SessionUpdate::AgentThoughtChunk(
                acp::ContentChunk::new(acp::ContentBlock::from(text))
                    .message_id(message_id.clone()),
            ),
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

/// The mode state advertised when a v1 session starts.
#[must_use]
pub(super) fn modes(current: &str) -> acp::SessionModeState {
    acp::SessionModeState::new(
        current.to_string(),
        vec![
            acp::SessionMode::new(demo::MODE_ASK, "Ask").description("Answer and explain"),
            acp::SessionMode::new(demo::MODE_CODE, "Code").description("Edit code"),
            acp::SessionMode::new(demo::MODE_REVIEW, "Review").description("Review changes"),
        ],
    )
}

/// Commands advertised when a session starts and by `/commands`.
#[must_use]
pub(super) fn available_commands() -> Vec<acp::AvailableCommand> {
    demo::COMMANDS
        .iter()
        .map(|descriptor| {
            let command = acp::AvailableCommand::new(descriptor.name, descriptor.description);
            match descriptor.hint {
                Some(hint) => command.input(acp::AvailableCommandInput::Unstructured(
                    acp::UnstructuredCommandInput::new(hint),
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
