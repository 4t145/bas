//! Conversions between the domain model and the protobuf wire types.
//!
//! The server converts `model` values into protobuf; clients convert protobuf
//! back so that TUI and GUI code shares the daemon's types instead of the
//! generated ones.

use std::{path::PathBuf, str::FromStr};

use crate::model;

use super::pb;

/// Failure converting between the domain model and protobuf wire values.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unknown {field} {value:?}")]
    Unknown { field: &'static str, value: String },
    #[error("the event carries no kind")]
    MissingEventKind,
    #[error("the turn carries no outcome")]
    MissingOutcome,
    #[error("session metadata carries no owner")]
    MissingOwner,
    #[error("working directory is not valid UTF-8; the protocol requires UTF-8 paths")]
    NonUtf8Cwd,
    #[error("invalid ACP JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid identifier: {0}")]
    Id(#[from] model::ParseIdError),
}

impl From<model::SessionStatus> for pb::SessionStatus {
    fn from(status: model::SessionStatus) -> Self {
        match status {
            model::SessionStatus::Starting => Self::Starting,
            model::SessionStatus::Idle => Self::Idle,
            model::SessionStatus::Working => Self::Working,
            model::SessionStatus::Stopped => Self::Stopped,
            model::SessionStatus::Failed => Self::Failed,
        }
    }
}

impl TryFrom<i32> for model::SessionStatus {
    type Error = Error;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match pb::SessionStatus::try_from(value) {
            Ok(pb::SessionStatus::Starting) => Ok(Self::Starting),
            Ok(pb::SessionStatus::Idle) => Ok(Self::Idle),
            Ok(pb::SessionStatus::Working) => Ok(Self::Working),
            Ok(pb::SessionStatus::Stopped) => Ok(Self::Stopped),
            Ok(pb::SessionStatus::Failed) => Ok(Self::Failed),
            Ok(pb::SessionStatus::Unspecified) | Err(_) => Err(Error::Unknown {
                field: "session status",
                value: value.to_string(),
            }),
        }
    }
}

impl From<&model::Owner> for pb::Owner {
    fn from(owner: &model::Owner) -> Self {
        Self {
            user: owner.user.clone(),
            host: owner.host.clone(),
        }
    }
}

impl From<pb::Owner> for model::Owner {
    fn from(owner: pb::Owner) -> Self {
        Self {
            user: owner.user,
            host: owner.host,
        }
    }
}

/// Validates a working directory at the UTF-8 protocol boundary.
///
/// # Errors
/// Returns an error rather than silently replacing non-UTF-8 path bytes.
pub fn utf8_cwd(path: &std::path::Path) -> Result<&str, Error> {
    path.to_str().ok_or(Error::NonUtf8Cwd)
}

impl TryFrom<&model::SessionMeta> for pb::Session {
    type Error = Error;

    fn try_from(meta: &model::SessionMeta) -> Result<Self, Self::Error> {
        Ok(Self {
            id: meta.id.to_string(),
            agent: meta.agent.to_string(),
            title: meta.title.clone(),
            owner: Some((&meta.owner).into()),
            cwd: meta
                .cwd
                .as_deref()
                .map(utf8_cwd)
                .transpose()?
                .map(str::to_owned),
            created_at: meta.created_at.millis(),
            updated_at: meta.updated_at.millis(),
            status: pb::SessionStatus::from(meta.status) as i32,
            archived: meta.archived,
            last_reply: meta.last_reply.raw(),
            revision: meta.revision.raw(),
            active_turn: meta.active_turn.map(|turn| turn.to_string()),
            initialization: meta.initialization.as_ref().map(pb::AcpPayload::from),
            acp: meta.acp.as_ref().map(pb::AcpPayload::from),
            available_commands: meta.available_commands.as_ref().map(pb::AcpPayload::from),
        })
    }
}

impl TryFrom<pb::Session> for model::SessionMeta {
    type Error = Error;

    fn try_from(session: pb::Session) -> Result<Self, Self::Error> {
        Ok(Self {
            id: model::SessionId::from_str(&session.id)?,
            agent: session.agent.into(),
            title: session.title,
            owner: session.owner.ok_or(Error::MissingOwner)?.into(),
            cwd: session.cwd.map(PathBuf::from),
            created_at: model::Timestamp::from_millis(session.created_at),
            updated_at: model::Timestamp::from_millis(session.updated_at),
            status: session.status.try_into()?,
            archived: session.archived,
            last_reply: model::Cursor::from_raw(session.last_reply),
            revision: model::Cursor::from_raw(session.revision),
            active_turn: session
                .active_turn
                .map(|turn| model::TurnId::from_str(&turn))
                .transpose()?,
            initialization: session.initialization.map(|p| p.decode()).transpose()?,
            acp: session.acp.map(|p| p.decode()).transpose()?,
            available_commands: session.available_commands.map(|p| p.decode()).transpose()?,
        })
    }
}

