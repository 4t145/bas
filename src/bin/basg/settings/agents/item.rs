//! One agent identity and action scale, shared by configuration and installation.

use super::Agents;
use bas::management::{Entry, Source, Status};
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
        gds_uim::el! {
            #div { flex; items_center; flex_shrink_0; gap_1; } [
                ..$bool::then(!repository.is_empty(), || gds_uim::el! {
                    #action($format!("agents-repository-{}", self.id), $IconName::Github, $crate::i18n::tr!("agent-repository")) {
                        on_click: $move |_, _, cx| cx.open_url(&repository);
                    }
                }),
                ..$bool::then(!self.license.is_empty(), || gds_uim::el! {
                    #action($format!("agents-license-{}", self.id), $IconName::Scale, $crate::i18n::tr!("agent-license", license = self.license.clone())) {
                        disabled: $license.is_empty();
                        on_click: $move |_, _, cx| cx.open_url(&license);
                    }
                }),
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
            crate::ui::tooltip::bounded(
                format!("agents-item-status-hint-{id}"),
                gds_uim::el! {
                    #div {
                        id: $format!("agents-item-status-{id}"); test_support;
                        role: gpui_kit::Role::Label; aria_label: $label.clone();
                        text_color: $cx.theme().primary; flex_shrink_0;
                    } [$label]
                },
                self.status_hint,
            )
        });
        gds_uim::el! {
            #div {
                id: $format!("agents-item-{id}"); test_support;
                w_full; min_w_0; flex; flex_col; gap_1; px_3; py_2;
                text_size: $crate::ui::SMALL;
                bg: $cx.theme().muted; border_1; border_color: $cx.theme().border; rounded_md;
            } [
                #div {
                    id: $format!("agents-item-heading-{id}"); test_support;
                    flex; items_center; gap_2; min_w_0;
                } [
                    #div {
                        size: $px(AVATAR_SIZE); flex_shrink_0; flex; items_center; justify_center;
                        text_color: $cx.theme().primary;
                    } [
                        crate::agents::icon::AgentIcon($format!("agents-icon-{id}"), $self.entry.icon.clone()) {
                            size: $px(ICON_SIZE); text_color: $cx.theme().primary;
                        },
                    ],
                    #div {
                        id: $format!("agents-item-name-{id}"); test_support;
                        role: gpui_kit::Role::Label; aria_label: $self.entry.name.clone();
                        min_w_0; truncate; font_weight: $FontWeight::SEMIBOLD;
                    } [$self.entry.name.clone()],
                    ..$version.map(|version| gds_uim::el! {
                        #div {
                            id: $format!("agents-item-version-{id}"); test_support;
                            role: gpui_kit::Role::Label; aria_label: $version.clone();
                            text_color: $cx.theme().muted_foreground; flex_shrink_0;
                        } [$version]
                    }),
                    ..status,
                    #div { flex_1; min_w_0; },
                    #div {
                        id: $format!("agents-item-actions-{id}"); test_support;
                        flex; items_center; gap_1; flex_shrink_0;
                    } [$self.actions],
                ],
                #div {
                    id: $format!("agents-item-details-{id}"); test_support;
                    flex; items_center; gap_2; min_w_0;
                } [
                    #div {
                        id: $format!("agents-item-description-{id}"); test_support;
                        role: gpui_kit::Role::Label; aria_label: $self.entry.description.clone();
                        min_w_0; flex_1; truncate; text_color: $cx.theme().muted_foreground;
                        @$!description.is_empty() => {
                            hoverable_tooltip: $move |window, cx| crate::ui::tooltip::view(description.clone(), window, cx);
                        };
                    } [$self.entry.description.clone()],
                    $self.entry.links(),
                ],
                ..$self.feedback.map(|feedback| crate::ui::feedback(
                    format!("agents-operation-{id}"), feedback, cx.theme().primary,
                )),
                ..$self.content.map(|content| gds_uim::el! {
                    #div { pt_2; border_t_1; border_color: $cx.theme().border; } [$content]
                }),
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
