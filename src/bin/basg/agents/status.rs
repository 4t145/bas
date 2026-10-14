//! Shared frontend state; transport mechanics live in `check`.

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Status {
    Checking,
    Connecting,
    Authenticating,
    Installed(String),
    Connected(String),
    AuthenticationRequired {
        program: String,
        methods: Vec<bas::authentication::Method>,
    },
    Missing(String),
    Failed(String),
}

impl Status {
    pub(crate) fn connected(&self) -> bool {
        matches!(self, Self::Connected(_))
    }

    pub(crate) fn busy(&self) -> bool {
        matches!(
            self,
            Self::Checking | Self::Connecting | Self::Authenticating
        )
    }
}
