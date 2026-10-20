use super::*;
use crate::theme;
use gpui_kit::{
    Context, FocusHandle, Render, TestAppContext, Window, WindowHandle, div, prelude::*, size,
    test::TestWindowExt,
};
use std::time::Duration;

struct Probe(FocusHandle);

impl Render for Probe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .track_focus(&self.0)
            .child("origin content")
    }
}

fn setup(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::init_default(cx);
        // Layout assertions target the settled card. One-shot entrance animations
        // use wall time rather than the test executor's clock.
        cx.set_reduce_motion(true);
        init(Store::new(), cx);
    });
}

fn open(cx: &mut TestAppContext, height: f32) -> WindowHandle<Root> {
    cx.open_window(size(px(640.), px(height)), |window, cx| {
        let view = cx.new(|cx| Probe(cx.focus_handle()));
        let focus = view.read(cx).0.clone();
        focus.focus(window, cx);
        theme::window_root(view, window, cx)
    })
}

fn settle(cx: &mut TestAppContext) {
    cx.run_until_parked();
    // Finish the toast entrance without reaching its five-second timeout.
    cx.executor().advance_clock(Duration::from_millis(500));
    cx.run_until_parked();
}

fn notice_id(level: Level, title: &str, message: &str) -> String {
    let mut hash = DefaultHasher::new();
    (level.as_str(), title, message).hash(&mut hash);
    format!("notice-{:016x}", hash.finish())
}

fn count(handle: WindowHandle<Root>, cx: &mut TestAppContext) -> usize {
    cx.update(|cx| {
        handle
            .update(cx, |root, _, cx| {
                root.notification.read(cx).notifications().len()
            })
            .expect("notification root")
    })
}

#[gpui_kit::test]
fn severities_stay_in_origin_top_right_without_windows_or_focus_changes(cx: &mut TestAppContext) {
    setup(cx);
    let origin = open(cx, 480.);
    let other = open(cx, 480.);
    let focus = cx
        .update_window(origin.into(), |_, window, cx| window.focused(cx))
        .unwrap();
    let active = cx.update(|cx| cx.active_window().map(|window| window.window_id()));
    for level in [Level::Info, Level::Warn, Level::Error] {
        cx.update(|cx| notify(Some(origin.into()), level, "Notice", "details", cx));
        settle(cx);
        let id = notice_id(level, "Notice", "details");
        cx.update_window(origin.into(), |_, window, cx| {
            window.render_frame(cx);
            let notice = window.find(id);
            assert!(notice.visible());
            assert!(notice.bounds().center().x > px(320.));
            assert!(notice.bounds().center().y < px(240.));
            assert_eq!(window.focused(cx), focus);
        })
        .unwrap();
        assert_eq!(count(other, cx), 0);
        cx.update(|cx| {
            assert_eq!(cx.windows().len(), 2);
            assert_eq!(cx.active_window().map(|window| window.window_id()), active);
        });
    }
    assert_eq!(count(origin, cx), 3);
    cx.update(|cx| {
        let entries = store(cx).snapshot().entries;
        assert_eq!(
            entries.iter().map(|entry| entry.level).collect::<Vec<_>>(),
            vec![Level::Info, Level::Warn, Level::Error]
        );
    });
}

#[gpui_kit::test]
fn duplicates_replace_toast_but_retain_every_log_and_exact_copy(cx: &mut TestAppContext) {
    setup(cx);
    let origin = open(cx, 480.);
    let title = "Failed  ";
    let message = "first  \r\n\tsecond\nlast  \n  ";
    let expected = format!("{title}\n{message}");
    let id = notice_id(Level::Error, title, message);
    for _ in 0..2 {
        cx.update(|cx| notify(Some(origin.into()), Level::Error, title, message, cx));
        settle(cx);
        assert_eq!(count(origin, cx), 1);
        cx.update_window(origin.into(), |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(window.find(id.clone()).label(), Some(expected.as_str()));
            window.click(format!("{id}-copy"), cx);
            assert_eq!(
                cx.read_from_clipboard().and_then(|item| item.text()),
                Some(expected.clone())
            );
        })
        .unwrap();
    }
    cx.update(|cx| {
        let entries = store(cx).snapshot().entries;
        assert_eq!(entries.len(), 2);
        assert_ne!(entries[0].id, entries[1].id);
        for entry in entries {
            // The diagnostic log intentionally strips CR controls; clipboard does not.
            assert_eq!(entry.message, expected.replace('\r', ""));
        }
    });
}

