use super::*;

#[test]
fn relative_times_share_boundaries_and_tolerate_clock_skew() {
    let now = Timestamp::from_millis(7 * DAY * 1_000);
    for (seconds, expected) in [
        (0, "刚刚"),
        (MINUTE - 1, "刚刚"),
        (MINUTE, "1 分钟前"),
        (HOUR - 1, "59 分钟前"),
        (HOUR, "1 小时前"),
        (DAY - 1, "23 小时前"),
        (DAY, "1 天前"),
        (3 * DAY, "3 天前"),
    ] {
        assert_eq!(
            crate::i18n::visible_text(&recent_time(
                Timestamp::from_millis(now.millis() - seconds * 1_000),
                now
            )),
            expected,
        );
        assert_eq!(
            relative_age(Duration::from_secs(seconds)),
            recent_time(Timestamp::from_millis(now.millis() - seconds * 1_000), now)
        );
    }
    assert_eq!(
        recent_time(Timestamp::from_millis(now.millis() + HOUR * 1_000), now),
        "刚刚"
    );
}

#[test]
fn acp_times_respect_offsets_and_do_not_invent_missing_or_invalid_dates() {
    assert_eq!(
        parse_rfc3339("2026-10-16T18:00:00+08:00"),
        parse_rfc3339("2026-10-16T10:00:00Z")
    );
    assert_eq!(parse_rfc3339("not a time"), None);
    assert_eq!(parse_rfc3339(""), None);
    assert_eq!(parse_rfc3339("1960-01-01T00:00:00Z"), None);
    let now = parse_rfc3339("2026-10-16T10:05:00Z").expect("now");
    let then = parse_rfc3339("2026-10-16T10:00:00Z").expect("then");
    assert_eq!(
        crate::i18n::visible_text(&recent_time(then, now)),
        "5 分钟前"
    );
}
