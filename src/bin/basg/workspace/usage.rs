//! Context usage in the composer, with a hover preview and live click details.

mod format;

use bas::acp_v2 as acp;
use gpui_kit::component::{
    menu::{DropdownMenu, PopupMenu, PopupMenuItem},
    progress::{Progress, ProgressCircle},
};
use std::rc::Rc;

use super::*;

const RING_SIZE: f32 = 16.;
const WARNING_RATIO: f64 = 0.85;
const DETAILS_WIDTH: f32 = 280.;
const DETAILS_ROWS: usize = 7;

fn ratio(usage: &acp::UsageUpdate) -> Option<f64> {
    (usage.size > 0).then(|| usage.used as f64 / usage.size as f64)
}

fn summary(usage: &acp::UsageUpdate) -> String {
    match ratio(usage) {
        Some(ratio) => format!(
            "{:.0}% · {} / {}",
            (ratio * 100.).round(),
            format::short_tokens(usage.used),
            format::short_tokens(usage.size),
        ),
        None => crate::i18n::tr!(
            "usage-unknown",
            used = (format::short_tokens(usage.used)).to_string()
        ),
    }
}

fn label(id: &'static str, text: String) -> impl IntoElement {
    el! { .div {
        # id; test_support; role: gpui_kit::Role::Label; aria_label: ~text;
    } [text] }
}

fn on_details_mouse_down(_: &gpui_kit::MouseDownEvent, window: &mut Window, cx: &mut App) {
    window.prevent_default();
    cx.stop_propagation();
}

fn details(usage: &acp::UsageUpdate, cx: &App) -> AnyElement {
    let fraction = ratio(usage);
    let remaining = usage.size.saturating_sub(usage.used);
    let used = label(
        "usage-used",
        crate::i18n::tr!(
            "usage-used",
            used = (format::exact_tokens(usage.used)).to_string()
        ),
    );
    let capacity = label(
        "usage-capacity",
        match ratio(usage) {
            Some(_) => crate::i18n::tr!(
                "usage-capacity",
                capacity = (format::exact_tokens(usage.size)).to_string()
            ),
            None => crate::i18n::tr!("usage-unknown-capacity").into(),
        },
    );
    let remaining = (usage.size > 0).then(|| {
        label(
            "usage-remaining",
            crate::i18n::tr!(
                "usage-remaining",
                remaining = (format::exact_tokens(remaining)).to_string()
            ),
        )
    });
    let cost = usage.cost.as_ref().map(|cost| {
        label(
            "usage-cost",
            crate::ui::tooltip::wrap_tokens(&crate::i18n::tr!(
                "usage-cost",
                cost = (format::cost_label(cost)).to_string()
            )),
        )
    });
    el! {
        .div {
            # "usage-details"; test_support;
            on_mouse_down: gpui_kit::MouseButton::Left, on_details_mouse_down;
            text_color: $cx.theme().popover_foreground;
            w: DETAILS_WIDTH.px; max_w_full; min_w_0; flex; flex_col; gap_2; py_1;
        } [
            .div {
                text_size: ui::SMALL; text_color: $cx.theme().muted_foreground;
            } [tr!("usage-title")],
            .div {
                # "usage-summary"; test_support; role: gpui_kit::Role::Label;
                aria_label: usage.summary; flex; flex_col; gap_2;
            } [
                .div { flex; items_center; justify_between; gap_2; } [
                    .div { font_weight: FontWeight::SEMIBOLD; } [used],
                    @if (let Some(ratio) = fraction)
                        .div { text_size: ui::SMALL; text_color: $cx.theme().muted_foreground; } [
                            ("usage-percentage", format!("{:.0}%", (ratio * 100.).round())).label,
                        ],
                    @end,
                ],
                @if (let Some(ratio) = fraction)
                    .div { # "usage-progress"; test_support; w_full; } [
                        "usage-progress-indicator".Progress::new {
                            small; value: $((ratio * 100.) as f32);
                            color: @if (ratio >= WARNING_RATIO) $cx.theme().warning @else $cx.theme().primary @end;
                            accessibility_label: usage.summary;
                        },
                    ],
                @end,
            ],
            .div {
                # "usage-secondary"; test_support; flex; flex_col; gap_1;
                text_size: ui::SMALL; text_color: $cx.theme().muted_foreground;
            } [capacity, ..remaining],
            @if (let Some(cost) = cost)
                .div { pt_2; border_t_1; border_color: $cx.theme().border;
                    text_size: ui::SMALL; text_color: $cx.theme().muted_foreground;
                } [cost],
            @end,
        ]
    }
    .into_any_element()
}

