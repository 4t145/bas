//! An independently-lived, in-memory frontend log viewer.

use super::log::{Entry, Level, Snapshot, Store};
use crate::{
    i18n::tr,
    theme,
    ui::{self, icon_action, window_controls::TitleBar},
};
use gds_uim::el;
use gpui_kit::{
    AnyElement, App, AppContext, Bounds, ClipboardItem, Context, Entity, FocusHandle, Global,
    KeyBinding, MouseButton, Render, Subscription, Task, TestSupportExt, UniformListScrollHandle,
    WeakEntity, Window, WindowBounds, WindowDecorations, WindowHandle, WindowKind, WindowOptions,
    actions,
    assets::IconName,
    component::{
        ActiveTheme, Root,
        input::{Input, InputEvent, InputState},
    },
    div,
    prelude::*,
    px, size, uniform_list,
};
use std::time::Duration;

mod filters;
mod navigation;

#[cfg(feature = "gui-tests")]
pub(crate) mod fixture;

#[cfg(all(test, feature = "gui-tests"))]
mod tests;

const WIDTH: f32 = 1_000.;
const HEIGHT: f32 = 720.;
const MIN_WIDTH: f32 = 780.;
const MIN_HEIGHT: f32 = 480.;
const PREVIEW_CHARS: usize = 240;
const ROW_HEIGHT: f32 = 22.;
const DETAIL_HEIGHT: f32 = 180.;
const UPDATE_INTERVAL: Duration = Duration::from_millis(50);

actions!(
    diagnostics,
    [
        OpenLogs,
        CloseLogs,
        PreviousLog,
        NextLog,
        PreviousLogPage,
        NextLogPage
    ]
);

struct Registered;
impl Global for Registered {}

struct OpenWindow {
    handle: WindowHandle<Root>,
    viewer: WeakEntity<Viewer>,
}
impl Global for OpenWindow {}

pub(crate) fn init(cx: &mut App) {
    if cx.has_global::<Registered>() {
        return;
    }
    cx.set_global(Registered);
    cx.bind_keys([
        KeyBinding::new(
            if cfg!(target_os = "macos") {
                "cmd-shift-l"
            } else {
                "ctrl-shift-l"
            },
            OpenLogs,
            None,
        ),
        KeyBinding::new(
            if cfg!(target_os = "macos") {
                "cmd-w"
            } else {
                "ctrl-w"
            },
            CloseLogs,
            Some("BasLogs"),
        ),
    ]);
    for context in ["BasLogsList", "BasLogs > Input"] {
        cx.bind_keys([
            KeyBinding::new("up", PreviousLog, Some(context)),
            KeyBinding::new("down", NextLog, Some(context)),
            KeyBinding::new("pageup", PreviousLogPage, Some(context)),
            KeyBinding::new("pagedown", NextLogPage, Some(context)),
        ]);
    }
    cx.on_action(|_: &OpenLogs, cx| {
        // Action dispatch may still hold the viewer (or a popup) borrowed.
        cx.defer(|cx| {
            if let Err(error) = open(cx) {
                super::notify(
                    None,
                    Level::Error,
                    tr!("logs-open-error"),
                    &error.to_string(),
                    cx,
                );
            }
        });
    });
}

/// # Errors
/// Returns the platform error if the native viewer window cannot be created.
pub(crate) fn open(cx: &mut App) -> anyhow::Result<WindowHandle<Root>> {
    if let Some(state) = cx.try_global::<OpenWindow>() {
        let handle = state.handle;
        let viewer = state.viewer.clone();
        if handle
            .update(cx, |_, window, cx| {
                let _ = viewer.update(cx, |viewer, cx| {
                    viewer
                        .search
                        .update(cx, |search, cx| search.focus(window, cx));
                });
                window.activate_window();
            })
            .is_ok()
        {
            return Ok(handle);
        }
    }
    let store = super::store(cx);
    let mut viewer = None;
    let handle = cx.open_window(
        WindowOptions {
            kind: WindowKind::Floating,
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(WIDTH), px(HEIGHT)),
                cx,
            ))),
            window_min_size: Some(size(px(MIN_WIDTH), px(MIN_HEIGHT))),
            window_decorations: Some(WindowDecorations::Client),
            window_background: cx.global::<theme::Appearance>().background,
            app_id: Some("org.bas.Gui".into()),
            ..TitleBar::window_options()
        },
        |window, cx| {
            window.set_window_title(tr!("logs-title"));
            let host = cx.new(|cx| Viewer::new(store, window, cx));
            viewer = Some(host.downgrade());
            host.read(cx)
                .search
                .clone()
                .update(cx, |search, cx| search.focus(window, cx));
            window.activate_window();
            cx.new(|cx| theme::window_root(host, window, cx))
        },
    )?;
    cx.set_global(OpenWindow {
        handle,
        viewer: viewer.expect("window creates its viewer"),
    });
    Ok(handle)
}

