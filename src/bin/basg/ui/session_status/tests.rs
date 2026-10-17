use bas::model::{Cursor, Owner, SessionId};

use super::*;

#[test]
fn status_uses_reply_cursor_not_catalog_revision() {
    let mut session = SessionMeta::new(SessionId::new(), "embedded".into(), Owner::current());
    session.status = SessionStatus::Idle;
    assert_eq!(Status::for_session(&session, 0, false), Status::Idle);
    session.last_reply = Cursor::from_raw(9);
    assert_eq!(Status::for_session(&session, 8, false), Status::Unread);
    assert_eq!(Status::for_session(&session, 9, false), Status::Idle);
    for (state, expected) in [
        (SessionStatus::Working, Status::Working),
        (SessionStatus::Starting, Status::Starting),
        (SessionStatus::Failed, Status::Failed),
        (SessionStatus::Stopped, Status::Stopped),
    ] {
        session.status = state;
        assert_eq!(Status::for_session(&session, 0, true), expected);
        assert_eq!(Status::for_snapshot(&session.clone().into()), expected);
    }
    session.status = SessionStatus::Idle;
    session.last_reply = Cursor::from_raw(0);
    assert_eq!(Status::for_session(&session, 0, true), Status::Unread);
    assert_eq!(Status::for_snapshot(&session.into()), Status::Idle);
}

#[test]
fn runtime_failure_uses_the_snapshots_observable_status() {
    let mut meta = SessionMeta::new(SessionId::new(), "embedded".into(), Owner::current());
    meta.status = SessionStatus::Idle;
    let mut snapshot = SessionSnapshot::from(meta);
    snapshot.runtime.failure = Some(bas::model::RuntimeFailure::Persistence {
        message: "storage unavailable".into(),
    });
    assert_eq!(snapshot.meta.status, SessionStatus::Idle);
    assert_eq!(Status::for_snapshot(&snapshot), Status::Failed);
}

#[cfg(feature = "gui-tests")]
mod animation {
    use super::*;
    use gpui_kit::{Context, Render, TestAppContext, Window, size, test::TestWindowExt};

    struct StatusHost(Status);

    impl Render for StatusHost {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            self.0.render("status", "indicator", cx)
        }
    }

    #[gpui_kit::test]
    fn spinner_requests_successive_frames_only_while_running(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let window = cx.open_window(size(px(40.), px(40.)), |_, _| StatusHost(Status::Starting));
        // Repeating animations are tested with bounded frames, not executor-idle waits.
        for state in [Status::Starting, Status::Working] {
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
                assert_eq!(window.find("status").role(), Some(gpui_kit::Role::Status));
                assert_eq!(
                    window.find("indicator").bounds().size,
                    size(px(12.), px(12.))
                );
            })
            .expect("running status requests animation frames");
        }
        for state in [
            Status::Unread,
            Status::Idle,
            Status::Failed,
            Status::Stopped,
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
                if state == Status::Idle {
                    assert!(window.try_find("status").is_none());
                    assert_eq!(window.find("indicator").bounds().size, size(px(0.), px(0.)));
                } else {
                    assert_eq!(window.find("status").role(), Some(gpui_kit::Role::Status));
                }
                if matches!(state, Status::Stopped | Status::Unread) {
                    let bounds = window.find("indicator").bounds();
                    assert_eq!(bounds.size, size(px(6.), px(6.)));
                    let color = match state {
                        Status::Stopped => cx.theme().muted_foreground,
                        _ => cx.theme().primary,
                    };
                    let painted = bounds.scale(window.scale_factor());
                    assert!(
                        window.painted_quads().iter().any(|quad| {
                            quad.bounds == painted && quad.background == color.into()
                        }),
                        "light color matches its state"
                    );
                }
            })
            .expect("static status stops requesting frames");
        }
        cx.update(|cx| cx.set_reduce_motion(true));
        window
            .update(cx, |host, _, cx| {
                host.0 = Status::Working;
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
