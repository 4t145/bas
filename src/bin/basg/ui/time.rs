//! One relative-time policy for conversation history and other interface timestamps.

use bas::model::Timestamp;
use std::time::Duration;

pub(crate) const REFRESH_INTERVAL: Duration = Duration::from_secs(60);
const MINUTE: u64 = 60;
const HOUR: u64 = 60 * MINUTE;
const DAY: u64 = 24 * HOUR;

pub(crate) fn recent_time(at: Timestamp, now: Timestamp) -> String {
    relative_age(Duration::from_millis(
        now.millis().saturating_sub(at.millis()),
    ))
}

pub(crate) fn relative_age(age: Duration) -> String {
    let seconds = age.as_secs();
    if seconds < MINUTE {
        crate::i18n::tr!("time-just-now").into()
    } else if seconds < HOUR {
        crate::i18n::tr!("time-minutes", count = seconds / MINUTE)
    } else if seconds < DAY {
        crate::i18n::tr!("time-hours", count = seconds / HOUR)
    } else {
        crate::i18n::tr!("time-days", count = seconds / DAY)
    }
}

pub(crate) fn parse_rfc3339(value: &str) -> Option<Timestamp> {
    let at =
        time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339).ok()?;
    let millis = u64::try_from(at.unix_timestamp_nanos() / 1_000_000).ok()?;
    Some(Timestamp::from_millis(millis))
}
