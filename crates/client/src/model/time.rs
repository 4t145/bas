use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// A wall-clock instant, encoded as milliseconds since the Unix epoch.
///
/// The daemon never reasons about local time zones; clients render timestamps
/// in the user's locale.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Timestamp(u64);

impl Timestamp {
    /// The current instant.
    #[must_use]
    pub fn now() -> Self {
        Self::from_system_time(SystemTime::now())
    }

    /// Converts a [`SystemTime`], saturating at the epoch for earlier values.
    #[must_use]
    pub fn from_system_time(time: SystemTime) -> Self {
        let millis = time
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_millis();
        Self(u64::try_from(millis).unwrap_or(u64::MAX))
    }

    /// Returns milliseconds since the Unix epoch.
    #[must_use]
    pub const fn millis(self) -> u64 {
        self.0
    }

    /// Wraps a raw millisecond value, for decoding protocol messages.
    #[must_use]
    pub const fn from_millis(millis: u64) -> Self {
        Self(millis)
    }
}
