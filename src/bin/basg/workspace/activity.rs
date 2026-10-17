//! Background conversations and protocol-announced subagents, never inferred tool counts.

use super::*;
use gpui_kit::{AnyWindowHandle, Bounds, WeakEntity};

pub(super) const PANEL_WIDTH: f32 = 380.;
pub(super) const PANEL_HEIGHT: f32 = 300.;

struct Running {
    id: String,
    agent: String,
    title: String,
    directory: String,
    status: String,
    description: Option<String>,
}

struct Snapshot {
    connected: bool,
    sessions: Vec<Running>,
}

impl Workspace {
    fn activity_sessions(&self) -> Vec<&SessionMeta> {
        if !self.backend.connected || self.backend.error.is_some() {
            return Vec::new();
        }
        let current = self.current();
        let mut sessions: Vec<_> = current
            .into_iter()
            .chain(
                self.sessions
                    .entries
                    .iter()
                    .filter(|meta| current.is_none_or(|selected| meta.id != selected.id)),
            )
            .filter(|meta| {
                self.sessions.healthy(meta.id)
                    && !matches!(meta.status, SessionStatus::Stopped | SessionStatus::Failed)
            })
            .collect();
        sessions.sort_by_key(|meta| (meta.created_at, meta.id));
        sessions
    }

    pub(super) fn running_sessions(&self) -> Vec<&SessionMeta> {
        if !self.backend.connected || self.backend.error.is_some() {
            return Vec::new();
        }
        let mut sessions: Vec<_> = self
            .sessions
            .entries
            .iter()
            .filter(|meta| Some(meta.id) != self.selected.id)
            .filter(|meta| self.sessions.healthy(meta.id) && meta.status == SessionStatus::Working)
            .collect();
        sessions.sort_by_key(|meta| (meta.created_at, meta.id));
        sessions
    }

    pub(super) fn activity_count(&self) -> usize {
        self.running_sessions().len()
            + self
                .activity_sessions()
                .into_iter()
                .map(|session| {
                    session
                        .subagents
                        .iter()
                        .filter(|child| child.is_active())
                        .count()
                })
                .sum::<usize>()
    }

    fn activity_snapshot(&self) -> Snapshot {
        let mut rows = Vec::new();
        for meta in self.activity_sessions() {
            if Some(meta.id) != self.selected.id && meta.status == SessionStatus::Working {
                rows.push(Running {
                    id: meta.id.to_string(),
                    agent: meta.agent.to_string(),
                    title: title(meta),
                    directory: navigation::groups::path_label(meta.cwd.as_deref()),
                    status: crate::i18n::tr!("status-running").into(),
                    description: None,
                });
            }
            for child in meta.subagents.iter().filter(|child| child.is_active()) {
                rows.push(Running {
                    id: format!("{}-child-{}", meta.id, child.id),
                    agent: crate::i18n::tr!("agent-activity-child", agent = meta.agent.to_string()),
                    title: child.title.clone().unwrap_or_else(|| child.id.clone()),
                    directory: crate::i18n::tr!("agent-activity-owner", title = title(meta)),
                    status: if matches!(child.state, Some(bas::acp::StateUpdate::RequiresAction(_)))
                    {
                        crate::i18n::tr!("status-input").into()
                    } else {
                        crate::i18n::tr!("status-running").into()
                    },
                    description: child.description.clone(),
                });
            }
        }
        Snapshot {
            connected: self.backend.connected && self.backend.error.is_none(),
            sessions: rows,
        }
    }

