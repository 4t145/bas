use crate::agents::Status;

pub(super) fn label(status: &Status) -> &'static str {
    match status {
        Status::Checking => crate::i18n::tr!("agent-checking-program"),
        Status::Connecting => crate::i18n::tr!("agent-checking"),
        Status::Authenticating => crate::i18n::tr!("auth-progress"),
        Status::Installed(_) => crate::i18n::tr!("agent-unchecked"),
        Status::Connected(_) => crate::i18n::tr!("agent-check-passed"),
        Status::AuthenticationRequired { .. } => crate::i18n::tr!("auth-required"),
        Status::Missing(_) => crate::i18n::tr!("agent-program-missing"),
        Status::Failed(_) => crate::i18n::tr!("agent-check-failed"),
    }
}

pub(super) fn check_label(status: &Status) -> &'static str {
    match status {
        Status::Checking | Status::Connecting => crate::i18n::tr!("agent-checking"),
        Status::Authenticating => crate::i18n::tr!("agent-check-again"),
        Status::Installed(_) => crate::i18n::tr!("agent-check"),
        _ => crate::i18n::tr!("agent-check-again"),
    }
}