struct Viewer {
    store: Store,
    snapshot: Snapshot,
    visible: Vec<usize>,
    selected: Option<u64>,
    level: Option<Level>,
    target: Option<String>,
    query: String,
    scroll: UniformListScrollHandle,
    list_focus: FocusHandle,
    search: Entity<InputState>,
    _search: Subscription,
    _updates: Task<()>,
}

impl Viewer {
    fn new(store: Store, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Subscribe before the snapshot so a concurrent record cannot be missed.
        let mut changes = store.subscribe();
        let snapshot = store.snapshot();
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(tr!("logs-search")));
        let subscription = cx.subscribe(&search, |this, input, _: &InputEvent, cx| {
            this.query = input.read(cx).value().to_string().to_lowercase();
            this.filter();
            cx.notify();
        });
        let updates = cx.spawn(async move |view, cx| {
            while changes.changed().await.is_ok() {
                // Event-driven batching yields even when producers never become idle.
                cx.background_executor().timer(UPDATE_INTERVAL).await;
                changes.borrow_and_update();
                if view
                    .update(cx, |this, cx| {
                        this.refresh();
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        let mut viewer = Self {
            store,
            snapshot,
            visible: Vec::new(),
            selected: None,
            level: None,
            target: None,
            query: String::new(),
            scroll: UniformListScrollHandle::new(),
            list_focus: cx.focus_handle(),
            search,
            _search: subscription,
            _updates: updates,
        };
        viewer.filter();
        viewer.scroll.scroll_to_bottom();
        viewer
    }

    fn filter(&mut self) {
        self.visible = self
            .snapshot
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                self.level.is_none_or(|level| level == entry.level)
                    && self
                        .target
                        .as_ref()
                        .is_none_or(|target| target == &entry.target)
                    && (self.query.is_empty()
                        || entry.message.to_lowercase().contains(&self.query)
                        || entry.target.to_lowercase().contains(&self.query))
            })
            .map(|(index, _)| index)
            .collect();
        if !self
            .visible
            .iter()
            .any(|&index| Some(self.snapshot.entries[index].id) == self.selected)
        {
            self.selected = None;
        }
    }

    fn refresh(&mut self) {
        let snapshot = self.store.snapshot();
        if snapshot.revision != self.snapshot.revision {
            let follow = self.scroll.is_scrolled_to_end().unwrap_or(true);
            self.snapshot = snapshot;
            self.filter();
            if follow {
                self.scroll.scroll_to_bottom();
            }
        }
    }

    fn clear(&mut self) {
        self.store.clear();
        self.refresh();
    }

    fn selected_entry(&self) -> Option<&Entry> {
        self.snapshot
            .entries
            .iter()
            .find(|entry| Some(entry.id) == self.selected)
    }

    fn copy_selected(&self, cx: &mut App) {
        if let Some(entry) = self.selected_entry() {
            cx.write_to_clipboard(ClipboardItem::new_string(entry.text()));
        }
    }

    fn row(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let entry = &self.snapshot.entries[self.visible[index]];
        let id = entry.id;
        let at = time::OffsetDateTime::from(entry.time);
        let color = match entry.level {
            Level::Error => cx.theme().danger,
            Level::Warn => cx.theme().warning,
            _ => cx.theme().foreground,
        };
        el! {
            .div {
                # $("log-entry", id); test_support;
                h: ROW_HEIGHT.px; w_full; px_2; overflow_hidden; whitespace_nowrap;
                cursor_pointer; flex; items_center; gap_2;
                font_family: ~$(cx.theme().mono_font_family); text_size: 12.0.px;
                when: $self.selected == Some(id), { bg: $cx.theme().accent };
            } [
                .div { w: 100.0.px; flex_shrink_0; text_color: $cx.theme().muted_foreground; } [
                    format!(
                        "{:02}:{:02}:{:02}.{:03}Z",
                        at.hour(),
                        at.minute(),
                        at.second(),
                        at.millisecond(),
                    ),
                ],
                .div { w: 40.0.px; flex_shrink_0; text_color: color; } [$entry.level.as_str()],
                .div {
                    w: 200.0.px; flex_shrink_0; truncate; text_color: $cx.theme().muted_foreground;
                } [~$(entry.target)],
                .div { flex_1; min_w_0; truncate; } [$(&entry.message).preview],
            ] {
                on_click: $cx.listener(move |this, _, window, cx| {
                    this.list_focus.focus(window, cx);
                    this.selected = (this.selected != Some(id)).then_some(id);
                    cx.notify();
                });
                into_any_element;
            }
        }
    }
}

fn preview(text: &str) -> String {
    let mut chars = text.chars();
    let mut preview: String = chars
        .by_ref()
        .take(PREVIEW_CHARS)
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    if chars.next().is_some() {
        preview.push('…');
    }
    preview
}

impl Render for Viewer {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let detail = self.selected_entry().map(Entry::text);
        el! {
            .div {
                # "logs-window"; test_support; key_context: "BasLogs";
                size_full; flex; flex_col; bg: $cx.theme().background; text_color: $cx.theme().foreground;
                on_action: $cx.listener(|_, _: &CloseLogs, window, _| window.remove_window());
                on_action: $cx.listener(|this, _: &PreviousLog, _, cx| this.move_selection(false, false, cx));
                on_action: $cx.listener(|this, _: &NextLog, _, cx| this.move_selection(true, false, cx));
                on_action: $cx.listener(|this, _: &PreviousLogPage, _, cx| this.move_selection(false, true, cx));
                on_action: $cx.listener(|this, _: &NextLogPage, _, cx| this.move_selection(true, true, cx));
            } [
                .TitleBar::new {
                    on_close_window: $|_, window, _| window.remove_window();
                } [tr!("logs-title")],
                .div { flex; flex_col; flex_1; min_h_0; min_w_0; } [
                    .div { # "logs-toolbar"; test_support; flex; items_center; gap_2; p_2; flex_shrink_0; } [
                        $self.level_filter(cx),
                        $self.target_filter(cx),
                        .div { flex_1; min_w_0; } [
                            ($(&self.search), $(&self.search).Input::new { cleanable: true; }).ui::input::placeholder,
                        ],
                        ("logs-clear", IconName::ListX, tr!("logs-clear")).icon_action {
                            on_click: $cx.listener(|this, _, _, cx| { this.clear(); cx.notify(); });
                        },
                    ],
                    .div {
                        # "logs-list-viewport"; test_support;
                        key_context: "BasLogsList"; track_focus: $&self.list_focus;
                        flex_1; min_h_0;
                        on_mouse_down: MouseButton::Left,
                            $cx.listener(|this, _, window, cx| this.list_focus.focus(window, cx));
                    } [
                        (
                            "logs-list",
                            $self.visible.len(),
                            $cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                                range.map(|index| this.row(index, cx)).collect()
                            }),
                        ).uniform_list { track_scroll: $&self.scroll; size_full; },
                    ],
                    @if (let Some(text) = detail)
                        .div {
                            # "logs-detail"; test_support; w_full; min_w_0; max_h: DETAIL_HEIGHT.px;
                            flex_shrink_0; flex; gap_2; p_2; border_t_1; border_color: $cx.theme().border;
                        } [
                            .div {
                                # "logs-detail-text"; flex_1; min_w_0; max_h: $(DETAIL_HEIGHT - 16.).px;
                                overflow_y_scroll; font_family: ~$(cx.theme().mono_font_family);
                                text_size: 12.0.px; whitespace_normal;
                            } [$(&text).ui::tooltip::wrap_tokens],
                            ("logs-copy-selected", IconName::Copy, tr!("logs-copy-selected")).icon_action {
                                on_click: $cx.listener(|this, _, _, cx| this.copy_selected(cx));
                            },
                            ("logs-close-detail", IconName::Close, tr!("logs-close-detail")).icon_action {
                                on_click: $cx.listener(|this, _, _, cx| { this.selected = None; cx.notify(); });
                            },
                        ],
                    @end,
                    .div { px_2; text_xs; text_color: $cx.theme().muted_foreground; } [
                        tr!("logs-count", visible = self.visible.len(), retained = self.snapshot.entries.len(), evicted = self.snapshot.evicted),
                    ],
                ],
            ]
        }
    }
}
