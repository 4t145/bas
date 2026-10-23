//! Passive session indicators shared by the conversation sidebar and history browser.

use crate::i18n::tr;
use bas::model::{SessionMeta, SessionSnapshot, SessionStatus};
use gds_uim::el;
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
            Self::Starting => tr!("status-starting"),
            Self::Working => tr!("status-running"),
            Self::Unread => tr!("status-unread"),
            Self::Idle => tr!("status-ready"),
            Self::Failed => tr!("status-failed"),
            Self::Stopped => tr!("status-stopped"),
        }
    }

    fn indicator(self, id: SharedString, cx: &App) -> AnyElement {
        let indicator = el! { .div {
            # id; test_support; flex; items_center; justify_center; flex_shrink_0;
        } };
        el! {
            @match (self)
            @case (Self::Starting | Self::Working)
                indicator { size: STATUS_SIZE.px; } [
                    .Spinner::new { icon: IconName::LoaderCircle; with_size: STATUS_SIZE.px; color: $cx.theme().primary; }
                ]
            @case (Self::Idle)
                indicator { size_0; }
            @case (Self::Unread | Self::Stopped)
                indicator {
                    size: LIGHT_SIZE.px; rounded_full;
                    bg: @if (self == Self::Stopped)
                        $cx.theme().muted_foreground
                    @else
                        $cx.theme().primary
                    @end;
                }
            @case (Self::Failed)
                indicator { size: STATUS_SIZE.px; } [
                    IconName::TriangleAlert.Icon::new { with_size: STATUS_SIZE.px; text_color: $cx.theme().danger; }
                ]
            @end
        }.into_any_element()
    }

    pub(crate) fn render(
        self,
        id: impl Into<SharedString>,
        indicator_id: impl Into<SharedString>,
        cx: &App,
    ) -> AnyElement {
        let id: SharedString = id.into();
        let slot = el! { .div {
            size: super::COMPACT_CONTROL.px; flex_shrink_0; flex; items_center; justify_center;
        } [$self.indicator(indicator_id.into(), cx)] };
        if self == Self::Idle {
            return slot.into_any_element();
        }
        el! { slot {
            # id; test_support; cursor_default; role: gpui_kit::Role::Status;
            aria_label: $self.label();
            hoverable_tooltip: move (window, cx) => ($self.label(), window, cx).super::tooltip::view;
        } }
        .into_any_element()
    }
}

pub(crate) fn archived(id: impl Into<SharedString>) -> AnyElement {
    let id: SharedString = id.into();
    let label = tr!("session-archived");
    el! { .div {
        # id; test_support; role: gpui_kit::Role::Status; aria_label: label;
        size: super::COMPACT_CONTROL.px; flex; items_center; justify_center; flex_shrink_0;
        cursor_default;
        hoverable_tooltip: move (window, cx) => (label, window, cx).super::tooltip::view;
    } [IconName::Archive.Icon::new { size: STATUS_SIZE.px; }] }
    .into_any_element()
}

#[cfg(test)]
mod tests;
