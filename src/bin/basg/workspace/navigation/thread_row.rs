use bas::model::Timestamp;
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::component::spinner::Spinner;

use crate::workspace::*;

const STATUS_SIZE: f32 = 12.;
const UNREAD_SIZE: f32 = 6.;
const STOPPED_WIDTH: f32 = 8.;
const STOPPED_HEIGHT: f32 = 1.;
pub(super) const ACTIONS_WIDTH: f32 = layout::ICON_BUTTON_SIZE * 2.;
pub(super) const ACTIONS_GAP: f32 = 4.;
const MINUTE_MILLIS: u64 = 60 * 1_000;
const HOUR_MILLIS: u64 = 60 * MINUTE_MILLIS;
const DAY_MILLIS: u64 = 24 * HOUR_MILLIS;

pub(super) fn recent_time(updated_at: Timestamp, now: Timestamp) -> String {
    let elapsed = now.millis().saturating_sub(updated_at.millis());
    if elapsed < MINUTE_MILLIS {
        crate::i18n::tr!("time-just-now").into()
    } else if elapsed < HOUR_MILLIS {
        crate::i18n::tr!("time-minutes", count = elapsed / MINUTE_MILLIS)
    } else if elapsed < DAY_MILLIS {
        crate::i18n::tr!("time-hours", count = elapsed / HOUR_MILLIS)
    } else {
        crate::i18n::tr!("time-days", count = elapsed / DAY_MILLIS)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RowStatus {
    Starting,
    Working,
    Unread,
    Idle,
    Failed,
    Stopped,
}

impl RowStatus {
    fn for_session(session: &SessionMeta, seen: u64, active_unread: bool) -> Self {
        match session.status {
            SessionStatus::Starting => Self::Starting,
            SessionStatus::Working => Self::Working,
            SessionStatus::Failed => Self::Failed,
            SessionStatus::Stopped => Self::Stopped,
            SessionStatus::Idle if session.last_reply.raw() > seen || active_unread => Self::Unread,
            SessionStatus::Idle => Self::Idle,
        }
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

    fn indicator(self, id: SessionId, cx: &App) -> AnyElement {
        let indicator = gds_uim::el! { #div {
            id: $SharedString::from(format!("thread-indicator-{id}"));
            test_support; flex; items_center; justify_center; flex_shrink_0;
        } };
        match self {
            Self::Starting | Self::Working => gds_uim::el! {
                $indicator { size: STATUS_SIZE.px; } [
                    Spinner { icon: IconName::LoaderCircle; with_size: STATUS_SIZE.px; color: $cx.theme().primary; }
                ]
            },
            Self::Unread => gds_uim::el! { $indicator { size: UNREAD_SIZE.px; rounded_full; bg: $cx.theme().primary; } },
            Self::Idle => gds_uim::el! { $indicator { size_0; } },
            Self::Failed => gds_uim::el! { $indicator { size: STATUS_SIZE.px; } [
                Icon(IconName::TriangleAlert) { with_size: STATUS_SIZE.px; text_color: $cx.theme().danger; }
            ] },
            Self::Stopped => gds_uim::el! { $indicator {
                w: STOPPED_WIDTH.px; h: STOPPED_HEIGHT.px; bg: $cx.theme().muted_foreground;
            } },
        }
        .into_any_element()
    }

    fn slot(self, id: SessionId, cx: &App) -> AnyElement {
        let slot = gds_uim::el! { #div {
            size: layout::ICON_BUTTON_SIZE.px; flex_shrink_0;
            flex; items_center; justify_center;
        } [$self.indicator(id, cx)] };
        if self == Self::Idle {
            return slot.into_any_element();
        }
        gds_uim::el! { $slot {
            id: $SharedString::from(format!("thread-status-{id}")); test_support;
            cursor_default; role: gpui_kit::Role::Status;
            aria_label: $self.label();
            hoverable_tooltip: $move |window, cx| crate::ui::tooltip::view(self.label(), window, cx);
        } }.into_any_element()
    }
}

impl Workspace {
    pub(super) fn thread_row(&self, session: &SessionMeta, cx: &mut Context<Self>) -> AnyElement {
        let id = session.id;
        let editing = self.renaming_thread(id);
        let pinned = self.navigation.pinned.contains(&id);
        let disabled = !self.backend.connected
            || self.sessions.pending.archiving.contains(&id)
            || self.sessions.pending.deleting.contains(&id);
        let state = if !self.sessions.healthy(id) {
            RowStatus::Failed
        } else {
            RowStatus::for_session(
                session,
                self.sessions.seen_replies.get(&id).copied().unwrap_or(0),
                self.selected.id == Some(id) && self.has_unread_reply(),
            )
        };
        let group = SharedString::from(format!("thread-row-{id}"));
        let menu_session = session.clone();
        let title_content = if editing {
            self.render_title_editor(cx)
        } else {
            gds_uim::el! { #div { min_w_0; truncate; text_size: $crate::ui::SMALL; } [
                $title(session)
            ] }
            .into_any_element()
        };
        let pin = (pinned && !editing).then(|| {
            gds_uim::el! { Icon(IconName::Pin) { xsmall; flex_shrink_0; } }
        });
        let title_row = gds_uim::el! {
            #div {
                id: $SharedString::from(format!("thread-title-{id}"));
                test_support; h: layout::ICON_BUTTON_SIZE.px; min_w_0; flex; items_center; gap_1;
                @$!editing => {
                    group_hover: $group.clone(), $gds_uim::m! { pr: $px(ACTIONS_WIDTH + ACTIONS_GAP); };
                };
            } [$title_content, ..pin]
        };
        let actions = gds_uim::el! {
            #div {
                id: $SharedString::from(format!("thread-actions-{id}"));
                test_support; absolute; top_0; right_0; flex; items_center;
                w: ACTIONS_WIDTH.px; h: layout::ICON_BUTTON_SIZE.px; invisible;
                @$!editing => { group_hover: $group.clone(), $gds_uim::m! { visible; }; };
            } [
                #layout::icon_button($SharedString::from(format!("rename-{id}")), IconName::Pencil) {
                    accessibility_label: $crate::i18n::tr!("session-rename");
                    hoverable_tooltip: $shortcuts::tooltip(crate::i18n::tr!("session-rename"), shortcuts::RenameThread);
                    disabled: disabled;
                    on_click: $cx.listener(move |this, _, window, cx| {
                        this.on_thread_row_rename(id, window, cx);
                    });
                },
                $self.thread_visibility_button(session, cx),
            ]
        };
        let details = gds_uim::el! {
            #div { flex; items_center; justify_between; h: layout::ICON_BUTTON_SIZE.px; } [
                #div {
                    id: $SharedString::from(format!("thread-time-{id}"));
                    test_support; text_size: $crate::ui::SMALL; text_color: $cx.theme().muted_foreground;
                } [$recent_time(session.agent_updated_at.unwrap_or(session.updated_at), Timestamp::now())],
                #div {
                    id: $SharedString::from(format!("thread-rail-{id}"));
                    test_support; size: layout::ICON_BUTTON_SIZE.px; flex_shrink_0;
                } [$state.slot(id, cx)],
            ]
        };
        gds_uim::el! {
            #div {
                id: $SharedString::from(format!("session-{id}"));
                test_support; group: group; w_full; px_3; py_2; cursor_pointer;
                flex; items_start; gap_2;
                @$self.selected.id == Some(id) => { bg: $cx.theme().sidebar_accent; };
                hover => { bg: $cx.theme().sidebar_accent; };
                on_click: $cx.listener(move |this, _, window, cx| this.select(id, window, cx));
                on_mouse_down: gpui_kit::MouseButton::Right,
                    $cx.listener(move |this, event, window, cx| {
                        this.on_thread_row_context_menu(&menu_session, event, window, cx);
                    });
            } [
                #div { h: layout::ICON_BUTTON_SIZE.px; flex; items_center; flex_shrink_0; } [
                    $self.agent_icon(format!("thread-agent-{id}"), session.agent.as_str(), cx),
                ],
                #div { relative; flex_1; min_w_0; flex; flex_col; } [$title_row, $details, $actions],
            ]
        }.into_any_element()
    }

    fn on_thread_row_rename(&mut self, id: SessionId, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        self.begin_sidebar_rename(id, window, cx);
    }

    fn on_thread_row_context_menu(
        &mut self,
        session: &SessionMeta,
        event: &gpui_kit::MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.prevent_default();
        cx.stop_propagation();
        self.open_thread_menu(session, event.position, window, cx);
    }

    fn thread_visibility_button(&self, session: &SessionMeta, cx: &mut Context<Self>) -> Button {
        let id = session.id;
        if session.archived || !self.sessions.is_draft(id) {
            return self.archive_button(session, "archive", cx);
        }
        gds_uim::el! { #layout::icon_button($SharedString::from(format!("discard-{id}")), IconName::Trash) {
            accessibility_label: $crate::i18n::tr!("draft-discard");
            tooltip: $crate::i18n::tr!("draft-discard");
            disabled: $(!self.backend.connected
                    || self.sessions.pending.archiving.contains(&id)
                    || self.sessions.pending.deleting.contains(&id));
            loading: $self.sessions.pending.deleting.contains(&id);
            on_click: $cx.listener(move |this, _, window, cx| {
                this.on_thread_row_discard(id, window, cx);
            });
        } }
    }

    fn on_thread_row_discard(
        &mut self,
        id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        self.discard_session_draft(id, window, cx);
    }

    fn open_thread_menu(
        &mut self,
        session: &SessionMeta,
        position: gpui_kit::Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = session.id;
        let archived = session.archived;
        let draft = !archived && self.sessions.is_draft(id);
        let pinned = self.navigation.pinned.contains(&id);
        let disabled = !self.backend.connected
            || self.sessions.pending.archiving.contains(&id)
            || self.sessions.pending.deleting.contains(&id);
        let state = RowStatus::for_session(
            session,
            self.sessions.seen_replies.get(&id).copied().unwrap_or(0),
            self.selected.id == Some(id) && self.has_unread_reply(),
        );
        let owner = cx.entity().downgrade();
        self.open_context_menu(
            position,
            if state == RowStatus::Unread { 6 } else { 5 },
            std::rc::Rc::new(move |menu, window, _| {
                let parent = window.window_handle();
                let pin_owner = owner.clone();
                let rename_owner = owner.clone();
                let archive_owner = owner.clone();
                let delete_owner = owner.clone();
                let read_owner = owner.clone();
                menu.item(
                    shortcuts::menu_item(
                        if pinned {
                            crate::i18n::tr!("session-unpin")
                        } else {
                            crate::i18n::tr!("session-pin")
                        },
                        Some(Box::new(shortcuts::ToggleThreadPin)),
                    )
                    .icon(IconName::Pin)
                    .on_click(move |_, window, cx| {
                        menus::in_parent(parent, window, cx, |_, cx| {
                            let _ = pin_owner.update(cx, |this, cx| this.toggle_pinned(id, cx));
                        });
                    }),
                )
                .item(
                    shortcuts::menu_item(
                        crate::i18n::tr!("session-rename"),
                        Some(Box::new(shortcuts::RenameThread)),
                    )
                    .disabled(disabled)
                    .on_click(move |_, window, cx| {
                        menus::in_parent(parent, window, cx, |window, cx| {
                            let _ = rename_owner.update(cx, |this, cx| {
                                this.begin_sidebar_rename(id, window, cx);
                            });
                        });
                    }),
                )
                .item(
                    shortcuts::menu_item(
                        if draft {
                            crate::i18n::tr!("draft-discard")
                        } else if archived {
                            crate::i18n::tr!("session-restore")
                        } else {
                            crate::i18n::tr!("session-archive")
                        },
                        (!draft && !archived).then(|| {
                            Box::new(shortcuts::ArchiveThread) as Box<dyn gpui_kit::Action>
                        }),
                    )
                    .icon(if draft {
                        IconName::Trash
                    } else {
                        IconName::Archive
                    })
                    .disabled(disabled)
                    .on_click(move |_, window, cx| {
                        menus::in_parent(parent, window, cx, |window, cx| {
                            let _ = archive_owner.update(cx, |this, cx| {
                                this.on_thread_row_menu_visibility(id, draft, archived, window, cx);
                            });
                        });
                    }),
                )
                .when(state == RowStatus::Unread, |menu| {
                    menu.item(
                        PopupMenuItem::new(crate::i18n::tr!("session-mark-read")).on_click(
                            move |_, window, cx| {
                                menus::in_parent(parent, window, cx, |_, cx| {
                                    let _ = read_owner
                                        .update(cx, |this, cx| this.mark_thread_read(id, cx));
                                });
                            },
                        ),
                    )
                })
                .separator()
                .item(
                    shortcuts::menu_item(
                        crate::i18n::tr!("session-delete"),
                        Some(Box::new(shortcuts::DeleteThread)),
                    )
                    .disabled(disabled)
                    .on_click(move |_, window, cx| {
                        menus::in_parent(parent, window, cx, |window, cx| {
                            let _ = delete_owner
                                .update(cx, |this, cx| this.request_delete(id, window, cx));
                        });
                    }),
                )
            }),
            window,
            cx,
        );
    }

    fn on_thread_row_menu_visibility(
        &mut self,
        id: SessionId,
        draft: bool,
        archived: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if draft {
            self.discard_session_draft(id, window, cx);
        } else {
            self.set_archived(id, !archived, cx);
        }
    }
}

#[cfg(test)]
mod tests;
