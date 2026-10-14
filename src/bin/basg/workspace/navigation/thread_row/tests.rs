use bas::model::{Cursor, Owner};

use super::*;

#[test]
fn recent_time_uses_compact_units_and_handles_clock_skew() {
    let now = Timestamp::from_millis(7 * DAY_MILLIS);
    for (elapsed, expected) in [
        (0, "刚刚"),
        (MINUTE_MILLIS - 1, "刚刚"),
        (MINUTE_MILLIS, "1 分钟"),
        (HOUR_MILLIS - 1, "59 分钟"),
        (HOUR_MILLIS, "1 小时"),
        (DAY_MILLIS - 1, "23 小时"),
        (DAY_MILLIS, "1 天"),
        (3 * DAY_MILLIS, "3 天"),
    ] {
        assert_eq!(
            crate::i18n::visible_text(&recent_time(
                Timestamp::from_millis(now.millis() - elapsed),
                now
            )),
            expected
        );
    }
    assert_eq!(
        recent_time(Timestamp::from_millis(now.millis() + HOUR_MILLIS), now),
        "刚刚"
    );
}

#[test]
fn rows_status_uses_reply_cursor_not_catalog_revision() {
    let mut session = SessionMeta::new(SessionId::new(), "embedded".into(), Owner::current());
    session.status = SessionStatus::Idle;
    assert_eq!(RowStatus::for_session(&session, 0, false), RowStatus::Idle);
    session.last_reply = Cursor::from_raw(9);
    assert_eq!(
        RowStatus::for_session(&session, 8, false),
        RowStatus::Unread
    );
    assert_eq!(RowStatus::for_session(&session, 9, false), RowStatus::Idle);
    session.status = SessionStatus::Working;
    assert_eq!(
        RowStatus::for_session(&session, 0, true),
        RowStatus::Working
    );
    session.status = SessionStatus::Starting;
    assert_eq!(
        RowStatus::for_session(&session, 0, true),
        RowStatus::Starting
    );
    session.status = SessionStatus::Failed;
    assert_eq!(
        RowStatus::for_session(&session, 0, false),
        RowStatus::Failed
    );
    session.status = SessionStatus::Stopped;
    assert_eq!(
        RowStatus::for_session(&session, 0, false),
        RowStatus::Stopped
    );
    session.status = SessionStatus::Idle;
    session.last_reply = Cursor::from_raw(0);
    assert_eq!(RowStatus::for_session(&session, 0, true), RowStatus::Unread);
}

#[cfg(feature = "gui-tests")]
mod animation {
    use super::*;
    use gpui_kit::{Render, TestAppContext, size, test::TestWindowExt};

    struct StatusHost(RowStatus, SessionId);

    impl Render for StatusHost {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            self.0.slot(self.1, cx)
        }
    }

    #[gpui_kit::test]
    fn rows_status_spinner_requests_successive_frames_only_while_running(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let window = cx.open_window(size(px(40.), px(40.)), |_, _| {
            StatusHost(RowStatus::Starting, SessionId::new())
        });
        // Spinner uses GPUI's repeating animation. Drive a bounded number of frames:
        // draining an executor until idle is not a contract for a repeating animation.
        for state in [RowStatus::Starting, RowStatus::Working] {
            window
                .update(cx, |host, _, cx| {
                    host.0 = state;
                    cx.notify();
                })
                .expect("set running status");
            cx.update_window(window.into(), |_, window, cx| {
                for _ in 0..3 {
                    window.render_frame(cx);
                    assert!(window.simulate_next_frame(cx) > 0);
                }
            })
            .expect("running status requests animation frames");
        }
        for state in [
            RowStatus::Unread,
            RowStatus::Idle,
            RowStatus::Failed,
            RowStatus::Stopped,
        ] {
            window
                .update(cx, |host, _, cx| {
                    host.0 = state;
                    cx.notify();
                })
                .expect("set static status");
            cx.update_window(window.into(), |_, window, cx| {
                window.render_frame(cx);
                window.simulate_next_frame(cx);
                window.render_frame(cx);
                assert_eq!(window.simulate_next_frame(cx), 0);
            })
            .expect("static status stops requesting frames");
        }
        cx.update(|cx| cx.set_reduce_motion(true));
        window
            .update(cx, |host, _, cx| {
                host.0 = RowStatus::Working;
                cx.notify();
            })
            .expect("set reduced-motion status");
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(window.simulate_next_frame(cx), 0);
        })
        .expect("spinner respects reduced motion");
    }
}
