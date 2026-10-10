//! Context usage in the composer, with a hover preview and live click details.

mod format;

use basr::acp_v2 as acp;
use gpui_kit::component::{
    menu::{DropdownMenu, PopupMenu, PopupMenuItem},
    progress::ProgressCircle,
    tooltip::Tooltip,
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
        None => format!("{} · 容量未知", format::short_tokens(usage.used)),
    }
}

fn label(id: &'static str, text: String) -> impl IntoElement {
    div()
        .id(id)
        .test_support()
        .role(gpui_kit::Role::Label)
        .aria_label(text.clone())
        .child(text)
}

fn hint(usage: &acp::UsageUpdate, cx: &App) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("Context"),
        )
        .child(label("usage-hover-summary", summary(usage)))
        .children(usage.cost.as_ref().map(|cost| {
            div()
                .text_sm()
                .child(format!("费用 · {}", format::cost_label(cost)))
        }))
        .into_any_element()
}

fn details(usage: &acp::UsageUpdate, cx: &App) -> AnyElement {
    let remaining = usage.size.saturating_sub(usage.used);
    div()
        .id("usage-details")
        .test_support()
        .on_mouse_down(gpui_kit::MouseButton::Left, |_, window, cx| {
            window.prevent_default();
            cx.stop_propagation();
        })
        .text_color(cx.theme().popover_foreground)
        .w(px(DETAILS_WIDTH))
        .max_w_full()
        .min_w_0()
        .flex()
        .flex_col()
        .gap_2()
        .py_1()
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("Context"),
        )
        .child(label("usage-summary", summary(usage)))
        .child(div().h(px(1.)).bg(cx.theme().border))
        .child(label(
            "usage-used",
            format!("已用 tokens · {}", format::exact_tokens(usage.used)),
        ))
        .child(label(
            "usage-capacity",
            match ratio(usage) {
                Some(_) => format!("上下文容量 · {}", format::exact_tokens(usage.size)),
                None => "上下文容量 · 未知".into(),
            },
        ))
        .when(usage.size > 0, |panel| {
            panel.child(label(
                "usage-remaining",
                format!("剩余 tokens · {}", format::exact_tokens(remaining)),
            ))
        })
        .children(usage.cost.as_ref().map(|cost| {
            label(
                "usage-cost",
                crate::tooltip::wrap_tokens(&format!(
                    "会话累计费用 · {}",
                    format::cost_label(cost)
                )),
            )
        }))
        .into_any_element()
}

impl Workspace {
    pub(super) fn render_usage(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let conversation = self.conversation.as_ref()?;
        let usage = conversation.usage.as_ref()?;
        let session = conversation.session.id;
        let fraction = ratio(usage);
        let color = if fraction.is_some_and(|ratio| ratio >= WARNING_RATIO) {
            cx.theme().warning
        } else {
            cx.theme().muted_foreground
        };
        let name = format!("上下文 {} · 查看详情", summary(usage));
        let button = Button::new(SharedString::from(format!("context-usage-{session}")))
            .ghost()
            .xsmall()
            .w(px(layout::ICON_BUTTON_SIZE))
            .h(px(layout::ICON_BUTTON_SIZE))
            .p_0()
            .flex_shrink_0()
            .accessibility_label(name.clone())
            .child(
                div()
                    .id("usage-ring")
                    .test_support()
                    .size(px(RING_SIZE))
                    .child(
                        ProgressCircle::new(SharedString::from(format!("context-ring-{session}")))
                            .value((fraction.unwrap_or(0.) * 100.) as f32)
                            .color(color)
                            .accessibility_label(name)
                            .size(px(RING_SIZE)),
                    ),
            );
        let owner = cx.entity().downgrade();
        let observer = owner.clone();
        let compact = self.compact;
        // Native popup creation paints synchronously while Workspace is borrowed.
        // Render from a detached snapshot, updated after Workspace notifications.
        let snapshot = Rc::new(RefCell::new(Some(usage.clone())));
        let build = move |menu: PopupMenu, _: &mut Window, cx: &mut Context<PopupMenu>| {
            if let Some(view) = observer.upgrade() {
                let snapshot = snapshot.clone();
                cx.observe(&view, move |_, view, cx| {
                    let workspace = view.read(cx);
                    *snapshot.borrow_mut() = workspace
                        .conversation
                        .as_ref()
                        .filter(|conversation| {
                            conversation.session.id == session && workspace.compact == compact
                        })
                        .and_then(|conversation| conversation.usage.clone());
                    if snapshot.borrow().is_none() {
                        cx.emit(gpui_kit::DismissEvent);
                    }
                    cx.notify();
                })
                .detach();
            }
            let snapshot = snapshot.clone();
            menu.item(
                PopupMenuItem::element(move |_, cx| {
                    snapshot
                        .borrow()
                        .as_ref()
                        .map(|usage| details(usage, cx))
                        .unwrap_or_else(|| div().into_any_element())
                })
                .disabled(true),
            )
        };
        let menu = if self.compact {
            self.menu_button(button, DETAILS_ROWS, false, build, cx)
        } else {
            button
                .dropdown_menu(build)
                .anchor(gpui_kit::Anchor::BottomLeft)
                .on_open_change(move |open, _, cx| {
                    let _ = owner.update(cx, |this, cx| {
                        if *open && this.active == Some(session) {
                            this.usage_details_open = Some(session);
                        } else if !*open && this.usage_details_open == Some(session) {
                            this.usage_details_open = None;
                        }
                        cx.notify();
                    });
                })
                .into_any_element()
        };
        let details_open =
            self.usage_details_open == Some(session) || (self.compact && self.menu.is_some());
        let preview = usage.clone();
        Some(
            div()
                .id("session-usage")
                .test_support()
                .flex_shrink_0()
                .when(!details_open, |trigger| {
                    trigger.hoverable_tooltip(move |window, cx| {
                        let preview = preview.clone();
                        Tooltip::element(move |_, cx| hint(&preview, cx)).build(window, cx)
                    })
                })
                .child(menu)
                .into_any_element(),
        )
    }
}
