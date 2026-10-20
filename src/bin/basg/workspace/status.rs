use super::*;
use gds_uim::el;
use gpui_kit::component::spinner::Spinner;

const ACTIVITY_ICON_SIZE: f32 = crate::ui::ICON;

/// Share the same motion and sizing across running activity indicators.
pub(super) fn activity_icon(
    id: impl Into<gpui_kit::ElementId>,
    icon: IconName,
    color: gpui_kit::Hsla,
    running: bool,
) -> AnyElement {
    let artwork = if running {
        el! {
            Spinner {
                icon: IconName::LoaderCircle;
                with_size: ACTIVITY_ICON_SIZE.px;
                color: color;
            }
        }
        .into_any_element()
    } else {
        el! {
            Icon(icon) {
                with_size: ACTIVITY_ICON_SIZE.px;
                text_color: color;
            }
        }
        .into_any_element()
    };
    el! {
        #div {
            id: id;
            test_support;
            size: ACTIVITY_ICON_SIZE.px;
            flex_shrink_0; flex; items_center; justify_center;
        } [$artwork]
    }
    .into_any_element()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ConnectionStatus {
    Connecting,
    Connected,
    Disconnected,
}

impl Workspace {
    pub(super) fn connection_status(&self) -> ConnectionStatus {
        if self.backend.error.is_some() {
            return ConnectionStatus::Disconnected;
        }
        if !self.backend.connected {
            return ConnectionStatus::Connecting;
        }
        if self.selected.id.is_none() {
            return ConnectionStatus::Connected;
        }
        if self
            .selected
            .id
            .is_some_and(|id| !self.sessions.healthy(id))
        {
            return ConnectionStatus::Disconnected;
        }
        match self.selected.stream {
            StreamState::Loading | StreamState::Reconnecting(_) => ConnectionStatus::Connecting,
            StreamState::Live => ConnectionStatus::Connected,
            StreamState::Unavailable(_) | StreamState::Closed => ConnectionStatus::Disconnected,
        }
    }

    pub(super) fn session_indicator(&self, id: &'static str, cx: &App) -> AnyElement {
        use crate::ui::session_status::Status;
        let state = if self.selected.creating {
            Status::Starting
        } else if let Some(meta) = self.current() {
            if !self.sessions.healthy(meta.id) {
                Status::Failed
            } else {
                Status::for_session(
                    meta,
                    self.sessions
                        .seen_replies
                        .get(&meta.id)
                        .copied()
                        .unwrap_or(0),
                    self.has_unread_reply(),
                )
            }
        } else {
            Status::Idle
        };
        let indicator = if state == Status::Unread {
            "unread-reply".to_owned()
        } else {
            format!("{id}-indicator")
        };
        state.render(id, indicator, cx)
    }

    pub(super) fn server_status_footer(&self, cx: &mut Context<Self>) -> AnyElement {
        use crate::connections::{self, presentation};
        use gpui_kit::component::menu::PopupMenuItem;

        let (label, color) = if self.backend.error.is_some() {
            (crate::i18n::tr!("status-disconnected"), cx.theme().danger)
        } else if self.backend.connected {
            (crate::i18n::tr!("status-connected"), cx.theme().success)
        } else {
            (crate::i18n::tr!("status-connecting"), cx.theme().warning)
        };
        let profiles = connections::shared(cx);
        let saved = profiles.read(cx).matching(&self.backend.server);
        let selected = saved.map(|profile| profile.id.clone());
        let target = &self.backend.server.target;
        let name = saved
            .map(|profile| profile.name.clone())
            .unwrap_or_else(|| presentation::endpoint(target));
        let hint = format!("{name} · {} · {label}", presentation::tooltip(target));
        let entries = profiles.read(cx).entries().to_vec();
        let owner = cx.entity().downgrade();
        let workspace_window = self.window.handle;
        let button = el! { Button("server-status") {
            small; ghost; w_full; min_w_0;
            accessibility_label: $hint.clone();
            tooltip: hint;
        } [
            #div {
                id: "server-status-indicator"; test_support; role: gpui_kit::Role::Status;
                aria_label: label; size: 6.0.px; rounded_full; flex_shrink_0; bg: color;
            },
            $presentation::icon(target) { size: crate::ui::ICON.px; flex_shrink_0; },
            #div {
                id: "server-status-name"; test_support; role: gpui_kit::Role::Label;
                aria_label: $name.clone(); flex_1; min_w_0; truncate; text_left;
            } [$name],
            Icon(IconName::ChevronDown) { size: crate::ui::SMALL_ICON.px; flex_shrink_0; },
        ] };
        let menu = self.menu_button(
            button,
            entries.len() * 2 + 2,
            false,
            move |menu, _, _| {
                let mut menu = menu.check_side(gpui_kit::component::Side::Right);
                for profile in &entries {
                    let id = profile.id.clone();
                    let owner = owner.clone();
                    menu = menu.item(
                        presentation::menu_item(profile)
                            .checked(selected.as_ref() == Some(&profile.id))
                            .on_click(move |_, _, cx| {
                                let id = id.clone();
                                let owner = owner.clone();
                                cx.defer(move |cx| {
                                    let result = connections::choose(&id, workspace_window, cx);
                                    let _ = owner.update(cx, |this, cx| {
                                        if let Err(error) = result {
                                            this.selected.error = Some(error);
                                        }
                                        cx.notify();
                                    });
                                });
                            }),
                    );
                }
                let owner = owner.clone();
                menu.separator().item(
                    PopupMenuItem::new(crate::i18n::tr!("connections-manage"))
                        .icon(IconName::Settings)
                        .on_click(move |_, _, cx| {
                            let owner = owner.clone();
                            cx.defer(move |cx| {
                                if let Err(error) = crate::settings::window::open_connections(cx) {
                                    let _ = owner.update(cx, |this, cx| {
                                        this.selected.error = Some(error.to_string());
                                        cx.notify();
                                    });
                                }
                            });
                        }),
                )
            },
            cx,
        );
        el! { #div {
            h: composer::COMPOSER_TOOLBAR_HEIGHT.px; mb: composer::COMPOSER_SIDE_PADDING.px;
            flex_shrink_0; flex; items_center; px_3; min_w_0;
        } [$menu] }
        .into_any_element()
    }

    pub(super) fn activity_label(&self) -> Option<String> {
        let active = self.composer.paste_pending
            || self.selected.creating
            || self.working()
            || !self.selected.approval.panel.is_empty()
            || !self.selected.elicitations.is_empty()
            || self.selected.id.is_some_and(|id| {
                self.sending(id) || self.sessions.pending.configuring.contains(&id)
            });
        (active && self.connection_status() == ConnectionStatus::Connected)
            .then(|| self.stream_label())
    }
}