struct DetailsSnapshot {
    owner: gpui_kit::WeakEntity<Workspace>,
    session: SessionId,
    compact: bool,
    usage: RefCell<Option<acp::UsageUpdate>>,
}

impl DetailsSnapshot {
    fn refresh(&self, view: Entity<Workspace>, cx: &mut Context<PopupMenu>) {
        let workspace = view.read(cx);
        *self.usage.borrow_mut() = workspace
            .selected
            .timeline
            .conversation
            .as_ref()
            .filter(|conversation| {
                conversation.session.id == self.session && workspace.window.compact == self.compact
            })
            .and_then(|conversation| conversation.usage.clone());
        if self.usage.borrow().is_none() {
            cx.emit(gpui_kit::DismissEvent);
        }
        cx.notify();
    }

    fn render(&self, cx: &App) -> AnyElement {
        el! {
            @match (self.usage.borrow().as_ref())
            @case (Some(usage))
                (usage, cx).details
            @case (None)
                .div { into_any_element; }
            @end
        }
    }

    fn build(self: &Rc<Self>, menu: PopupMenu, cx: &mut Context<PopupMenu>) -> PopupMenu {
        if let Some(view) = self.owner.upgrade() {
            let snapshot = self.clone();
            cx.observe(&view, move |_, view, cx| snapshot.refresh(view, cx))
                .detach();
        }
        let snapshot = self.clone();
        el! {
            menu {
                item: (move (_, cx) => $snapshot.render(cx)).PopupMenuItem::element { disabled: true; };
            }
        }
    }
}

impl Workspace {
    pub(super) fn render_usage(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let conversation = self.selected.timeline.conversation.as_ref()?;
        let usage = conversation.usage.as_ref()?;
        let session = conversation.session.id;
        let fraction = ratio(usage);
        let color = if fraction.is_some_and(|ratio| ratio >= WARNING_RATIO) {
            cx.theme().warning
        } else {
            cx.theme().muted_foreground
        };
        let name = crate::i18n::tr!(
            "usage-accessibility",
            summary = (summary(usage)).to_string()
        );
        let button = el! {
            format!("context-usage-{session}").SharedString::from.Button::new {
                ghost; xsmall; w: layout::ICON_BUTTON_SIZE.px; h: layout::ICON_BUTTON_SIZE.px;
                p_0; flex_shrink_0; accessibility_label: ~name;
            } [
                .div { # "usage-ring"; test_support; size: RING_SIZE.px; } [
                    format!("context-ring-{session}").SharedString::from.ProgressCircle::new {
                        value: $((fraction.unwrap_or(0.) * 100.) as f32);
                        color: color; accessibility_label: name; size: RING_SIZE.px;
                    }
                ]
            ]
        };
        let owner = cx.entity().downgrade();
        // Native popup creation paints synchronously while Workspace is borrowed.
        // Render from a detached snapshot, updated after Workspace notifications.
        let snapshot = Rc::new(DetailsSnapshot {
            owner: owner.clone(),
            session,
            compact: self.window.compact,
            usage: RefCell::new(Some(usage.clone())),
        });
        let build = move |menu: PopupMenu, _: &mut Window, cx: &mut Context<PopupMenu>| {
            snapshot.build(menu, cx)
        };
        let menu = if self.window.compact {
            self.menu_button(button, DETAILS_ROWS, false, build, cx)
        } else {
            el! { button {
                dropdown_menu: move (menu, window, cx) => (menu, window, cx).build.ui::menu;
                anchor: gpui_kit::Anchor::BottomLeft;
                on_open_change: $move |open, _, cx| {
                    let _ = owner.update(cx, |this, cx| {
                        this.on_usage_open_change(session, *open, cx);
                    });
                };
            } }
            .into_any_element()
        };
        let hint = crate::i18n::tr!("usage-accessibility", summary = summary(usage));
        let details_open = self.window.usage_details == Some(session)
            || (self.window.compact && self.window.menu.is_some());
        Some(
            el! {
                .div {
                    # "session-usage"; test_support; flex_shrink_0;
                    @if (!details_open)
                        hoverable_tooltip: move (window, cx) => ("usage-hint", ~hint, window, cx).ui::tooltip::view_with_id;
                    @end;
                } [menu]
            }
            .into_any_element(),
        )
    }

    fn on_usage_open_change(&mut self, session: SessionId, open: bool, cx: &mut Context<Self>) {
        if open && self.selected.id == Some(session) {
            self.window.usage_details = Some(session);
        } else if !open && self.window.usage_details == Some(session) {
            self.window.usage_details = None;
        }
        cx.notify();
    }
}
