//! Passive session indicators shared by the conversation sidebar and history browser.

use bas::model::{SessionMeta, SessionSnapshot, SessionStatus};
use gpui_kit::{
    AnyElement, App, SharedString, TestSupportExt,
    assets::IconName,
    component::{ActiveTheme, Icon, Sizable, spinner::Spinner},
    div,
    prelude::*,
    px,
};

const STATUS_SIZE: f32 = super::SMALL_ICON;
const LIGHT_SIZE: f32 = 6.;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Status {
    Starting,
    Working,
    Unread,
    Idle,
    Failed,
    Stopped,
}

impl From<SessionStatus> for Status {
    fn from(status: SessionStatus) -> Self {
        match status {
            SessionStatus::Starting => Self::Starting,
            SessionStatus::Working => Self::Working,
            SessionStatus::Idle => Self::Idle,
            SessionStatus::Failed => Self::Failed,
            SessionStatus::Stopped => Self::Stopped,
        }
    }
}

impl Status {
    pub(crate) fn for_session(session: &SessionMeta, seen: u64, active_unread: bool) -> Self {
        match Self::from(session.status) {
            Self::Idle if session.last_reply.raw() > seen || active_unread => Self::Unread,
            status => status,
        }
    }

    pub(crate) fn for_snapshot(session: &SessionSnapshot) -> Self {
        session.status().into()
    }

    fn label(self) -> &'static str {
        match self {
            Self::Starting => crate::i18n::tr!("status-starting"),
            Self::Working => crate::i18n::tr!("status-running"),
            Self::Unread => crate::i18n::tr!("status-unread"),
            Self::Idle => crate::i18n::tr!("status-ready"),
            Self::Failed => crate::i18n::tr!("status-failed"),
            Self::Stopped => crate::i18n::tr!("status-stopped"),
        }
    }

    fn indicator(self, id: SharedString, cx: &App) -> AnyElement {
        let indicator = gds_uim::el! { #div {
            id: id; test_support; flex; items_center; justify_center; flex_shrink_0;
        } };
        match self {
            Self::Starting | Self::Working => gds_uim::el! {
                $indicator { size: STATUS_SIZE.px; } [
                    Spinner { icon: IconName::LoaderCircle; with_size: STATUS_SIZE.px; color: $cx.theme().primary; }
                ]
            },
            Self::Idle => gds_uim::el! { $indicator { size_0; } },
            Self::Unread | Self::Stopped => gds_uim::el! { $indicator {
                size: LIGHT_SIZE.px; rounded_full;
                bg: $(match self {
                    Self::Stopped => cx.theme().muted_foreground,
                    _ => cx.theme().primary,
                });
            } },
            Self::Failed => gds_uim::el! { $indicator { size: STATUS_SIZE.px; } [
                Icon(IconName::TriangleAlert) { with_size: STATUS_SIZE.px; text_color: $cx.theme().danger; }
            ] },
        }.into_any_element()
    }

    pub(crate) fn render(
        self,
        id: impl Into<SharedString>,
        indicator_id: impl Into<SharedString>,
        cx: &App,
    ) -> AnyElement {
        let id: SharedString = id.into();
        let slot = gds_uim::el! { #div {
            size: super::COMPACT_CONTROL.px; flex_shrink_0; flex; items_center; justify_center;
        } [$self.indicator(indicator_id.into(), cx)] };
        if self == Self::Idle {
            return slot.into_any_element();
        }
        gds_uim::el! { $slot {
            id: id; test_support; cursor_default; role: gpui_kit::Role::Status;
            aria_label: $self.label();
            hoverable_tooltip: $move |window, cx| super::tooltip::view(self.label(), window, cx);
        } }
        .into_any_element()
    }
}

pub(crate) fn archived(id: impl Into<SharedString>) -> AnyElement {
    let id: SharedString = id.into();
    let label = crate::i18n::tr!("session-archived");
    gds_uim::el! { #div {
        id: id; test_support; role: gpui_kit::Role::Status; aria_label: label;
        size: super::COMPACT_CONTROL.px; flex; items_center; justify_center; flex_shrink_0;
        cursor_default;
        hoverable_tooltip: $move |window, cx| super::tooltip::view(label, window, cx);
    } [Icon(IconName::Archive) { size: STATUS_SIZE.px; }] }
    .into_any_element()
}

#[cfg(test)]
mod tests;
