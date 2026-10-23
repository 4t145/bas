//! User notices and the current process's frontend diagnostic log.

pub(crate) mod log;
mod surface;
pub(crate) mod window;

#[cfg(all(test, feature = "gui-tests"))]
mod tests;

pub(crate) use surface::{content, wrap};
pub(crate) use window::OpenLogs;

use crate::ui::{copy_button, tooltip};
use gds_uim::el;
use gpui_kit::{
    Anchor, AnyWindowHandle, App, FontWeight, Global, IntoElement, Role, TestSupportExt,
    component::{
        ActiveTheme, Root,
        notification::{Notification, NotificationDelivery, NotificationType},
    },
    div,
    prelude::*,
    px,
};
use log::{Level, Store};
use std::hash::{DefaultHasher, Hash, Hasher};
use tracing_subscriber::{EnvFilter, prelude::*};

struct Shared(Store);
impl Global for Shared {}

/// Install the same collector used by the Debug window before starting frontend tasks.
pub(crate) fn init_tracing() -> Store {
    let store = Store::new();
    let console = EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into());
    let memory = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "warn,basg=debug,bas::service::client::history=debug".into());
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_filter(console))
        .with(store.layer().with_filter(memory))
        .init();
    store
}

pub(crate) fn init(store: Store, cx: &mut App) {
    cx.set_global(Shared(store));
}

pub(crate) fn store(cx: &mut App) -> Store {
    if !cx.has_global::<Shared>() {
        init(Store::new(), cx);
    }
    cx.global::<Shared>().0.clone()
}

pub(crate) fn record(level: Level, title: &str, message: &str, cx: &mut App) {
    store(cx).record(level, "basg::ui", text(title, message));
}

fn text(title: &str, message: &str) -> String {
    if title.is_empty() {
        message.to_owned()
    } else {
        format!("{title}\n{message}")
    }
}

struct Notice;

const MAX_NOTICES: usize = 4;
const COMPACT_HEIGHT: f32 = 160.;

fn compact_window(window: &gpui_kit::Window) -> bool {
    window.viewport_size().height < px(COMPACT_HEIGHT)
}

/// Record user feedback and show it in its originating window without creating
/// a native window or moving focus. A closed window leaves the log entry intact.
pub(crate) fn notify(
    parent: Option<AnyWindowHandle>,
    level: Level,
    title: &str,
    message: &str,
    cx: &mut App,
) {
    record(level, title, message, cx);
    let Some(handle) = parent.or_else(|| cx.active_window()) else {
        return;
    };
    let Some(handle) = handle.downcast::<Root>() else {
        return;
    };
    let title = title.to_owned();
    let message = message.to_owned();
    let mut hash = DefaultHasher::new();
    (level.as_str(), &title, &message).hash(&mut hash);
    let key = format!("{:016x}", hash.finish());
    // Callers often still hold the originating window's update borrow.
    cx.defer(move |cx| {
        let _ = handle.update(cx, |root, window, cx| {
            let copy = text(&title, &message);
            let id = format!("notice-{key}");
            let compact_window = compact_window(window);
            let notice = Notification::new()
                .id1::<Notice>(key.clone())
                .with_type(match level {
                    Level::Warn => NotificationType::Warning,
                    Level::Error => NotificationType::Error,
                    _ => NotificationType::Info,
                })
                .placement(Anchor::TopRight)
                .delivery(NotificationDelivery::InApp)
                .autohide(true)
                .content(move |_, _, cx| {
                    let body = if compact_window {
                        copy.clone()
                    } else {
                        message.clone()
                    };
                    let body_id = id.clone();
                    let copy_id = format!("{id}-copy");
                    let hint = copy.clone();
                    el! {
                        .div {
                            # ~id; test_support; role: Role::Status;
                            aria_label: ~copy; flex; flex_col; gap_1; min_w_0;
                        } [
                            @if (!compact_window)
                                .div {
                                    min_w_0; truncate; font_weight: FontWeight::SEMIBOLD;
                                } [~title],
                            @end,
                            .div { flex; items_center; gap_2; min_w_0; } [
                                .div {
                                    # format!("{body_id}-message"); test_support;
                                    flex_1; min_w_0; truncate; text_color: $cx.theme().foreground;
                                    hoverable_tooltip: $move |window, cx| {
                                        tooltip::view(hint.clone(), window, cx)
                                    };
                                } [$body.replace('\n', " ")],
                                (copy_id, ~copy).copy_button { flex_shrink_0; },
                            ],
                        ] { into_any_element; }
                    }
                });
            let pending = root.notification.read(cx).notifications();
            let limit = if compact_window { 1 } else { MAX_NOTICES };
            if pending.len() < limit {
                root.push_notification(notice, window, cx);
            }
        });
    });
}
