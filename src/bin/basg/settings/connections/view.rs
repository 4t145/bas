use super::*;
use crate::ui::{self, Button};
use crate::{
    i18n::tr,
    settings::{action_button, feedback, form::CONTROLS_WIDTH},
};
use gds_uim::el;
use gpui_kit::{
    AnyElement, FontWeight, Render, TestSupportExt,
    assets::IconName,
    component::{
        menu::{DropdownMenu, PopupMenuItem},
        spinner::Spinner,
        *,
    },
    div, px,
};

impl Page {
    fn add_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let manual = cx.entity().downgrade();
        let discover = manual.clone();
        el! { ("connection-new", IconName::Plus, tr!("connections-add")).ui::icon_action {
            disabled: $(self.editor.is_some() || self.profiles.read(cx).entries().is_empty());
            dropdown_menu: $move |menu, _, _| {
                let manual = manual.clone();
                let discover = discover.clone();
                ui::menu(menu)
                .item(PopupMenuItem::new(tr!("connections-manual"))
                    .on_click(move |_, window, cx| {
                        let _ = manual.update(cx, |this, cx| this.add(window, cx));
                    }))
                .item(PopupMenuItem::new(tr!("connections-discover"))
                    .on_click(move |_, window, cx| {
                        let _ = discover.update(cx, |this, cx| this.discover(window, cx));
                    }))
            };
        } }
        .into_any_element()
    }

    fn discovery_view(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        use discovery::State;
        let state = self.discovery.as_ref()?;
        let found = if let State::Found(endpoint) = state {
            let saved = self.local_profile(endpoint, cx);
            let added = saved.is_some();
            let name = saved
                .map(|profile| profile.name.as_str())
                .unwrap_or(discovery::LOCAL_NAME)
                .to_owned();
            let address = connections::presentation::endpoint(endpoint);
            Some(el! { .div {
                # "connection-discovered"; test_support;
                flex; items_center; gap_3; min_w_0;
            } [
                .div { flex_1; flex; flex_col; gap_2; min_w_0; } [
                    .div { flex; items_center; gap_2; min_w_0; } [
                        ($&bas::service::Target::Local).connections::presentation::icon { size: ui::ICON.px; flex_shrink_0; },
                        .div {
                            # "connection-discovered-name"; test_support;
                            role: gpui_kit::Role::Label; aria_label: ~name;
                            flex_1; min_w_0; truncate;
                        } [name],
                    ],
                    ("connection-discovered-endpoint", address, $cx.theme().muted_foreground).feedback { compact },
                ],
                ("connection-discovered-add").action_button {
                    label: @if (added) tr!("connections-already-added") @else tr!("connections-create") @end;
                    disabled: added; flex_shrink_0;
                    on_click: $cx.listener(|this, _, window, cx| this.add_discovered(window, cx));
                },
            ] })
        } else {
            None
        };
        Some(el! { .div {
            # "connection-discovery"; test_support;
            flex; flex_col; gap_3; p_3; border_1; rounded_md; min_w_0;
            border_color: $cx.theme().border; flex_shrink_0;
        } [
            .div { flex; items_center; gap_2; } [
                .div { flex_1; font_weight: FontWeight::SEMIBOLD; } [tr!("connections-discover")],
                ("connection-discovery-cancel", IconName::X, tr!("connections-cancel")).ui::icon_action {
                    on_click: $cx.listener(|this, _, window, cx| this.cancel(window, cx));
                },
            ],
            .div { text_color: $cx.theme().muted_foreground; } [tr!("connections-discovery-scope")],
            ..found,
            @match (state)
            @case (State::Loading)
                .div { flex; items_center; gap_2; } [
                    .Spinner::new { with_size: ui::ICON.px; color: $cx.theme().muted_foreground; },
                    ("connection-discovery-loading", tr!("connections-discovery-loading"), $cx.theme().foreground).feedback,
                ],
            @case (State::Failed)
                .div { flex; flex_col; gap_2; } [
                    ("connection-discovery-failed", tr!("connections-discovery-failed"), $cx.theme().foreground).feedback,
                    ("connection-discovery-retry").action_button {
                        label: tr!("connections-discovery-retry");
                        on_click: $cx.listener(|this, _, window, cx| this.discover(window, cx));
                    },
                ],
            @case (_)
            @end,
        ] }.into_any_element())
    }

    fn transport_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let owner = cx.entity().downgrade();
        let selected = self.transport;
        let button = el! { ("connection-type").Button::new {
            small; w_full; min_w_0;
            accessibility_label: tr!("connections-type");
        } [
            $(selected.icon()) { size: ui::ICON.px; },
            .div { flex_1; min_w_0; text_left; truncate; } [$selected.label()],
            (IconName::ChevronDown).Icon::new { size: ui::SMALL_ICON.px; },
        ] };
        el! { button {
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
        el! { .div {
            # format!("{id}-row"); test_support;
            flex; items_center; gap_3; py_2; min_w_0; flex_shrink_0;
        } [
            .div { flex_1; min_w_0; } [label],
            .div { # $id.to_owned(); test_support; w: CONTROLS_WIDTH.px; min_w_0; flex_shrink_0; } [control],
        ] }.into_any_element()
    }

    fn error_view(&self, cx: &App) -> Option<AnyElement> {
        self.error(cx).map(|message| {
            crate::settings::feedback("connections-message", message, cx.theme().danger)
                .into_any_element()
        })
    }

    fn certificate_field(&self, field: certificates::Field, cx: &mut Context<Self>) -> AnyElement {
        let input = field.input(self);
        let control = el! { .div { w_full; min_w_0; flex; items_center; gap_1; } [
            .div { flex_1; min_w_0; } [
                (input).Input::new { small; w_full; },
            ],
            (format!("{}-pick", field.id()), IconName::Folder, $field.picker_label()).ui::icon_action {
                disabled: $self.certificate_picker_open;
                flex_shrink_0;
                on_click: $cx.listener(move |this, _, window, cx| this.choose_certificate(field, window, cx));
            },
        ] }.into_any_element();
        Self::field(field.id(), field.label(), control)
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
            use certificates::Field;
            vec![
                self.certificate_field(Field::Ca, cx),
                self.certificate_field(Field::Certificate, cx),
                self.certificate_field(Field::Key, cx),
                Self::field(
                    "connection-tls-server-name",
                    crate::i18n::tr!("connections-tls-server-name"),
                    Input::new(&self.tls_server_name)
                        .small()
                        .w_full()
                        .into_any_element(),
                ),
            ]
        } else {
            Vec::new()
        };
        el! { .div {
            # "connection-editor"; test_support; key_context: "BasConnectionEditor";
            flex; flex_col; gap_2; min_w_0;
            on_action: $cx.listener(|this, _: &CancelEditing, window, cx| this.cancel(window, cx));
        } [
            .div { flex; items_center; gap_2; } [
                .div { flex_1; font_weight: FontWeight::SEMIBOLD; } [
                    @if (adding) tr!("connections-add") @else tr!("connections-edit-title") @end,
                ],
                ("connection-cancel", IconName::X, tr!("connections-cancel")).ui::icon_action {
                    on_click: $cx.listener(|this, _, window, cx| this.cancel(window, cx));
                },
            ],
            .div { flex; flex_col; } [
                ..fields,
                ..address,
                ..tls,
                $Self::field("connection-note", tr!("connections-note"),
                    Input::new(&self.note).small().w_full().into_any_element()),
            ],
            ..$self.error_view(cx),
            .div { flex; justify_end; pt_1; } [
                ("connection-save").action_button {
                    icon: @if (adding) IconName::Plus @else IconName::Check @end;
                    label: @if (adding) tr!("connections-create") @else tr!("connections-save") @end;
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
        let checked = profile.clone();
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
        let checking = matches!(self.check_state(&profile), Some(check::State::Checking));
        let check_result = match self.check_state(&profile) {
            Some(check::State::Complete(result)) => Some(match result {
                Ok(()) => (
                    crate::i18n::tr!("connections-check-passed").to_owned(),
                    cx.theme().success,
                ),
                Err(error) => (error.message(&profile.target), cx.theme().danger),
            }),
            _ => None,
        };
        let current = default && connections::current(cx).as_ref() == Some(&profile.options());
        el! { .div {
            # format!("connection-card-{}", profile.id); test_support;
            w_full; min_w_0; flex; flex_col; gap_2; px_3; py_2; flex_shrink_0;
            bg: $cx.theme().muted; border_1; rounded_md;
            border_color: @if (default) $cx.theme().primary @else $cx.theme().border @end;
        } [
            .div { w_full; flex; items_center; gap_2; min_w_0; } [
                .div { flex_1; min_w_0; overflow_hidden; flex; flex_col; gap_1; } [
                    .div { w_full; flex; items_center; gap_2; min_w_0; text_base; } [
                        ($&profile.target).connections::presentation::icon { size: ui::ICON.px; flex_shrink_0; },
                        .div {
                            # format!("connection-title-{}", profile.id); test_support;
                            role: gpui_kit::Role::Label; aria_label: ~name_hint;
                            min_w_0; truncate; font_weight: FontWeight::SEMIBOLD;
                            hoverable_tooltip: $move |window, cx| ui::tooltip::view(name_hint.clone(), window, cx);
                        } [name],
                        @if (default)
                            .div {
                                # format!("connection-default-{id}"); test_support;
                                text_color: $cx.theme().primary; flex_shrink_0;
                            } [tr!("connections-default")],
                        @end,
                    ],
                    .div {
                        # format!("connection-endpoint-{id}"); test_support;
                        role: gpui_kit::Role::Label; aria_label: ~target;
                        w_full; min_w_0; truncate; text_size: ui::SMALL; text_color: $cx.theme().muted_foreground;
                        hoverable_tooltip: $move |window, cx| ui::tooltip::view(target_hint.clone(), window, cx);
                    } [target],
                ],
                .div { # format!("connection-actions-{id}"); test_support; flex; items_center; gap_1; flex_shrink_0; } [
                    (format!("connection-check-{}", checked.id), IconName::Activity,
                        @if (checking) tr!("connections-checking") @else tr!("connections-check") @end
                    ).ui::icon_action {
                        disabled: $(busy || checking); loading: checking;
                        on_click: $cx.listener(move |this, _, _, cx| this.check_connection(&checked, cx));
                    },
                    (format!("connection-select-{id}"), IconName::Check,
                        @if (current) tr!("connections-in-use") @else tr!("connections-use") @end
                    ).ui::icon_action {
                        disabled: $(busy || current);
                        on_click: $cx.listener(move |this, _, _, cx| this.choose(&id, cx));
                    },
                    (format!("connection-edit-{}", edit.id), IconName::Pencil, tr!("connections-edit")).ui::icon_action {
                        disabled: busy;
                        on_click: $cx.listener(move |this, _, window, cx| this.edit(&edit, window, cx));
                    },
                    (format!("connection-remove-{}", remove.id), IconName::Trash,
                        @if (keep_one) tr!("connections-keep-one") @else tr!("connections-remove") @end
                    ).ui::icon_action {
                        disabled: $(busy || keep_one);
                        on_click: $cx.listener(move |this, _, _, cx| this.request_remove(remove.clone(), cx));
                    },
                ],
            ],
            ..$profile.note.map(|note| {
                let hint = note.clone();
                el! { .div {
                    # format!("connection-note-{}", profile.id); test_support;
                    role: gpui_kit::Role::Label; aria_label: ~note;
                    w_full; min_w_0; truncate;
                    text_size: ui::SMALL; text_color: $cx.theme().muted_foreground;
                    hoverable_tooltip: $move |window, cx| ui::tooltip::view(hint.clone(), window, cx);
                } [$note.replace('\n', " ")] }
            }),
            @if (let Some((message, color)) = check_result)
                (format!("connection-check-result-{}", profile.id), message, color).feedback,
            @end,
            @if (editing)
                .div {
                    border_t_1; border_color: $cx.theme().border; pt_3; pb_1;
                } [$self.editor_view(cx)],
            @end,
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
        el! { .div {
            # "connections-page"; test_support; track_focus: $&self.focus; key_context: "BasConnections";
            size_full; px_5; py_4; text_size: ui::SMALL;
            flex; flex_col; gap_3;
            on_action: $cx.listener(|this, _: &AddConnection, window, cx| this.add(window, cx));
            on_action: $cx.listener(|this, _: &CancelEditing, window, cx| this.cancel(window, cx));
        } [
            .div { # "connections-toolbar"; test_support; flex; items_center; gap_2; flex_shrink_0; } [
                .div { font_weight: FontWeight::SEMIBOLD; } [tr!("connections-list")],
                .div { flex_1; text_color: $cx.theme().muted_foreground; } [$count.to_string()],
                ("connection-reload", IconName::RefreshCw,
                    @if (editing) tr!("connections-finish-edit") @else tr!("connections-reload") @end
                ).ui::icon_action {
                    disabled: editing;
                    on_click: $cx.listener(|this, _, _, cx| this.reload(cx));
                },
                $self.add_menu(cx),
            ],
            .div {
                # "connections-list"; test_support; flex_1; min_h_0;
                overflow_y_scroll; track_scroll: $&self.scroll; flex; flex_col; gap_3;
            } [
                ..error,
                ..$self.discovery_view(cx),
                @if (adding)
                    .div {
                        border_1; border_color: $cx.theme().border; rounded_md;
                        bg: $cx.theme().muted; p_3; flex_shrink_0;
                    } [$self.editor_view(cx)],
                @end,
                ..cards,
            ],
        ] }
    }
}
