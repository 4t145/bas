//! Transport adaptation for daemon failures and native ACP errors.
//! Only native ACP errors populate status details with ACP JSON.

use bas::acp;
use tonic::Status;

use crate::session::{AttachError, CreateError, HistoryError, SessionError};

#[cfg(test)]
mod tests;

fn acp_status(error: &acp::Error) -> Status {
    acp_status_raw(error, &serde_json::to_value(error).expect("ACP error JSON"))
}

fn acp_status_raw(error: &acp::Error, raw: &serde_json::Value) -> Status {
    let code = match error.code {
        acp::ErrorCode::AuthRequired => tonic::Code::Unauthenticated,
        acp::ErrorCode::InvalidParams | acp::ErrorCode::InvalidRequest => {
            tonic::Code::InvalidArgument
        }
        acp::ErrorCode::MethodNotFound => tonic::Code::Unimplemented,
        _ => tonic::Code::Unknown,
    };
    Status::with_details(
        code,
        error.message.clone(),
        serde_json::to_vec(raw).expect("ACP error JSON").into(),
    )
}

impl From<SessionError> for Status {
    fn from(error: SessionError) -> Self {
        match &error {
            SessionError::Runtime(_) => Status::failed_precondition(error.to_string()),
            SessionError::Inactive(_) => Status::failed_precondition(error.to_string()),
            SessionError::History(error) => (*error).into(),
            SessionError::Storage(crate::storage::Error::Expired { .. }) => {
                Status::data_loss(error.to_string())
            }
            SessionError::Storage(
                crate::storage::Error::Future { .. } | crate::storage::Error::EmptyPage,
            ) => Status::invalid_argument(error.to_string()),
            SessionError::Storage(_) => Status::unavailable(error.to_string()),
            SessionError::CommitTask(_) => Status::unavailable(error.to_string()),
            SessionError::Agent(crate::agent::Error::AcpRaw { error, raw }) => {
                acp_status_raw(error, raw)
            }
            SessionError::Agent(crate::agent::Error::Acp(error)) => acp_status(error),
            SessionError::Agent(crate::agent::Error::Unsupported { .. }) => {
                Status::unimplemented(error.to_string())
            }
            SessionError::Agent(_) => Status::unavailable(error.to_string()),
        }
    }
}

impl From<HistoryError> for Status {
    fn from(error: HistoryError) -> Self {
        match error {
            HistoryError::Expired { .. } => Status::data_loss(error.to_string()),
            HistoryError::Future { .. } | HistoryError::EmptyPage => {
                Status::invalid_argument(error.to_string())
            }
        }
    }
}

impl From<CreateError> for Status {
    fn from(error: CreateError) -> Self {
        match error {
            CreateError::UnknownAgent(_) => Status::not_found(error.to_string()),
            CreateError::NotResumable(_) => Status::failed_precondition(error.to_string()),
            CreateError::Cwd(_) => Status::invalid_argument(error.to_string()),
            CreateError::Timeout => Status::deadline_exceeded(error.to_string()),
            CreateError::Session(error) => error.into(),
            CreateError::Storage(crate::storage::Error::NotFound(_)) => {
                Status::not_found(error.to_string())
            }
            CreateError::Storage(_) => Status::unavailable(error.to_string()),
            CreateError::Agent(crate::agent::Error::Acp(acp_error)) => acp_status(&acp_error),
            CreateError::Agent(crate::agent::Error::AcpRaw { error, raw }) => {
                acp_status_raw(&error, &raw)
            }
            CreateError::Agent(crate::agent::Error::Unsupported { .. }) => {
                Status::failed_precondition(error.to_string())
            }
            CreateError::Agent(_) | CreateError::Stopping => Status::unavailable(error.to_string()),
        }
    }
}

impl From<AttachError> for Status {
    fn from(error: AttachError) -> Self {
        match error {
            AttachError::Lagged(_) => Self::data_loss(error.to_string()),
            AttachError::Runtime(_) => Self::failed_precondition(error.to_string()),
            AttachError::Closed => Self::unavailable(error.to_string()),
            AttachError::Storage(_) => Self::unavailable(error.to_string()),
        }
    }
}

pub(super) fn metadata_error(error: bas::service::Error) -> Status {
    Status::failed_precondition(error.to_string())
}
