//! Calendar groups for Bas session activity, in the user's local time zone.

use bas::model::SessionMeta;
use jiff::{Span, Timestamp, Zoned, civil::Date};

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Period {
    Today,
    Yesterday,
    Week,
    Month { year: i16, month: i8 },
    Unknown,
}

impl Period {
    pub(super) fn id(&self) -> String {
        match self {
            Self::Today => "history-today".into(),
            Self::Yesterday => "history-yesterday".into(),
            Self::Week => "history-week".into(),
            Self::Month { year, month } => format!("history-{year}-{month}"),
            Self::Unknown => "history-unknown".into(),
        }
    }

    pub(super) fn label(&self, today: Date) -> String {
        match self {
            Self::Today => crate::i18n::tr!("history-today").into(),
            Self::Yesterday => crate::i18n::tr!("history-yesterday").into(),
            Self::Week => crate::i18n::tr!("history-week").into(),
            Self::Month { year, month } => {
                let month = crate::i18n::tr!("history-month", month = *month);
                if *year == today.year() {
                    month
                } else {
                    crate::i18n::tr!("history-month-year", year = year.to_string(), month = month)
                }
            }
            Self::Unknown => crate::i18n::tr!("history-unknown").into(),
        }
    }
}

pub(super) struct Group<'a> {
    pub period: Period,
    pub sessions: Vec<&'a SessionMeta>,
}

pub(super) fn sorted(sessions: &[SessionMeta]) -> Vec<&SessionMeta> {
    let mut sessions: Vec<_> = sessions.iter().collect();
    sessions.sort_by_key(|session| (std::cmp::Reverse(session.updated_at), session.id));
    sessions
}

fn period(session: &SessionMeta, now: &Zoned) -> Period {
    let at = i64::try_from(session.updated_at.millis())
        .ok()
        .and_then(|millis| Timestamp::from_millisecond(millis).ok());
    let Some(at) = at else {
        return Period::Unknown;
    };
    let date = at.to_zoned(now.time_zone().clone()).date();
    let today = now.date();
    if date >= today {
        return Period::Today;
    }
    if today.checked_sub(Span::new().days(1)).ok() == Some(date) {
        return Period::Yesterday;
    }
    let week_start = today
        .checked_sub(Span::new().days(today.weekday().to_monday_zero_offset()))
        .unwrap_or(today);
    if date >= week_start {
        return Period::Week;
    }
    Period::Month {
        year: date.year(),
        month: date.month(),
    }
}

pub(super) fn groups<'a>(sessions: &'a [SessionMeta], query: &str, now: &Zoned) -> Vec<Group<'a>> {
    let mut groups: Vec<Group<'a>> = Vec::new();
    for session in sorted(sessions)
        .into_iter()
        .filter(|session| crate::workspace::matches_query(session, query))
    {
        let period = period(session, now);
        if let Some(group) = groups.last_mut().filter(|group| group.period == period) {
            group.sessions.push(session);
        } else {
            groups.push(Group {
                period,
                sessions: vec![session],
            });
        }
    }
    groups
}

#[cfg(test)]
mod tests;