    pub(super) fn hover_activity(
        &mut self,
        hovered: bool,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.window.activity_hover = None;
        let activity_open = self
            .window
            .reply_preview
            .as_ref()
            .is_some_and(|preview| preview.key == reply::PreviewKey::Activity);
        if hovered && (activity_open || self.window.menu.is_some() || self.activity_count() == 0) {
            return;
        }
        if !hovered && !activity_open {
            return;
        }
        self.window.activity_hover = Some(cx.spawn_in(window, async move |view, cx| {
            cx.background_executor()
                .timer(if hovered {
                    reply::HOVER_DELAY
                } else {
                    reply::CLOSE_DELAY
                })
                .await;
            let _ = view.update_in(cx, |view, window, cx| {
                if hovered {
                    if view.activity_count() == 0 || view.window.menu.is_some() {
                        return;
                    }
                    view.close_reply_preview(window, cx);
                    let snapshot = view.activity_snapshot();
                    let owner = cx.entity();
                    let parent = window.window_handle();
                    let content = cx.new(|cx| Panel {
                        owner: owner.downgrade(),
                        parent,
                        snapshot,
                        _changed: cx.observe(&owner, |panel: &mut Panel, owner, cx| {
                            panel.snapshot = owner.read(cx).activity_snapshot();
                            cx.notify();
                        }),
                    });
                    view.open_preview(
                        content.into(),
                        reply::PreviewKey::Activity,
                        bounds,
                        window,
                        cx,
                    );
                } else if view
                    .window
                    .reply_preview
                    .as_ref()
                    .is_some_and(|preview| preview.key == reply::PreviewKey::Activity)
                {
                    view.close_reply_preview(window, cx);
                }
            });
        }));
    }
}

struct Panel {
    owner: WeakEntity<Workspace>,
    parent: AnyWindowHandle,
    snapshot: Snapshot,
    _changed: Subscription,
}

impl Render for Panel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.snapshot.sessions.len();
        let heading = if self.snapshot.connected {
            crate::i18n::tr!("agent-activity-count", count = count)
        } else {
            crate::i18n::tr!("agent-activity-disconnected").into()
        };
        gds_uim::el! { #div {
            id: "agent-activity-panel"; test_support; size_full; flex; flex_col;
            p_3; gap_2; border_1; rounded_md; overflow_hidden;
            border_color: $cx.theme().border; bg: $cx.theme().popover;
            text_color: $cx.theme().popover_foreground; text_size: crate::ui::SMALL;
            on_hover: $cx.listener(|this, hovered: &bool, window, cx| {
                menus::in_parent(this.parent, window, cx, |window, cx| {
                    let _ = this.owner.update(cx, |view, cx| view.hover_activity(*hovered, Bounds::default(), window, cx));
                });
            });
        } [
            #div {
                id: "agent-activity-heading"; test_support; role: gpui_kit::Role::Status;
                aria_label: $heading.clone();
                font_weight: FontWeight::SEMIBOLD; flex_shrink_0;
            } [$heading],
            #div {
                id: "agent-activity-scroll"; test_support; flex_1; min_h_0; overflow_y_scroll;
                flex; flex_col; gap_2;
            } [
                ..$bool::then(self.snapshot.connected && count == 0, || caption(crate::i18n::tr!("agent-activity-empty"), cx)),
                ..$self.snapshot.sessions.iter().map(|row| gds_uim::el! { #div {
                    id: $format!("agent-activity-{}", row.id); test_support; flex; flex_col; flex_shrink_0;
                    gap_1; pb_2; border_b_1; border_color: $cx.theme().border;
                } [
                    #div { flex; items_center; gap_2; min_w_0; } [
                        #div { flex_1; min_w_0; truncate; } [$row.agent.clone()],
                        #div { text_color: $cx.theme().muted_foreground; } [$row.status.clone()],
                    ],
                    #div {
                        id: $format!("agent-activity-title-{}", row.id); test_support;
                        role: gpui_kit::Role::Label; aria_label: $row.title.clone();
                        whitespace_normal; min_w_0;
                    } [$crate::ui::tooltip::wrap_tokens(&row.title)],
                    ..$row.description.as_ref().map(|description| gds_uim::el! {
                        #div { text_color: $cx.theme().muted_foreground; whitespace_normal; } [$crate::ui::tooltip::wrap_tokens(description)]
                    }),
                    #div { text_color: $cx.theme().muted_foreground; truncate; } [$row.directory.clone()],
                ] }),
            ],
        ] }
    }
}
