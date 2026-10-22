use super::*;
use crate::ui::{self, Button};
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
            .chain(self.pools.iter().map(|(id, _)| Some(id.clone())))
            .collect();
        let owner = cx.entity().downgrade();
        let selected = self.default_agent.clone();
        el! { Button("acp-policy-default") {
            small; disabled: $self.disabled();
            label: $selected.as_ref().map(ToString::to_string)
                .unwrap_or_else(|| crate::i18n::tr!("acp-policy-last-used").into());
            icon: IconName::ChevronDown;
            dropdown_menu: $move |menu, _, _| {
                let mut menu = ui::menu(menu);
                for id in &choices {
                    let owner = owner.clone();
                    let id = id.clone();
                    let label = id.as_ref().map(ToString::to_string)
                        .unwrap_or_else(|| crate::i18n::tr!("acp-policy-last-used").into());
                    menu = menu.item(PopupMenuItem::new(label).checked(id == selected)
                        .on_click(move |_, _, cx| {
                            let _ = owner.update(cx, |this, cx| {
                                if !this.disabled() { this.default_agent = id.clone(); cx.notify(); }
                            });
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
            last: false,
            reset: None,
            control: el! { #div { id: $id; test_support; w: $px(140.); } [
                $Input::new(input).small().disabled(self.disabled())
            ] }
            .into_any_element(),
        }
        .into_any_element()
    }
}

impl Render for Page {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let validation = self
            .baseline
            .as_ref()
            .and_then(|_| self.candidate(cx).err());
        let default_agent = super::super::form::Row {
            id: "acp-policy-default".into(),
            label: crate::i18n::tr!("acp-policy-default"),
            help: crate::i18n::tr!("acp-policy-default-help"),
            control: self.default_menu(cx),
            reset: None,
            last: false,
        };
        el! { #div {
            id: "acp-policy-page"; test_support; track_focus: $&self.focus;
            size_full; overflow_y_scroll; track_scroll: $&self.scroll;
            p_5; flex; flex_col; gap_3;
        } [
            #div { text_lg; font_weight: FontWeight::SEMIBOLD; } [$crate::i18n::tr!("settings-acp")],
            #div { text_size: $ui::SMALL; text_color: $cx.theme().muted_foreground; } [
                $crate::i18n::tr!("acp-policy-description")
            ],
            $self.field("acp-policy-idle", crate::i18n::tr!("acp-policy-idle"),
                crate::i18n::tr!("acp-policy-idle-help"), &self.idle),
            $default_agent,
            $self.field("acp-policy-size", crate::i18n::tr!("acp-policy-size"),
                crate::i18n::tr!("acp-policy-size-help"), &self.size),
            #div { font_weight: FontWeight::SEMIBOLD; } [$crate::i18n::tr!("acp-policy-pools")],
            #div { text_size: $ui::SMALL; text_color: $cx.theme().muted_foreground; } [
                $crate::i18n::tr!("acp-policy-pools-help")
            ],
            ..$self.pools.iter().map(|(id, input)| el! { #div {
                id: $format!("acp-policy-pool-row-{id}"); test_support;
                flex; items_center; gap_3; py_2; flex_shrink_0;
            } [
                #div { flex_1; min_w_0; truncate; } [$id.to_string()],
                #div { id: $format!("acp-policy-pool-{id}"); test_support; w: $px(140.); } [
                    $Input::new(input).small().disabled(self.disabled())
                ],
            ] }),
            ..$validation.as_ref().map(|error| super::super::feedback(
                "acp-policy-validation", error.clone(), cx.theme().danger)),
            ..$bool::then(!self.message.is_empty(), || super::super::feedback(
                "acp-policy-message", self.message.clone(), cx.theme().muted_foreground)),
            #div { flex; items_center; gap_2; flex_shrink_0; py_3; } [
                #super::super::action_button("acp-policy-save") {
                    label: $crate::i18n::tr!("acp-policy-save");
                    loading: $self.task.is_some();
                    disabled: $(self.disabled() || !self.dirty(cx) || validation.is_some());
                    on_click: $cx.listener(|this, _, window, cx| this.save(window, cx));
                },
                Button("acp-policy-reload") {
                    small; label: $crate::i18n::tr!("acp-policy-reload");
                    tooltip: $crate::i18n::tr!("acp-policy-reload-help");
                    disabled: $(self.task.is_some() || self.service.is_none());
                    on_click: $cx.listener(|this, _, window, cx| this.request(None, window, cx));
                },
            ],
        ] }
    }
}
