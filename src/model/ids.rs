use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Error returned when a textual identifier cannot be parsed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParseIdError {
    kind: &'static str,
    value: String,
}

impl ParseIdError {
    fn new(kind: &'static str, value: impl Into<String>) -> Self {
        Self {
            kind,
            value: value.into(),
        }
    }
}

impl fmt::Display for ParseIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid {} {:?}", self.kind, self.value)
    }
}

impl std::error::Error for ParseIdError {}

macro_rules! define_uuid_id {
    ($name:ident, $kind:literal, $generator:ident) => {
        #[doc = concat!("Opaque identifier for a ", $kind, ".")]
        #[derive(
            Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            /// Generates a fresh identifier.
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::$generator())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl FromStr for $name {
            type Err = ParseIdError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Uuid::parse_str(value)
                    .map(Self)
                    .map_err(|_| ParseIdError::new($kind, value))
            }
        }
    };
}

define_uuid_id!(SessionId, "session id", new_v4);
define_uuid_id!(MessageId, "message id", now_v7);
define_uuid_id!(TurnId, "turn id", now_v7);

/// Name of a configured agent backend, for example `embedded` or `opencode`.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AgentId(String);

impl AgentId {
    /// Wraps a configured backend name.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// Borrows the configured name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AgentId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl From<&str> for AgentId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for AgentId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

/// Position of a replayable event inside a session.
///
/// Cursors are assigned by the daemon in strictly increasing order and never
/// reused, so a client can resume a stream with the last cursor it has seen.
#[derive(
    Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Cursor(u64);

impl Cursor {
    /// The position before the first event of a session.
    ///
    /// A client passes this to load history from the beginning; real cursors
    /// start at [`START`](Self::START)`.next()` so the sentinel never collides
    /// with an event.
    pub const START: Self = Self(0);

    /// Wraps a raw cursor value, as received from a client.
    #[must_use]
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    /// Returns the raw value, for encoding into a protocol message.
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }

    /// Returns the cursor of the event that follows this one.
    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

impl fmt::Display for Cursor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}
