use super::*;
use crate::ui::{self, Button};
use crate::{i18n::tr, settings::feedback};
use gds_uim::el;
use gpui_kit::{
    AnyElement, FontWeight, Render, TestSupportExt,
    assets::IconName,
    component::{
        menu::{DropdownMenu, PopupMenuItem},
        *,
    },
    div, px,
};

impl Page {
    fn default_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let choices: Vec<_> = std::iter::once(None)
            .chain(self.acp_agents().cloned().map(Some))
            .collect();
        let owner = cx.entity().downgrade();
        let selected = self.default_agent.clone();
        el! { ("acp-policy-default").Button::new {
            small; disabled: $self.disabled();
            label: $selected.as_ref().map(ToString::to_string)
                .unwrap_or_else(|| tr!("acp-policy-last-used").into());
            icon: IconName::ChevronDown;
            dropdown_menu: $move |menu, _, _| {
                let mut menu = ui::menu(menu);
                for id in &choices {
                    let owner = owner.clone();
                    let id = id.clone();
                    let label = id.as_ref().map(ToString::to_string)
                        .unwrap_or_else(|| tr!("acp-policy-last-used").into());
                    menu = menu.item(PopupMenuItem::new(label).checked(id == selected)
                        .on_click(move |_, _, cx| {
                            let _ = owner.update(cx, |this, cx| {
                                if !this.disabled() {
                                    this.default_agent = id.clone();
                                    this.changed(cx);
                                    this.flush_pending(cx);
                                }
                            });
                        }));
                }
                menu
            };
        } }
        .into_any_element()
    }

    fn add_pool_button(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let choices = self.pool_candidates();
        let disabled = self.disabled() || choices.is_empty();
        let owner = cx.entity().downgrade();
        let parent = window.window_handle();
        el! { ("acp-policy-add-pool").Button::new {
            small; icon: IconName::Plus;
            label: tr!("acp-policy-add-pool");
            disabled: disabled;
            dropdown_menu: $move |menu, _, _| {
                let mut menu = ui::menu(menu);
                for id in &choices {
                    let owner = owner.clone();
                    let id = id.clone();
                    menu = menu.item(PopupMenuItem::new(id.to_string())
                        .on_click(move |_, window, cx| {
                            let add = |window: &mut Window, cx: &mut App| {
                                let _ = owner.update(cx, |page, cx| page.add_pool(id.clone(), window, cx));
                            };
                            if window.window_handle() == parent {
                                add(window, cx);
                            } else {
                                let _ = parent.update(cx, |_, window, cx| add(window, cx));
                            }
                        }));
                }
                menu
            };
        } }.into_any_element()
    }

    fn field(
        &self,
        id: &'static str,
        label: &'static str,
        help: &'static str,
        input: &Entity<InputState>,
    ) -> AnyElement {
        super::super::form::Row {
            id: id.into(),
            label,
            help,
            last: true,
            reset: None,
            control: el! { .div { # id; test_support; w: 140.0.px; } [
                (input).Input::new { small; disabled: $self.disabled() },
            ] }
            .into_any_element(),
        }
        .into_any_element()
    }
}

impl Render for Page {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let error = self.error(cx);
        let default_agent = super::super::form::Row {
            id: "acp-policy-default".into(),
            label: crate::i18n::tr!("acp-policy-default"),
            help: crate::i18n::tr!("acp-policy-default-help"),
            control: self.default_menu(cx),
            reset: None,
            last: false,
        };
        el! { .div {
            # "acp-policy-page"; test_support; track_focus: $&self.focus;
            size_full; overflow_y_scroll; track_scroll: $&self.scroll;
            p_5; flex; flex_col; gap_5;
        } [
            .div { flex; flex_col; gap_2; flex_shrink_0; } [
                .div {
                    # "session-management-title"; test_support;
                    role: gpui_kit::Role::Heading; aria_label: tr!("settings-session-management");
                    text_size: ui::HEADING;
                } [tr!("settings-session-management")],
                .div { text_size: ui::SMALL; text_color: $cx.theme().muted_foreground; } [
                    tr!("acp-policy-description")
                ],
            ],
            .div { # "session-management-retirement"; test_support; flex; flex_col; gap_2; flex_shrink_0; } [
                .div { text_size: ui::HEADING; } [tr!("session-management-retirement")],
                .div { border_t_1; border_color: $cx.theme().border; } [
                    $self.field("acp-policy-idle", tr!("acp-policy-idle"),
                        tr!("acp-policy-idle-help"), &self.idle),
                ],
            ],
            .div { # "session-management-prewarming"; test_support; flex; flex_col; gap_2; flex_shrink_0; } [
                .div { text_size: ui::HEADING; } [tr!("session-management-prewarming")],
                .div { border_t_1; border_color: $cx.theme().border; flex; flex_col; } [
                    default_agent,
                    $self.field("acp-policy-size", tr!("acp-policy-size"),
                        tr!("acp-policy-size-help"), &self.size),
                ],
                .div { flex; items_center; justify_between; gap_3; } [
                    .div { font_weight: FontWeight::SEMIBOLD; } [tr!("acp-policy-pools")],
                    $self.add_pool_button(window, cx),
                ],
                .div { text_size: ui::SMALL; text_color: $cx.theme().muted_foreground; } [
                    tr!("acp-policy-pools-help")
                ],
                @if (self.pools.is_empty())
                    .div {
                        # "acp-policy-pools-empty"; test_support; py_2;
                        text_size: ui::SMALL; text_color: $cx.theme().muted_foreground;
                    } [tr!("acp-policy-pools-empty")],
                @end,
                ..$(self.pools.iter()) { map: pool => .div {
                        # format!("acp-policy-pool-row-{}", pool.agent); test_support;
                        flex; items_center; gap_3; py_2;
                    } [
                        .div { flex_1; min_w_0; truncate; } [$pool.agent.to_string()],
                        .div { # format!("acp-policy-pool-{}", pool.agent); test_support; w: 100.0.px; } [
                            ($&pool.size).Input::new { small; disabled: $self.disabled() },
                        ],
                        (format!("acp-policy-remove-pool-{}", pool.agent), IconName::Trash,
                            tr!("acp-policy-remove-pool")).ui::icon_action {
                            disabled: $self.disabled();
                            on_click: $cx.listener({
                                let id = pool.agent.clone();
                                move |page, _, _, cx| page.remove_pool(&id, cx)
                            });
                        },
                    ]
                },
            ],
            @if (let Some(error) = error)
                .div { flex; flex_col; gap_2; flex_shrink_0; } [
                    ("acp-policy-error", error, $cx.theme().danger).feedback,
                    @if (self.service.is_some())
                        ("acp-policy-reload").Button::new {
                            small; label: tr!("acp-policy-reload");
                            tooltip: tr!("acp-policy-reload-help");
                            disabled: $(self.load_task.is_some() || self.write.is_some());
                            on_click: $cx.listener(|this, _, window, cx| this.load(window, cx));
                        },
                    @end,
                ],
            @end,
        ] }
    }
}
