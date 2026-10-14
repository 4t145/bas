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

    pub(super) fn connection_indicator(&self, id: &'static str, cx: &App) -> Button {
        let (label, color) = match self.connection_status() {
            ConnectionStatus::Connecting => {
                (crate::i18n::tr!("status-connecting"), cx.theme().warning)
            }
            ConnectionStatus::Connected => {
                (crate::i18n::tr!("status-connected"), cx.theme().success)
            }
            ConnectionStatus::Disconnected => {
                (crate::i18n::tr!("status-disconnected"), cx.theme().danger)
            }
        };
        let unread = self.has_unread_reply();
        let label = if unread {
            crate::i18n::tr!("status-with-unread", label = (label).to_string())
        } else {
            label.to_owned()
        };
        let unread_indicator = unread.then(|| {
            el! {
                #div {
                    id: "unread-reply";
                    test_support;
                    absolute; inset_0; rounded_full; border_2;
                    border_color: $cx.theme().warning;
                }
            }
        });
        el! {
            Button(id) {
                ghost;
                xsmall;
                cursor_default;
                accessibility_label: $label.clone();
                tooltip: label;
            } [
                #div {
                    relative; size_3; flex_shrink_0; flex; items_center; justify_center;
                } [
                    #div { size_2; rounded_full; bg: color; },
                    ..unread_indicator,
                ],
            ]
        }
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
