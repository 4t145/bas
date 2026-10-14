use crate::connection;

#[derive(Debug, thiserror::Error)]
pub(in crate::authentication) enum Failure {
    #[error("authentication elicitation has no request")]
    MissingRequest,
    #[error("invalid authentication payload: {0}")]
    InvalidPayload(String),
    #[error("could not read authentication configuration")]
    ConfigUnreadable,
    #[error("authentication registration changed")]
    RegistrationChanged,
    #[error("authentication input ended before start")]
    InputEnded,
    #[error("authentication input closed")]
    InputClosed,
    #[error("authentication stream ended without a check")]
    StreamEnded,
    #[error("could not start authentication: {0}")]
    StartRpc(connection::Failure),
    #[error("authentication stream failed: {0}")]
    StreamRpc(connection::Failure),
    #[error("{0}")]
    External(String),
}

impl Failure {
    pub(in crate::authentication) fn presentation(&self) -> String {
        match self {
            Self::MissingRequest => crate::i18n::tr!("auth-empty-request").into(),
            Self::InvalidPayload(message) => message.clone(),
            Self::ConfigUnreadable => crate::i18n::tr!("auth-check-config").into(),
            Self::RegistrationChanged => crate::i18n::tr!("auth-config-changed").into(),
            Self::InputEnded => crate::i18n::tr!("auth-ended").into(),
            Self::InputClosed => crate::i18n::tr!("auth-connection-closed").into(),
            Self::StreamEnded => crate::i18n::tr!("auth-disconnected").into(),
            Self::StartRpc(error) => rpc_error(crate::i18n::tr!("auth-start-failed"), error),
            Self::StreamRpc(error) => rpc_error(crate::i18n::tr!("auth-failed"), error),
            Self::External(message) => message.clone(),
        }
    }
}

fn rpc_error(action: &str, error: &connection::Failure) -> String {
    if error.code() == Some(tonic::Code::Unimplemented) {
        return crate::i18n::tr!("auth-upgrade").into();
    }
    if let connection::Failure::Rpc { message, code, .. } = error {
        let message = message.trim();
        if message.is_empty() {
            crate::i18n::tr!(
                "auth-rpc-error",
                action = (action).to_string(),
                code = format!("{code:?}")
            )
        } else {
            format!("{action}：{message}")
        }
    } else {
        format!("{action}：{}", error.presentation())
    }
}
