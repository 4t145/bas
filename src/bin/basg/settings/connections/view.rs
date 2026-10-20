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
    fn transport_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let owner = cx.entity().downgrade();
        let selected = self.transport;
        let button = el! { Button("connection-type") {
            small; w_full; min_w_0;
            accessibility_label: $crate::i18n::tr!("connections-type");
        } [
            $selected.icon() { size: $px(ui::ICON); },
            #div { flex_1; min_w_0; text_left; truncate; } [$selected.label()],
            Icon(IconName::ChevronDown) { size: $px(ui::SMALL_ICON); },
        ] };
        el! { $button {
            dropdown_menu: $move |menu, _, _| {
                let mut menu = ui::menu(menu);
                for &kind in Transport::AVAILABLE {
                    let owner = owner.clone();
                    menu = menu.item(PopupMenuItem::new(kind.label())
                        .icon(kind.icon()).checked(kind == selected)
                        .on_click(move |_, window, cx| {
                            let _ = owner.update(cx, |this, cx| this.select_transport(kind, window, cx));
                        }));
                }
                menu
            };
        } }.into_any_element()
    }

    fn field(id: &str, label: &'static str, control: AnyElement) -> AnyElement {
        el! { #div {
            id: $format!("{id}-row"); test_support;
            flex; items_center; gap_3; py_2; min_w_0; flex_shrink_0;
        } [
            #div { flex_1; min_w_0; } [$label],
            #div { id: $id.to_owned(); test_support; w: $px(super::super::form::CONTROLS_WIDTH); min_w_0; flex_shrink_0; } [$control],
        ] }.into_any_element()
    }

    fn error_view(&self, cx: &App) -> Option<AnyElement> {
        self.error(cx).map(|message| {
            el! { #div {
                id: "connections-message"; test_support; role: gpui_kit::Role::Status;
                aria_label: $message.clone(); flex; items_start; gap_2; min_w_0;
                text_size: ui::SMALL; text_color: $cx.theme().danger;
            } [
                Icon(IconName::CircleAlert) { size: $px(ui::ICON); flex_shrink_0; },
                #div { flex_1; min_w_0; } [$message],
            ] }
            .into_any_element()
        })
    }

    fn editor_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let adding = matches!(self.editor, Some(Editor::Add));
        let fields = [
            Self::field(
                "connection-name",
                crate::i18n::tr!("connections-name"),
                Input::new(&self.name).small().w_full().into_any_element(),
            ),
            Self::field(
                "connection-kind",
                crate::i18n::tr!("connections-type"),
                self.transport_menu(cx),
            ),
        ];
        let address = (self.transport != Transport::Local).then(|| {
            Self::field(
                "connection-target",
                self.transport.address_label(),
                ui::input::placeholder(&self.target, Input::new(&self.target).small().w_full())
                    .into_any_element(),
            )
        });
        let tls: Vec<_> = if self.transport == Transport::Mtls {
            [
                (
                    "connection-tls-ca",
                    crate::i18n::tr!("connections-tls-ca"),
                    &self.tls_ca,
                ),
                (
                    "connection-tls-certificate",
                    crate::i18n::tr!("connections-tls-certificate"),
                    &self.tls_certificate,
                ),
                (
                    "connection-tls-key",
                    crate::i18n::tr!("connections-tls-key"),
                    &self.tls_key,
                ),
                (
                    "connection-tls-server-name",
                    crate::i18n::tr!("connections-tls-server-name"),
                    &self.tls_server_name,
                ),
            ]
            .into_iter()
            .map(|(id, label, input)| {
                Self::field(
                    id,
                    label,
                    Input::new(input).small().w_full().into_any_element(),
                )
            })
            .collect()
        } else {
            Vec::new()
        };
        el! { #div {
            id: "connection-editor"; test_support; key_context: "BasConnectionEditor";
            flex; flex_col; gap_2; min_w_0;
            on_action: $cx.listener(|this, _: &CancelEditing, window, cx| this.cancel(window, cx));
        } [
            #div { flex; items_center; gap_2; } [
                #div { flex_1; font_weight: FontWeight::SEMIBOLD; } [
                    $(if adding { crate::i18n::tr!("connections-add") } else { crate::i18n::tr!("connections-edit-title") }),
                ],
                #ui::icon_action("connection-cancel", IconName::X, $crate::i18n::tr!("connections-cancel")) {
                    on_click: $cx.listener(|this, _, window, cx| this.cancel(window, cx));
                },
            ],
            #div { flex; flex_col; } [
                ..fields,
                ..address,
                ..tls,
                $Self::field("connection-note", crate::i18n::tr!("connections-note"),
                    Input::new(&self.note).small().w_full().into_any_element()),
            ],
            ..$self.error_view(cx),
            #div { flex; justify_end; pt_1; } [
                #super::super::action_button("connection-save") {
                    icon: $(if adding { IconName::Plus } else { IconName::Check });
                    label: $(if adding { crate::i18n::tr!("connections-create") } else { crate::i18n::tr!("connections-save") });
                    on_click: $cx.listener(|this, _, window, cx| this.save(window, cx));
                },
            ],
        ] }.into_any_element()
    }

    fn card(
        &self,
        profile: ConnectionProfile,
        default: bool,
        keep_one: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = profile.id.clone();
        let edit = profile.clone();
        let remove = profile.clone();
        let name = profile.name.clone();
        let name_hint = format!(
            "{name} · {}",
            connections::presentation::tooltip(&profile.target)
        );
        let target = connections::presentation::endpoint(&profile.target);
        let target_hint = connections::presentation::tooltip(&profile.target);
        let editing =
            matches!(&self.editor, Some(Editor::Edit(original)) if original.id == profile.id);
        let busy = self.editor.is_some();
        let current = default && connections::current(cx).as_ref() == Some(&profile.options());
        el! { #div {
            id: $format!("connection-card-{}", profile.id); test_support;
            w_full; min_w_0; flex; flex_col; gap_2; px_3; py_2; flex_shrink_0;
            bg: $cx.theme().muted; border_1; rounded_md;
            border_color: $(if default { cx.theme().primary } else { cx.theme().border });
        } [
            #div { w_full; flex; items_center; gap_2; min_w_0; } [
                #div { flex_1; min_w_0; overflow_hidden; flex; flex_col; gap_1; } [
                    #div { w_full; flex; items_center; gap_2; min_w_0; text_base; } [
                        $connections::presentation::icon(&profile.target) { size: $px(ui::ICON); flex_shrink_0; },
                        #div {
                            id: $format!("connection-title-{}", profile.id); test_support;
                            role: gpui_kit::Role::Label; aria_label: $name_hint.clone();
                            min_w_0; truncate; font_weight: FontWeight::SEMIBOLD;
                            hoverable_tooltip: $move |window, cx| ui::tooltip::view(name_hint.clone(), window, cx);
                        } [$name],
                        ..$default.then(|| el! { #div {
                            id: $format!("connection-default-{id}"); test_support;
                            text_color: $cx.theme().primary; flex_shrink_0;
                        } [$crate::i18n::tr!("connections-default")] }),
                    ],
                    #div {
                        id: $format!("connection-endpoint-{id}"); test_support;
                        role: gpui_kit::Role::Label; aria_label: $target.clone();
                        w_full; min_w_0; truncate; text_size: ui::SMALL; text_color: $cx.theme().muted_foreground;
                        hoverable_tooltip: $move |window, cx| ui::tooltip::view(target_hint.clone(), window, cx);
                    } [$target],
                ],
                #div { id: $format!("connection-actions-{id}"); test_support; flex; items_center; gap_1; flex_shrink_0; } [
                    #ui::icon_action($format!("connection-select-{id}"), IconName::Check, $(if current {
                        crate::i18n::tr!("connections-in-use")
                    } else { crate::i18n::tr!("connections-use") })) {
                        disabled: $(busy || current);
                        on_click: $cx.listener(move |this, _, _, cx| this.choose(&id, cx));
                    },
                    #ui::icon_action($format!("connection-edit-{}", edit.id), IconName::Pencil, $crate::i18n::tr!("connections-edit")) {
                        disabled: busy;
                        on_click: $cx.listener(move |this, _, window, cx| this.edit(&edit, window, cx));
                    },
                    #ui::icon_action($format!("connection-remove-{}", remove.id), IconName::Trash, $(if keep_one {
                        crate::i18n::tr!("connections-keep-one")
                    } else { crate::i18n::tr!("connections-remove") })) {
                        disabled: $(busy || keep_one);
                        on_click: $cx.listener(move |this, _, _, cx| this.request_remove(remove.clone(), cx));
                    },
                ],
            ],
            ..$profile.note.map(|note| {
                let hint = note.clone();
                el! { #div {
                id: $format!("connection-note-{}", profile.id); test_support;
                role: gpui_kit::Role::Label; aria_label: $note.clone();
                min_w_0; truncate; text_color: $cx.theme().muted_foreground;
                hoverable_tooltip: $move |window, cx| ui::tooltip::view(hint.clone(), window, cx);
            } [$note] } }),
            ..$editing.then(|| el! { #div {
                border_t_1; border_color: $cx.theme().border; pt_3; pb_1;
            } [$self.editor_view(cx)] }),
        ] }.into_any_element()
    }
}