impl From<model::TurnReceipt> for pb::TurnAccepted {
    fn from(receipt: model::TurnReceipt) -> Self {
        Self {
            turn: receipt.turn.to_string(),
            cursor: receipt.cursor.raw(),
        }
    }
}

impl TryFrom<pb::TurnAccepted> for model::TurnReceipt {
    type Error = Error;

    /// # Errors
    /// Returns an error if the daemon returned an invalid turn identifier.
    fn try_from(receipt: pb::TurnAccepted) -> Result<Self, Self::Error> {
        Ok(Self {
            turn: model::TurnId::from_str(&receipt.turn)?,
            cursor: model::Cursor::from_raw(receipt.cursor),
        })
    }
}

impl pb::AcpPayload {
    /// Decodes using the ACP SDK schema expected by the RPC.
    /// # Errors
    /// Rejects malformed JSON or values that do not match the requested schema.
    pub fn decode<T: serde::de::DeserializeOwned>(&self) -> Result<T, Error> {
        Ok(serde_json::from_slice(&self.json)?)
    }
}

// Implement only SDK schemas carried by this service, avoiding an untyped public API.
macro_rules! acp_payload {
    ($($ty:ty),* $(,)?) => {$(
        impl From<&$ty> for pb::AcpPayload {
            fn from(value: &$ty) -> Self {
                Self { json: serde_json::to_vec(value).expect("ACP schema serializes as JSON") }
            }
        }
    )*};
}
acp_payload!(
    crate::acp::Error,
    crate::acp::InitializeResponse,
    crate::acp::NewSessionResponse,
    crate::acp::AvailableCommandsUpdate,
    crate::acp::PromptRequest,
    crate::acp::PromptResponse,
    crate::acp::CancelNotification,
    crate::acp::SetSessionConfigOptionRequest,
    crate::acp::SetSessionConfigOptionResponse,
    crate::acp::SetSessionModeRequest,
    crate::acp::SetSessionModeResponse,
    crate::acp::SessionUpdate,
);

impl From<&model::Event> for pb::Event {
    fn from(event: &model::Event) -> Self {
        use pb::event::Kind;
        let kind = match &event.kind {
            model::EventKind::Acp(update) => Kind::AcpUpdate(update.into()),
            model::EventKind::TurnStarted { turn } => Kind::TurnStarted(pb::TurnStarted {
                turn: turn.to_string(),
            }),
            model::EventKind::TurnFinished { turn, result } => {
                Kind::TurnFinished(pb::TurnFinished {
                    turn: turn.to_string(),
                    outcome: Some(match result {
                        Ok(response) => pb::turn_finished::Outcome::Response(response.into()),
                        Err(error) => pb::turn_finished::Outcome::Error(error.clone()),
                    }),
                })
            }
            model::EventKind::TitleChanged { title } => Kind::TitleChanged(pb::TitleChanged {
                title: title.clone(),
            }),
            model::EventKind::ArchiveChanged { archived } => {
                Kind::ArchiveChanged(pb::ArchiveChanged {
                    archived: *archived,
                })
            }
            model::EventKind::StatusChanged { status } => Kind::StatusChanged(pb::StatusChanged {
                status: pb::SessionStatus::from(*status) as i32,
            }),
            model::EventKind::Failed { message } => Kind::Failed(pb::Failed {
                message: message.clone(),
            }),
        };
        Self {
            session: event.session.to_string(),
            cursor: event.cursor.raw(),
            at: event.at.millis(),
            kind: Some(kind),
        }
    }
}

impl TryFrom<pb::Event> for model::Event {
    type Error = Error;
    fn try_from(event: pb::Event) -> Result<Self, Error> {
        use pb::event::Kind;
        Ok(Self {
            session: event.session.parse()?,
            cursor: model::Cursor::from_raw(event.cursor),
            at: model::Timestamp::from_millis(event.at),
            kind: match event.kind.ok_or(Error::MissingEventKind)? {
                Kind::AcpUpdate(payload) => model::EventKind::Acp(payload.decode()?),
                Kind::TurnStarted(value) => model::EventKind::TurnStarted {
                    turn: value.turn.parse()?,
                },
                Kind::TurnFinished(value) => model::EventKind::TurnFinished {
                    turn: value.turn.parse()?,
                    result: match value.outcome.ok_or(Error::MissingOutcome)? {
                        pb::turn_finished::Outcome::Response(payload) => Ok(payload.decode()?),
                        pb::turn_finished::Outcome::Error(error) => Err(error),
                    },
                },
                Kind::TitleChanged(value) => model::EventKind::TitleChanged { title: value.title },
                Kind::ArchiveChanged(value) => model::EventKind::ArchiveChanged {
                    archived: value.archived,
                },
                Kind::StatusChanged(value) => model::EventKind::StatusChanged {
                    status: value.status.try_into()?,
                },
                Kind::Failed(value) => model::EventKind::Failed {
                    message: value.message,
                },
            },
        })
    }
}
