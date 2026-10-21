use tonic::Code;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Condition {
    MissingCreatedSession,
    MissingArchivedSession,
    MissingSessionInfo,
    UnknownSubmission,
    ElicitationWrongSession,
    ElicitationEnded,
    MissingElicitationRequest,
    PermissionStreamClosed,
    PermissionWrongSession,
    TrackingWrongSession,
    TrackingEnded,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum FailureContext {
    StreamDeadline,
    RpcDeadline,
    ArchiveUnsupported,
    PermissionsUnsupported,
    PermissionSubscription,
    PermissionConnection,
    PermissionDisconnected,
    InvalidPermission,
    ElicitationDisconnected,
    ElicitationRead,
    InvalidEvent,
}

/// Owned failures cross the executor/UI boundary without choosing a locale.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub(crate) enum Failure {
    #[error("request timed out")]
    Timeout,
    #[error("RPC {code}: {message}")]
    Rpc {
        code: Code,
        message: String,
        details: Vec<u8>,
        diagnostic: String,
    },
    #[error("invalid payload: {0}")]
    InvalidPayload(String),
    #[error("{0}")]
    External(String),
    #[error("{0}")]
    Runtime(bas::model::RuntimeFailure),
    #[error("{0:?}")]
    Condition(Condition),
    #[error("{context:?}: {source}")]
    Context {
        context: FailureContext,
        source: Box<Failure>,
    },
    #[error("submission preflight failed: {0}")]
    Preflight(Box<Failure>),
}

impl Failure {
    pub fn context(self, context: FailureContext) -> Self {
        Self::Context {
            context,
            source: Box::new(self),
        }
    }

    pub fn code(&self) -> Option<Code> {
        match self {
            Self::Rpc { code, .. } => Some(*code),
            Self::Context { source, .. } | Self::Preflight(source) => source.code(),
            _ => None,
        }
    }

    /// Evaluate only when presenting a failure to the user.
    pub fn presentation(&self) -> String {
        match self {
            Self::Timeout => crate::i18n::tr!("connection-request-timeout").into(),
            Self::Rpc { diagnostic, .. } => diagnostic.clone(),
            Self::InvalidPayload(message) | Self::External(message) => message.clone(),
            Self::Runtime(failure) => failure.to_string(),
            Self::Preflight(source) => source.presentation(),
            Self::Condition(condition) => match condition {
                Condition::MissingCreatedSession => crate::i18n::tr!("session-create-empty").into(),
                Condition::MissingArchivedSession => {
                    crate::i18n::tr!("session-archive-empty").into()
                }
                Condition::MissingSessionInfo => crate::i18n::tr!("session-info-empty").into(),
                Condition::UnknownSubmission => crate::i18n::tr!("submission-unknown").into(),
                Condition::ElicitationWrongSession => {
                    crate::i18n::tr!("interaction-wrong-session").into()
                }
                Condition::ElicitationEnded => crate::i18n::tr!("interaction-ended").into(),
                Condition::MissingElicitationRequest => {
                    crate::i18n::tr!("elicitation-empty").into()
                }
                Condition::PermissionStreamClosed => crate::i18n::tr!("approval-closed").into(),
                Condition::PermissionWrongSession => {
                    crate::i18n::tr!("approval-wrong-session").into()
                }
                Condition::TrackingWrongSession => {
                    crate::i18n::tr!("turn-tracking-wrong-session").into()
                }
                Condition::TrackingEnded => crate::i18n::tr!("turn-tracking-ended").into(),
            },
            Self::Context { context, source } => {
                let error = source.presentation();
                match context {
                    FailureContext::StreamDeadline => "deadline has elapsed".into(),
                    FailureContext::RpcDeadline => {
                        tonic::Status::deadline_exceeded(error).to_string()
                    }
                    FailureContext::ArchiveUnsupported => {
                        crate::i18n::tr!("session-archive-upgrade").into()
                    }
                    FailureContext::PermissionsUnsupported => {
                        crate::i18n::tr!("approval-upgrade").into()
                    }
                    FailureContext::PermissionSubscription => {
                        crate::i18n::tr!("approval-subscribe-failed", status = error)
                    }
                    FailureContext::PermissionConnection => {
                        crate::i18n::tr!("approval-connect-failed", error = error)
                    }
                    FailureContext::PermissionDisconnected => {
                        crate::i18n::tr!("approval-disconnected", error = error)
                    }
                    FailureContext::InvalidPermission => {
                        crate::i18n::tr!("approval-invalid", error = error)
                    }
                    FailureContext::ElicitationDisconnected => {
                        crate::i18n::tr!("interaction-disconnected", error = error)
                    }
                    FailureContext::ElicitationRead => {
                        crate::i18n::tr!("interaction-read-failed", error = error)
                    }
                    FailureContext::InvalidEvent => {
                        crate::i18n::tr!("session-invalid-event", error = error)
                    }
                }
            }
        }
    }
}

impl From<tonic::Status> for Failure {
    fn from(status: tonic::Status) -> Self {
        Self::Rpc {
            code: status.code(),
            message: status.message().into(),
            details: status.details().to_vec(),
            diagnostic: status.to_string(),
        }
    }
}

impl From<String> for Failure {
    fn from(message: String) -> Self {
        Self::External(message)
    }
}

impl From<&str> for Failure {
    fn from(message: &str) -> Self {
        Self::External(message.into())
    }
}

impl From<anyhow::Error> for Failure {
    fn from(error: anyhow::Error) -> Self {
        for cause in error.chain() {
            if let Some(failure) = cause.downcast_ref::<Self>() {
                return failure.clone();
            }
            if let Some(status) = cause.downcast_ref::<tonic::Status>() {
                return status.clone().into();
            }
            if cause.is::<bas::service::Error>() || cause.is::<serde_json::Error>() {
                return Self::InvalidPayload(format!("{error:#}"));
            }
        }
        Self::External(format!("{error:#}"))
    }
}