impl Render for Page {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let profiles = self.profiles.read(cx).entries().to_vec();
        let selected = self
            .profiles
            .read(cx)
            .selected()
            .map(|profile| profile.id.clone());
        let count = profiles.len();
        let editing = self.editor.is_some();
        let error = (!editing).then(|| self.error_view(cx)).flatten();
        let adding = matches!(self.editor, Some(Editor::Add));
        if let Some(index) = self.scroll_to.take() {
            self.scroll.scroll_to_top_of_item(index);
        }
        let cards: Vec<_> = profiles
            .into_iter()
            .map(|profile| {
                let default = selected.as_ref() == Some(&profile.id);
                self.card(profile, default, count <= 1, cx)
            })
            .collect();
        el! { #div {
            id: "connections-page"; test_support; track_focus: $&self.focus; key_context: "BasConnections";
            size_full; px_5; py_4; text_size: ui::SMALL;
            flex; flex_col; gap_3;
            on_action: $cx.listener(|this, _: &AddConnection, window, cx| this.add(window, cx));
        } [
            #div { id: "connections-toolbar"; test_support; flex; items_center; gap_2; flex_shrink_0; } [
                #div { font_weight: FontWeight::SEMIBOLD; } [$crate::i18n::tr!("connections-list")],
                #div { flex_1; text_color: $cx.theme().muted_foreground; } [$count.to_string()],
                #ui::icon_action("connection-reload", IconName::RefreshCw, $(if editing {
                    crate::i18n::tr!("connections-finish-edit")
                } else { crate::i18n::tr!("connections-reload") })) {
                    disabled: editing;
                    on_click: $cx.listener(|this, _, _, cx| this.reload(cx));
                },
                #ui::icon_action("connection-new", IconName::Plus, $crate::i18n::tr!("connections-add")) {
                    disabled: $(editing || count == 0);
                    on_click: $cx.listener(|this, _, window, cx| this.add(window, cx));
                },
            ],
            #div {
                id: "connections-list"; test_support; flex_1; min_h_0;
                overflow_y_scroll; track_scroll: $&self.scroll; flex; flex_col; gap_3;
            } [
                ..error,
                ..$bool::then(adding, || el! { #div {
                    border_1; border_color: $cx.theme().border; rounded_md;
                    bg: $cx.theme().muted; p_3; flex_shrink_0;
                } [$self.editor_view(cx)] }),
                ..cards,
            ],
        ] }
    }
}
