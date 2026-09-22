use super::*;
use gpui_kit::{TestAppContext, test::TestWindowExt};

fn init(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::init_default(cx);
        super::init(cx);
    });
}

#[gpui_kit::test]
fn about_reuses_a_window_and_opens_homepage(cx: &mut TestAppContext) {
    init(cx);
    let handle = cx.update(|cx| open_about(cx).expect("about window"));
    let again = cx.update(|cx| open_about(cx).expect("focus existing window"));
    assert_eq!(handle.window_id(), again.window_id());
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("about-window").visible());
        window.click("about-home", cx);
    })
    .expect("about content");
    assert_eq!(cx.opened_url().as_deref(), Some(HOME_URL));
    cx.update_window(handle.into(), |_, window, cx| {
        window.click("about-close", cx);
    })
    .expect("close about");
    assert!(cx.update_window(handle.into(), |_, _, _| ()).is_err());
    let reopened = cx.update(|cx| open_about(cx).expect("reopen after closing"));
    assert_ne!(handle.window_id(), reopened.window_id());
}

#[gpui_kit::test]
fn releases_opens_only_the_manual_release_page(cx: &mut TestAppContext) {
    init(cx);
    cx.update(open_releases);
    assert_eq!(cx.opened_url().as_deref(), Some(RELEASES_URL));
}

#[gpui_kit::test]
fn quit_can_be_cancelled_and_reopened_without_a_workspace(cx: &mut TestAppContext) {
    init(cx);
    let about = cx.update(|cx| open_about(cx).expect("independent about"));
    let quit = cx.update(|cx| open_quit(cx).expect("confirmation"));
    let again = cx.update(|cx| open_quit(cx).expect("existing confirmation"));
    assert_eq!(quit.window_id(), again.window_id());
    cx.update_window(quit.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("quit-cancel", cx);
    })
    .expect("cancel");
    assert!(cx.update_window(quit.into(), |_, _, _| ()).is_err());
    cx.update_window(about.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("about-window").visible());
    })
    .expect("cancel leaves other windows open");
    let quit = cx.update(|cx| open_quit(cx).expect("new confirmation"));
    cx.update_window(quit.into(), |_, window, cx| {
        window.render_frame(cx);
        // TestPlatform::quit is deliberately a no-op; exercise the real button
        // without relying on it terminating the test runner.
        window.click("quit-confirm", cx);
    })
    .expect("confirm is usable without a workspace or Connection");
}

fn request() -> Confirmation {
    Confirmation {
        title: "永久删除线程？".into(),
        message: "此操作不可撤销。".into(),
        confirm_label: "永久删除".into(),
    }
}

#[gpui_kit::test]
fn messages_do_not_replace_confirmations_and_confirmation_runs_once(cx: &mut TestAppContext) {
    use std::{cell::Cell, rc::Rc};
    init(cx);
    let calls = Rc::new(Cell::new(0));
    let callback_calls = calls.clone();
    let confirmation = cx.update(|cx| {
        open_confirmation(
            request(),
            move |_| {
                callback_calls.set(callback_calls.get() + 1);
            },
            cx,
        )
        .expect("confirmation")
    });
    let again = cx.update(|cx| {
        open_confirmation(
            request(),
            |_| panic!("new request must not replace pending request"),
            cx,
        )
        .expect("focus pending confirmation")
    });
    assert_eq!(confirmation.window_id(), again.window_id());
    let quit = cx.update(|cx| open_quit(cx).expect("separate quit"));
    let message =
        cx.update(|cx| open_message("归档失败", "请稍后重试", cx).expect("error message"));
    let replacement =
        cx.update(|cx| open_message("删除失败", "连接已断开", cx).expect("replace only error"));
    assert_ne!(message.window_id(), replacement.window_id());
    assert!(cx.update_window(message.into(), |_, _, _| ()).is_err());
    assert_eq!(calls.get(), 0);
    cx.update_window(confirmation.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("message-confirm", cx);
    })
    .expect("confirm original request");
    assert_eq!(calls.get(), 1);
    assert!(cx.update_window(confirmation.into(), |_, _, _| ()).is_err());
    cx.update_window(quit.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("quit-window").visible());
    })
    .expect("unrelated quit is intact");
}

#[gpui_kit::test]
fn cancellation_drops_confirmation_callback(cx: &mut TestAppContext) {
    init(cx);
    let confirmation = cx.update(|cx| {
        open_confirmation(
            request(),
            |_| panic!("cancel must not execute callback"),
            cx,
        )
        .expect("confirmation")
    });
    cx.update_window(confirmation.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("message-close", cx);
    })
    .expect("cancel");
    assert!(cx.update_window(confirmation.into(), |_, _, _| ()).is_err());
}

#[gpui_kit::test]
fn confirm_quit_flushes_valid_settings_before_debounce_without_a_workspace(
    cx: &mut TestAppContext,
) {
    let directory = tempfile::tempdir().expect("temporary appearance");
    let path = directory.path().join("settings.json");
    std::fs::write(&path, r#"{"appearance":{"theme_source":"builtin"}}"#).expect("test config");
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::init(None, Some(path.clone()), cx);
    });
    let settings = cx.update(|cx| settings_window::open(cx).expect("settings"));
    cx.update_window(settings.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("settings-fonts", cx);
        window.render_frame(cx);
        window.click("appearance-ui-size", cx);
        window.input("27", cx);
    })
    .expect("edit before debounce");
    let quit = cx.update(|cx| open_quit(cx).expect("quit"));
    cx.update_window(quit.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("quit-confirm", cx);
    })
    .expect("flush then quit");
    let saved: basr::config::Settings =
        serde_json::from_slice(&std::fs::read(&path).expect("saved settings"))
            .expect("preferences");
    assert_eq!(saved.appearance.ui_font_size, Some(27.));
}
