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

    pub(super) fn server_status_footer(&self, cx: &App) -> AnyElement {
        let (label, color) = if self.backend.error.is_some() {
            (crate::i18n::tr!("status-disconnected"), cx.theme().danger)
        } else if self.backend.connected {
            (crate::i18n::tr!("status-connected"), cx.theme().success)
        } else {
            (crate::i18n::tr!("status-connecting"), cx.theme().warning)
        };
        let address = self.backend.server.to_string();
        let hint = format!("{address} · {label}");
        el! { #div {
            id: "server-status"; test_support; role: gpui_kit::Role::Status;
            aria_label: $hint.clone();
            h: composer::COMPOSER_TOOLBAR_HEIGHT.px; mb: composer::COMPOSER_SIDE_PADDING.px;
            flex_shrink_0; flex; items_center; gap_2; px_3; min_w_0;
            text_size: crate::ui::SMALL; text_color: $cx.theme().muted_foreground;
            hoverable_tooltip: $move |window, cx| crate::ui::tooltip::view(hint.clone(), window, cx);
        } [
            #div { size: 6.0.px; rounded_full; flex_shrink_0; bg: color; },
            #div { flex_1; min_w_0; truncate; } [$address],
            #div { flex_shrink_0; } [$label],
        ] }.into_any_element()
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
