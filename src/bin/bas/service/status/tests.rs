use bas::{model::SessionStatus, service::acp_error};
use serde_json::json;
use tonic::Code;

use super::*;
use crate::{agent, storage};

#[test]
fn native_acp_errors_keep_one_mapping_across_creation_and_session_operations() {
    for (code, expected) in [
        (acp::ErrorCode::AuthRequired, Code::Unauthenticated),
        (acp::ErrorCode::InvalidParams, Code::InvalidArgument),
        (acp::ErrorCode::InvalidRequest, Code::InvalidArgument),
        (acp::ErrorCode::MethodNotFound, Code::Unimplemented),
        (acp::ErrorCode::InternalError, Code::Unknown),
    ] {
        let error = acp::Error::new(i32::from(code), "agent rejected request").data(json!({
            "counter": 9_007_199_254_740_993_u64,
            "extension": {"retry": false}
        }));
        let original = serde_json::to_value(&error).expect("ACP error JSON");
        let statuses = [
            Status::from(CreateError::Agent(agent::Error::Acp(error.clone()))),
            Status::from(SessionError::Agent(agent::Error::Acp(error.clone()))),
            Status::from(CreateError::Session(SessionError::Agent(
                agent::Error::Acp(error),
            ))),
        ];
        for status in statuses {
            assert_eq!(status.code(), expected);
            assert_eq!(status.message(), "agent rejected request");
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(status.details())
                    .expect("native ACP details"),
                original
            );
            assert_eq!(
                acp_error(&status).expect("client decodes ACP error").code,
                code
            );
        }
    }
}

#[test]
fn daemon_failures_never_masquerade_as_native_acp_errors() {
    for (status, expected) in [
        (Status::from(CreateError::Timeout), Code::DeadlineExceeded),
        (Status::from(CreateError::Stopping), Code::Unavailable),
        (
            Status::from(SessionError::Inactive(SessionStatus::Failed)),
            Code::FailedPrecondition,
        ),
        (
            Status::from(SessionError::Storage(storage::Error::WorkerClosed)),
            Code::Unavailable,
        ),
        (
            Status::from(CreateError::Storage(storage::Error::WorkerClosed)),
            Code::Unavailable,
        ),
        (
            Status::from(SessionError::Agent(agent::Error::backend(
                "fixture",
                "process exited",
            ))),
            Code::Unavailable,
        ),
        (
            Status::from(AttachError::Storage("read failed".into())),
            Code::Unavailable,
        ),
        (Status::from(AttachError::Lagged(2)), Code::DataLoss),
    ] {
        assert_eq!(status.code(), expected);
        assert!(!status.message().is_empty());
        assert!(
            status.details().is_empty(),
            "daemon failures have no ACP payload"
        );
        assert!(acp_error(&status).is_err());
    }
}
