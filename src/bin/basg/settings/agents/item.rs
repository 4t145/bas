//! One agent identity and action scale, shared by configuration and installation.

use super::Agents;
use crate::{agents::icon::AgentIcon, i18n::tr, settings, ui};
use bas::management::{Entry, Source, Status};
use gds_uim::el;
use gpui_kit::{
    AnyElement, App, ElementId, FontWeight, IntoElement, RenderOnce, SharedString, TestSupportExt,
    Window,
    assets::IconName,
    component::{ActiveTheme, Disableable},
    div,
    prelude::*,
    px,
};

const AVATAR_SIZE: f32 = 32.;
const ICON_SIZE: f32 = 22.;

#[cfg(all(test, feature = "gui-tests"))]
mod tests;

pub(super) fn action(
    id: impl Into<ElementId>,
    icon: IconName,
    hint: impl Into<SharedString>,
) -> crate::ui::Button {
    crate::ui::icon_action(id, icon, hint)
}

#[derive(IntoElement)]
pub(super) struct Item {
    pub entry: Entry,
    pub status: Option<String>,
    pub status_hint: String,
    pub feedback: Option<String>,
    pub actions: AnyElement,
    pub content: Option<AnyElement>,
}

trait EntryPresentation {
    fn displayed_version(&self) -> Option<&str>;
    fn links(&self) -> AnyElement;
}

impl EntryPresentation for Entry {
    fn displayed_version(&self) -> Option<&str> {
        self.installed_version
            .as_deref()
            .filter(|version| !version.is_empty())
            .or_else(|| {
                (self.source == Source::Registry && !self.version.is_empty())
                    .then_some(self.version.as_str())
            })
    }

    fn links(&self) -> AnyElement {
        let repository = self.repository.clone();
        let license = self.license_url.clone();
        el! {
            .div { flex; items_center; flex_shrink_0; gap_1; } [
                @if (!repository.is_empty())
                    (format!("agents-repository-{}", self.id), IconName::Github, tr!("agent-repository")).action {
                        on_click: $move |_, _, cx| cx.open_url(&repository);
                    },
                @end,
                @if (!self.license.is_empty())
                    (format!("agents-license-{}", self.id), IconName::Scale, tr!("agent-license", license = self.license.clone())).action {
                        disabled: $license.is_empty();
                        on_click: $move |_, _, cx| cx.open_url(&license);
                    },
                @end,
            ]
        }
        .into_any_element()
    }
}

impl RenderOnce for Item {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = &self.entry.id;
        let version = self.entry.displayed_version().map(str::to_owned);
        let description = self.entry.description.clone();
        let status = self.status.map(|label| {
            ui::tooltip::bounded(
                format!("agents-item-status-hint-{id}"),
                el! {
                    .div {
                        # format!("agents-item-status-{id}"); test_support;
                        role: gpui_kit::Role::Label; aria_label: ~label;
                        text_color: $cx.theme().primary; flex_shrink_0;
                    } [label]
                },
                self.status_hint,
            )
        });
        el! {
            .div {
                # format!("agents-item-{id}"); test_support;
                w_full; min_w_0; flex; flex_col; gap_1; px_3; py_2;
                text_size: ui::SMALL;
                bg: $cx.theme().muted; border_1; border_color: $cx.theme().border; rounded_md;
            } [
                .div {
                    # format!("agents-item-heading-{id}"); test_support;
                    flex; items_center; gap_2; min_w_0;
                } [
                    .div {
                        size: AVATAR_SIZE.px; flex_shrink_0; flex; items_center; justify_center;
                        text_color: $cx.theme().primary;
                    } [
                        (format!("agents-icon-{id}"), ~$(self.entry.icon)).AgentIcon::new {
                            size: ICON_SIZE.px; text_color: $cx.theme().primary;
                        },
                    ],
                    .div {
                        # format!("agents-item-name-{id}"); test_support;
                        role: gpui_kit::Role::Label; aria_label: ~$(self.entry.name);
                        min_w_0; truncate; font_weight: FontWeight::SEMIBOLD;
                    } [~$(self.entry.name)],
                    @if (let Some(version) = version)
                        .div {
                            # format!("agents-item-version-{id}"); test_support;
                            role: gpui_kit::Role::Label; aria_label: ~version;
                            text_color: $cx.theme().muted_foreground; flex_shrink_0;
                        } [version],
                    @end,
                    ..status,
                    .div { flex_1; min_w_0; },
                    .div {
                        # format!("agents-item-actions-{id}"); test_support;
                        flex; items_center; gap_1; flex_shrink_0;
                    } [$self.actions],
                ],
                .div {
                    # format!("agents-item-details-{id}"); test_support;
                    flex; items_center; gap_2; min_w_0;
                } [
                    .div {
                        # format!("agents-item-description-{id}"); test_support;
                        role: gpui_kit::Role::Label; aria_label: ~$(self.entry.description);
                        min_w_0; flex_1; truncate; text_color: $cx.theme().muted_foreground;
                        @if (!description.is_empty())
                            hoverable_tooltip: $move |window, cx| ui::tooltip::view(description.clone(), window, cx);
                        @end;
                    } [~$(self.entry.description)],
                    $self.entry.links(),
                ],
                @if (let Some(feedback) = self.feedback)
                    (format!("agents-operation-{id}"), feedback, $cx.theme().primary).settings::feedback,
                @end,
                @if (let Some(content) = self.content)
                    .div { pt_2; border_t_1; border_color: $cx.theme().border; } [content],
                @end,
            ]
        }
    }
}

impl Agents {
    pub(super) fn item_status(&self, entry: &Entry) -> Option<String> {
        Some(if entry.source == Source::Embedded {
            crate::i18n::tr!("agent-builtin").into()
        } else {
            match entry.status {
                Status::Installed => crate::i18n::tr!("agent-installed").into(),
                Status::NotInstalled => crate::i18n::tr!("agent-not-installed").into(),
                Status::Missing => crate::i18n::tr!("agent-program-unavailable").into(),
                Status::Unsupported => crate::i18n::tr!("agent-platform-unsupported").into(),
            }
        })
    }
}
