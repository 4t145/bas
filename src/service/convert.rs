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
    #[error("submission carries no ACP prompt")]
    MissingPrompt,
    #[error("submission response carries no outcome")]
    MissingSubmissionOutcome,
    #[error("acceptance carries a zero cursor")]
    InvalidReceipt,
    #[error("session metadata carries no owner")]
    MissingOwner,
    #[error("session snapshot carries no runtime state")]
    MissingRuntime,
    #[error("permission carries no ACP request")]
    MissingPermissionRequest,
    #[error("working directory is not valid UTF-8; the protocol requires UTF-8 paths")]
    NonUtf8Cwd,
    #[error("invalid ACP JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid identifier: {0}")]
    Id(#[from] model::ParseIdError),
}

impl From<&model::PendingPermission> for pb::PendingPermission {
    fn from(permission: &model::PendingPermission) -> Self {
        Self {
            request_id: permission.request_id.clone(),
            request: Some((&permission.request).into()),
        }
    }
}

impl TryFrom<pb::PendingPermission> for model::PendingPermission {
    type Error = Error;

    fn try_from(permission: pb::PendingPermission) -> Result<Self, Self::Error> {
        Ok(Self {
            request_id: permission.request_id,
            request: permission
                .request
                .ok_or(Error::MissingPermissionRequest)?
                .decode()?,
        })
    }
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

impl TryFrom<&model::SessionSnapshot> for pb::Session {
    type Error = Error;

    fn try_from(snapshot: &model::SessionSnapshot) -> Result<Self, Self::Error> {
        let meta = &snapshot.meta;
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
            agent_updated_at: meta.agent_updated_at.map(model::Timestamp::millis),
            status: pb::SessionStatus::from(meta.status) as i32,
            archived: meta.archived,
            last_reply: meta.last_reply.raw(),
            revision: meta.revision.raw(),
            active_turn: meta.active_turn.map(|turn| turn.to_string()),
            initialization: meta
                .initialization_raw
                .as_ref()
                .map(pb::AcpPayload::from)
                .or_else(|| meta.initialization.as_ref().map(pb::AcpPayload::from)),
            acp: meta.acp.as_ref().map(pb::AcpPayload::from),
            available_commands: meta.available_commands.as_ref().map(pb::AcpPayload::from),
            runtime: Some((&snapshot.runtime).into()),
        })
    }
}

impl From<&model::SessionRuntime> for pb::SessionRuntime {
    fn from(runtime: &model::SessionRuntime) -> Self {
        Self {
            id: runtime.id.to_string(),
            failure: runtime.failure.as_ref().map(Into::into),
        }
    }
}

impl From<&model::RuntimeFailure> for pb::RuntimeFailure {
    fn from(failure: &model::RuntimeFailure) -> Self {
        match failure {
            model::RuntimeFailure::Persistence { message } => Self {
                kind: pb::runtime_failure::Kind::Persistence as i32,
                message: message.clone(),
            },
        }
    }
}

impl TryFrom<pb::RuntimeFailure> for model::RuntimeFailure {
    type Error = Error;

    fn try_from(failure: pb::RuntimeFailure) -> Result<Self, Self::Error> {
        match pb::runtime_failure::Kind::try_from(failure.kind) {
            Ok(pb::runtime_failure::Kind::Persistence) => Ok(Self::Persistence {
                message: failure.message,
            }),
            Ok(pb::runtime_failure::Kind::Unspecified) | Err(_) => Err(Error::Unknown {
                field: "runtime failure kind",
                value: failure.kind.to_string(),
            }),
        }
    }
}

impl TryFrom<pb::SessionRuntime> for model::SessionRuntime {
    type Error = Error;

    fn try_from(runtime: pb::SessionRuntime) -> Result<Self, Self::Error> {
        Ok(Self {
            id: runtime.id.parse().map_err(|_| Error::Unknown {
                field: "session runtime id",
                value: runtime.id,
            })?,
            failure: runtime.failure.map(TryInto::try_into).transpose()?,
        })
    }
}

