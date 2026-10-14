//! Context usage in the composer, with a hover preview and live click details.

mod format;

use bas::acp_v2 as acp;
use gpui_kit::component::{
    menu::{DropdownMenu, PopupMenu, PopupMenuItem},
    progress::ProgressCircle,
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
    gds_uim::el! { #div {
        id: $id; test_support; role: $gpui_kit::Role::Label; aria_label: $text.clone();
    } [ $text ] }
}

fn on_details_mouse_down(_: &gpui_kit::MouseDownEvent, window: &mut Window, cx: &mut App) {
    window.prevent_default();
    cx.stop_propagation();
}

fn details(usage: &acp::UsageUpdate, cx: &App) -> AnyElement {
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
    gds_uim::el! {
        #div {
            id: "usage-details"; test_support;
            on_mouse_down: $gpui_kit::MouseButton::Left, $on_details_mouse_down;
            text_color: $cx.theme().popover_foreground;
            w: $px(DETAILS_WIDTH); max_w_full; min_w_0; flex; flex_col; gap_2; py_1;
        } [
            #div {
                text_size: $crate::ui::SMALL; text_color: $cx.theme().muted_foreground;
            } [ $crate::i18n::tr!("usage-title") ],
            $label("usage-summary", summary(usage)),
            #div { h: 1.0.px; bg: $cx.theme().border; },
            $used,
            $capacity,
            ..remaining,
            ..cost
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
        self.usage
            .borrow()
            .as_ref()
            .map(|usage| details(usage, cx))
            .unwrap_or_else(|| gds_uim::el! { #div }.into_any_element())
    }

    fn build(self: &Rc<Self>, menu: PopupMenu, cx: &mut Context<PopupMenu>) -> PopupMenu {
        if let Some(view) = self.owner.upgrade() {
            let snapshot = self.clone();
            cx.observe(&view, move |_, view, cx| snapshot.refresh(view, cx))
                .detach();
        }
        let snapshot = self.clone();
        gds_uim::el! {
            $menu {
                item: $gds_uim::el! {
                    $PopupMenuItem::element(move |_, cx| snapshot.render(cx)) { disabled: true; }
                };
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
        let button = gds_uim::el! {
            Button($SharedString::from(format!("context-usage-{session}"))) {
                ghost; xsmall; w: $px(layout::ICON_BUTTON_SIZE); h: $px(layout::ICON_BUTTON_SIZE);
                p_0; flex_shrink_0; accessibility_label: $name.clone();
            } [
                #div { id: "usage-ring"; test_support; size: $px(RING_SIZE); } [
                    ProgressCircle($SharedString::from(format!("context-ring-{session}"))) {
                        value: $((fraction.unwrap_or(0.) * 100.) as f32);
                        color: $color; accessibility_label: $name; size: $px(RING_SIZE);
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
            gds_uim::el! { $button {
                dropdown_menu: $move |menu, window, cx| crate::ui::menu(build(menu, window, cx));
                anchor: $gpui_kit::Anchor::BottomLeft;
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
            gds_uim::el! {
                #div {
                    id: "session-usage"; test_support; flex_shrink_0;
                    @$(!details_open) => {
                        hoverable_tooltip: $move |window, cx| {
                            crate::ui::tooltip::view_with_id("usage-hint", hint.clone(), window, cx)
                        };
                    };
                } [ $menu ]
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