#[gpui_kit::test]
fn closed_explicit_origin_does_not_fall_back_to_another_window(cx: &mut TestAppContext) {
    setup(cx);
    let origin = open(cx, 480.);
    let other = open(cx, 480.);
    cx.update_window(origin.into(), |_, window, _| window.remove_window())
        .unwrap();
    cx.run_until_parked();
    cx.update_window(other.into(), |_, window, _| window.activate_window())
        .unwrap();
    cx.update(|cx| {
        notify(
            Some(origin.into()),
            Level::Warn,
            "Closed",
            "still logged",
            cx,
        )
    });
    settle(cx);
    assert_eq!(count(other, cx), 0);
    cx.update(|cx| {
        assert_eq!(cx.windows().len(), 1);
        let entries = store(cx).snapshot().entries;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].message, "Closed\nstill logged");
    });
}

#[gpui_kit::test]
fn compact_640_by_98_toast_keeps_copy_inside_viewport(cx: &mut TestAppContext) {
    setup(cx);
    let origin = open(cx, 98.);
    let message = format!("first  \n{}  \n  ", "long details ".repeat(50));
    let expected = format!("Notice\n{message}");
    cx.update(|cx| notify(Some(origin.into()), Level::Info, "Notice", &message, cx));
    settle(cx);
    let id = notice_id(Level::Info, "Notice", &message);
    cx.update_window(origin.into(), |_, window, cx| {
        window.render_frame(cx);
        for target in [id.clone(), format!("{id}-copy")] {
            let target = window.find(target);
            assert!(
                target.visible(),
                "compact target bounds: {:?}; viewport: {:?}",
                target.bounds(),
                window.viewport_size()
            );
            let bounds = target.bounds();
            assert!(bounds.left() >= px(0.) && bounds.top() >= px(0.));
            assert!(bounds.right() <= px(640.) && bounds.bottom() <= px(98.));
        }
        window.click(format!("{id}-copy"), cx);
        assert_eq!(
            cx.read_from_clipboard().and_then(|item| item.text()),
            Some(expected)
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn burst_is_bounded_and_smallest_window_keeps_copy_visible(cx: &mut TestAppContext) {
    setup(cx);
    let origin = open(cx, 480.);
    let small = cx.open_window(size(px(420.), px(80.)), |window, cx| {
        let view = cx.new(|cx| Probe(cx.focus_handle()));
        theme::window_root(view, window, cx)
    });
    cx.update(|cx| {
        for index in 0..20 {
            for window in [origin, small] {
                notify(
                    Some(window.into()),
                    Level::Error,
                    "Error",
                    &index.to_string(),
                    cx,
                );
            }
        }
    });
    settle(cx);
    assert_eq!(count(origin, cx), MAX_NOTICES);
    assert_eq!(count(small, cx), 1);
    cx.update(|cx| assert_eq!(store(cx).snapshot().entries.len(), 40));
    cx.update_window(small.into(), |_, window, cx| {
        window.render_frame(cx);
        let id = notice_id(Level::Error, "Error", "0");
        let copy = window.find(format!("{id}-copy"));
        assert!(
            copy.visible(),
            "small copy bounds: {:?}; viewport: {:?}",
            copy.bounds(),
            window.viewport_size()
        );
        assert!(copy.bounds().right() <= px(420.));
        assert!(copy.bounds().bottom() <= px(80.));
        window.click(format!("{id}-copy"), cx);
        assert_eq!(
            cx.read_from_clipboard().and_then(|item| item.text()),
            Some("Error\n0".into())
        );
    })
    .unwrap();
}
