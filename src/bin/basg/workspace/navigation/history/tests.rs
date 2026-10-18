use super::*;
use bas::model::{Owner, SessionId};

fn session(title: &str, at: &str) -> SessionMeta {
    let mut session = SessionMeta::new(SessionId::new(), "test".into(), Owner::current());
    session.title = Some(title.into());
    session.updated_at = bas::model::Timestamp::from_millis(
        at.parse::<Timestamp>().expect("timestamp").as_millisecond() as u64,
    );
    session
}

#[test]
fn history_uses_local_calendar_boundaries_and_sorts_recent_activity_first() {
    let now: Zoned = "2026-10-16T00:30:00+08:00[+08:00]"
        .parse()
        .expect("local now");
    let sessions = [
        session("last year", "2025-10-01T00:00:00Z"),
        session("this month", "2026-10-05T00:00:00Z"),
        session("yesterday", "2026-10-15T15:59:00Z"),
        session("last month", "2026-09-30T00:00:00Z"),
        session("this week", "2026-10-11T16:01:00Z"),
        session("today", "2026-10-15T16:10:00Z"),
    ];
    let groups = groups(&sessions, "", &now);
    assert_eq!(
        groups.iter().map(|group| &group.period).collect::<Vec<_>>(),
        [
            &Period::Today,
            &Period::Yesterday,
            &Period::Week,
            &Period::Month {
                year: 2026,
                month: 10
            },
            &Period::Month {
                year: 2026,
                month: 9
            },
            &Period::Month {
                year: 2025,
                month: 10
            },
        ]
    );
    assert!(
        groups
            .last()
            .expect("older month")
            .period
            .label(now.date())
            .contains("2025")
    );
    let filtered = super::groups(&sessions, "yesterday", &now);
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].period, Period::Yesterday);
}

#[test]
fn yesterday_and_this_week_take_precedence_across_month_and_year_boundaries() {
    let now: Zoned = "2026-01-01T12:00:00Z[UTC]".parse().expect("new year");
    let sessions = [
        session("last month", "2025-12-20T00:00:00Z"),
        session("monday", "2025-12-29T00:00:00Z"),
        session("yesterday", "2025-12-31T00:00:00Z"),
    ];
    let groups = groups(&sessions, "", &now);
    assert_eq!(
        groups.iter().map(|group| &group.period).collect::<Vec<_>>(),
        [
            &Period::Yesterday,
            &Period::Week,
            &Period::Month {
                year: 2025,
                month: 12
            },
        ]
    );
    let monday: Zoned = "2026-10-19T12:00:00Z[UTC]".parse().expect("monday");
    assert_eq!(
        period(&session("sunday", "2026-10-18T10:00:00Z"), &monday),
        Period::Yesterday
    );
    assert_eq!(
        period(&session("saturday", "2026-10-17T10:00:00Z"), &monday),
        Period::Month {
            year: 2026,
            month: 10
        }
    );
}