impl TryFrom<pb::Session> for model::SessionSnapshot {
    type Error = Error;

    fn try_from(session: pb::Session) -> Result<Self, Self::Error> {
        let meta = model::SessionMeta {
            id: model::SessionId::from_str(&session.id)?,
            agent: session.agent.into(),
            title: session.title,
            owner: session.owner.ok_or(Error::MissingOwner)?.into(),
            cwd: session.cwd.map(PathBuf::from),
            created_at: model::Timestamp::from_millis(session.created_at),
            updated_at: model::Timestamp::from_millis(session.updated_at),
            agent_updated_at: session.agent_updated_at.map(model::Timestamp::from_millis),
            status: session.status.try_into()?,
            archived: session.archived,
            last_reply: model::Cursor::from_raw(session.last_reply),
            revision: model::Cursor::from_raw(session.revision),
            active_turn: session
                .active_turn
                .map(|turn| model::TurnId::from_str(&turn))
                .transpose()?,
            initialization_raw: session
                .initialization
                .as_ref()
                .map(|p| p.decode())
                .transpose()?,
            initialization: session.initialization.map(|p| p.decode()).transpose()?,
            acp: session.acp.map(|p| p.decode()).transpose()?,
            available_commands: session.available_commands.map(|p| p.decode()).transpose()?,
        };
        Ok(Self {
            meta,
            runtime: session.runtime.ok_or(Error::MissingRuntime)?.try_into()?,
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
    /// Rejects an invalid turn identifier or a cursor that cannot prove registration.
    fn try_from(receipt: pb::TurnAccepted) -> Result<Self, Self::Error> {
        if receipt.cursor == 0 {
            return Err(Error::InvalidReceipt);
        }
        Ok(Self {
            turn: model::TurnId::from_str(&receipt.turn)?,
            cursor: model::Cursor::from_raw(receipt.cursor),
        })
    }
}

impl From<&model::PromptSubmission> for pb::PromptSubmission {
    fn from(submission: &model::PromptSubmission) -> Self {
        Self {
            turn: submission.turn.to_string(),
            prompt: Some((&submission.request).into()),
        }
    }
}

impl TryFrom<pb::PromptSubmission> for model::PromptSubmission {
    type Error = Error;

    fn try_from(submission: pb::PromptSubmission) -> Result<Self, Self::Error> {
        Ok(Self {
            turn: submission.turn.parse()?,
            request: submission.prompt.ok_or(Error::MissingPrompt)?.decode()?,
        })
    }
}

impl From<model::PromptSubmissionOutcome> for pb::PromptSubmissionResponse {
    fn from(outcome: model::PromptSubmissionOutcome) -> Self {
        use model::{PromptRejectionKind as Kind, PromptSubmissionOutcome as Outcome};
        let outcome = match outcome {
            Outcome::Accepted(receipt) => {
                pb::prompt_submission_response::Outcome::Accepted(receipt.into())
            }
            Outcome::Rejected(rejection) => {
                let kind = match rejection.kind {
                    Kind::InvalidRequest => pb::PromptRejectionKind::InvalidRequest,
                    Kind::Busy => pb::PromptRejectionKind::Busy,
                    Kind::Archived => pb::PromptRejectionKind::Archived,
                    Kind::Unavailable => pb::PromptRejectionKind::Unavailable,
                    Kind::Conflict => pb::PromptRejectionKind::Conflict,
                };
                pb::prompt_submission_response::Outcome::Rejected(pb::PromptRejection {
                    kind: kind as i32,
                    message: rejection.message,
                })
            }
        };
        Self {
            outcome: Some(outcome),
        }
    }
}

impl TryFrom<pb::PromptSubmissionResponse> for model::PromptSubmissionOutcome {
    type Error = Error;

    fn try_from(response: pb::PromptSubmissionResponse) -> Result<Self, Self::Error> {
        use model::PromptRejectionKind as Kind;
        Ok(
            match response.outcome.ok_or(Error::MissingSubmissionOutcome)? {
                pb::prompt_submission_response::Outcome::Accepted(receipt) => {
                    Self::Accepted(receipt.try_into()?)
                }
                pb::prompt_submission_response::Outcome::Rejected(rejection) => {
                    let kind = match pb::PromptRejectionKind::try_from(rejection.kind) {
                        Ok(pb::PromptRejectionKind::InvalidRequest) => Kind::InvalidRequest,
                        Ok(pb::PromptRejectionKind::Busy) => Kind::Busy,
                        Ok(pb::PromptRejectionKind::Archived) => Kind::Archived,
                        Ok(pb::PromptRejectionKind::Unavailable) => Kind::Unavailable,
                        Ok(pb::PromptRejectionKind::Conflict) => Kind::Conflict,
                        Ok(pb::PromptRejectionKind::Unspecified) | Err(_) => {
                            return Err(Error::Unknown {
                                field: "prompt rejection kind",
                                value: rejection.kind.to_string(),
                            });
                        }
                    };
                    Self::Rejected(model::PromptRejection {
                        kind,
                        message: rejection.message,
                    })
                }
            },
        )
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

// Include original protocol objects so future extension fields survive projection.
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
    serde_json::Value,
    crate::acp::Error,
    crate::acp::InitializeResponse,
    crate::acp::ListSessionsResponse,
    crate::acp::SessionInfo,
    crate::acp::NewSessionResponse,
    crate::acp::AvailableCommandsUpdate,
    crate::acp::PromptRequest,
    crate::acp::PromptResponse,
    crate::acp::CancelNotification,
    crate::acp::RequestPermissionRequest,
    crate::acp::RequestPermissionResponse,
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
                        Err(error) => pb::turn_finished::Outcome::Error(error.into()),
                    }),
                })
            }
            model::EventKind::TurnInterrupted { turn, reason } => {
                use model::InterruptionReason as Reason;
                let (reason, message) = match reason {
                    Reason::BackendFailure { message } => {
                        (pb::InterruptionReason::BackendFailure, message.clone())
                    }
                    Reason::DispatchFailure { message } => {
                        (pb::InterruptionReason::DispatchFailure, message.clone())
                    }
                    Reason::SessionStopped => {
                        (pb::InterruptionReason::SessionStopped, String::new())
                    }
                    Reason::DaemonRestarted => {
                        (pb::InterruptionReason::DaemonRestarted, String::new())
                    }
                    Reason::CancelledBeforeDispatch => (
                        pb::InterruptionReason::CancelledBeforeDispatch,
                        String::new(),
                    ),
                };
                Kind::TurnInterrupted(pb::TurnInterrupted {
                    turn: turn.to_string(),
                    reason: reason as i32,
                    message,
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
                        pb::turn_finished::Outcome::Error(payload) => Err(payload.decode()?),
                    },
                },
                Kind::TurnInterrupted(value) => {
                    use model::InterruptionReason as Reason;
                    let reason = match pb::InterruptionReason::try_from(value.reason) {
                        Ok(pb::InterruptionReason::BackendFailure) => Reason::BackendFailure {
                            message: value.message,
                        },
                        Ok(pb::InterruptionReason::DispatchFailure) => Reason::DispatchFailure {
                            message: value.message,
                        },
                        Ok(pb::InterruptionReason::SessionStopped) => Reason::SessionStopped,
                        Ok(pb::InterruptionReason::DaemonRestarted) => Reason::DaemonRestarted,
                        Ok(pb::InterruptionReason::CancelledBeforeDispatch) => {
                            Reason::CancelledBeforeDispatch
                        }
                        Ok(pb::InterruptionReason::Unspecified) | Err(_) => {
                            return Err(Error::Unknown {
                                field: "interruption reason",
                                value: value.reason.to_string(),
                            });
                        }
                    };
                    model::EventKind::TurnInterrupted {
                        turn: value.turn.parse()?,
                        reason,
                    }
                }
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
