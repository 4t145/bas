use basr::model::Timestamp;
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::component::spinner::Spinner;

use super::*;

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
        let indicator = div()
            .id(SharedString::from(format!("thread-indicator-{id}")))
            .test_support()
            .flex()
            .items_center()
            .justify_center()
            .flex_shrink_0();
        match self {
            Self::Starting | Self::Working => indicator.size(px(STATUS_SIZE)).child(
                Spinner::new()
                    .icon(IconName::LoaderCircle)
                    .with_size(px(STATUS_SIZE))
                    .color(cx.theme().primary),
            ),
            Self::Unread => indicator
                .size(px(UNREAD_SIZE))
                .rounded_full()
                .bg(cx.theme().primary),
            Self::Idle => indicator.size_0(),
            Self::Failed => indicator.size(px(STATUS_SIZE)).child(
                Icon::new(IconName::TriangleAlert)
                    .with_size(px(STATUS_SIZE))
                    .text_color(cx.theme().danger),
            ),
            Self::Stopped => indicator
                .w(px(STOPPED_WIDTH))
                .h(px(STOPPED_HEIGHT))
                .bg(cx.theme().muted_foreground),
        }
        .into_any_element()
    }

    fn slot(self, id: SessionId, cx: &App) -> Button {
        Button::new(SharedString::from(format!("thread-status-{id}")))
            .ghost()
            .xsmall()
            .w(px(layout::ICON_BUTTON_SIZE))
            .h(px(layout::ICON_BUTTON_SIZE))
            .p_0()
            .flex_shrink_0()
            .tab_stop(false)
            .role(gpui_kit::Role::Status)
            .accessibility_label(self.label())
            .tooltip(self.label())
            .child(self.indicator(id, cx))
    }
}

impl Workspace {
    pub(super) fn thread_row(&self, session: &SessionMeta, cx: &mut Context<Self>) -> AnyElement {
        let id = session.id;
        let editing = self.renaming_thread(id);
        let pinned = self.pinned.contains(&id);
        let disabled = !self.connected
            || self.archive_pending.contains(&id)
            || self.delete_pending.contains(&id);
        let state = if !self.runtime_healthy(id) {
            RowStatus::Failed
        } else {
            RowStatus::for_session(
                session,
                self.seen_replies.get(&id).copied().unwrap_or(0),
                self.active == Some(id) && self.has_unread_reply(),
            )
        };
        let group = SharedString::from(format!("thread-row-{id}"));
        let menu_session = session.clone();
        div()
            .id(SharedString::from(format!("session-{id}")))
            .test_support()
            .group(group.clone())
            .w_full()
            .px_3()
            .py_2()
            .cursor_pointer()
            .flex()
            .items_start()
            .gap_2()
            .when(self.active == Some(id), |row| {
                row.bg(cx.theme().sidebar_accent)
            })
            .hover(|row| row.bg(cx.theme().sidebar_accent))
            .on_click(cx.listener(move |this, _, window, cx| this.select(id, window, cx)))
            .child(
                div()
                    .h(px(layout::ICON_BUTTON_SIZE))
                    .flex()
                    .items_center()
                    .flex_shrink_0()
                    .child(self.agent_icon(
                        format!("thread-agent-{id}"),
                        session.agent.as_str(),
                        cx,
                    )),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .id(SharedString::from(format!("thread-title-{id}")))
                            .test_support()
                            .h(px(layout::ICON_BUTTON_SIZE))
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap_1()
                            .when(!editing, |title| {
                                title.group_hover(group.clone(), |style| {
                                    style.pr(px(ACTIONS_WIDTH + ACTIONS_GAP))
                                })
                            })
                            .when(editing, |title| title.child(self.render_title_editor(cx)))
                            .when(!editing, |row| {
                                row.child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .text_size(crate::ui::SMALL)
                                        .child(title(session)),
                                )
                            })
                            .when(pinned && !editing, |title| {
                                title.child(Icon::new(IconName::Pin).xsmall().flex_shrink_0())
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .h(px(layout::ICON_BUTTON_SIZE))
                            .child(
                                div()
                                    .id(SharedString::from(format!("thread-time-{id}")))
                                    .test_support()
                                    .text_size(crate::ui::SMALL)
                                    .text_color(cx.theme().muted_foreground)
                                    .child(recent_time(
                                        session.agent_updated_at.unwrap_or(session.updated_at),
                                        Timestamp::now(),
                                    )),
                            )
                            .child(
                                div()
                                    .id(SharedString::from(format!("thread-rail-{id}")))
                                    .test_support()
                                    .size(px(layout::ICON_BUTTON_SIZE))
                                    .flex_shrink_0()
                                    .child(state.slot(id, cx)),
                            ),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("thread-actions-{id}")))
                            .test_support()
                            .absolute()
                            .top_0()
                            .right_0()
                            .flex()
                            .items_center()
                            .w(px(ACTIONS_WIDTH))
                            .h(px(layout::ICON_BUTTON_SIZE))
                            .invisible()
                            .when(!editing, |actions| {
                                actions.group_hover(group, |style| style.visible())
                            })
                            .child(
                                layout::icon_button(
                                    SharedString::from(format!("rename-{id}")),
                                    IconName::Pencil,
                                )
                                .accessibility_label(crate::i18n::tr!("session-rename"))
                                .tooltip(crate::i18n::tr!("session-rename"))
                                .disabled(disabled)
                                .on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.begin_sidebar_rename(id, window, cx);
                                    },
                                )),
                            )
                            .child(self.thread_visibility_button(session, cx)),
                    ),
            )
            .on_mouse_down(
                gpui_kit::MouseButton::Right,
                cx.listener(move |this, event: &gpui_kit::MouseDownEvent, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                    this.open_thread_menu(&menu_session, event.position, window, cx);
                }),
            )
            .into_any_element()
    }

    fn thread_visibility_button(&self, session: &SessionMeta, cx: &mut Context<Self>) -> Button {
        let id = session.id;
        if session.archived || !self.session_is_draft(id) {
            return self.archive_button(session, "archive", cx);
        }
        layout::icon_button(SharedString::from(format!("discard-{id}")), IconName::Trash)
            .accessibility_label(crate::i18n::tr!("draft-discard"))
            .tooltip(crate::i18n::tr!("draft-discard"))
            .disabled(
                !self.connected
                    || self.archive_pending.contains(&id)
                    || self.delete_pending.contains(&id),
            )
            .loading(self.delete_pending.contains(&id))
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                this.discard_session_draft(id, window, cx);
            }))
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
        let draft = !archived && self.session_is_draft(id);
        let pinned = self.pinned.contains(&id);
        let disabled = !self.connected
            || self.archive_pending.contains(&id)
            || self.delete_pending.contains(&id);
        let state = RowStatus::for_session(
            session,
            self.seen_replies.get(&id).copied().unwrap_or(0),
            self.active == Some(id) && self.has_unread_reply(),
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
                    PopupMenuItem::new(if pinned {
                        crate::i18n::tr!("session-unpin")
                    } else {
                        crate::i18n::tr!("session-pin")
                    })
                    .icon(IconName::Pin)
                    .on_click(move |_, window, cx| {
                        menus::in_parent(parent, window, cx, |_, cx| {
                            let _ = pin_owner.update(cx, |this, cx| this.toggle_pinned(id, cx));
                        });
                    }),
                )
                .item(
                    PopupMenuItem::new(crate::i18n::tr!("session-rename"))
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
                    PopupMenuItem::new(if draft {
                        crate::i18n::tr!("draft-discard")
                    } else if archived {
                        crate::i18n::tr!("session-restore")
                    } else {
                        crate::i18n::tr!("session-archive")
                    })
                    .icon(if draft {
                        IconName::Trash
                    } else {
                        IconName::Archive
                    })
                    .disabled(disabled)
                    .on_click(move |_, window, cx| {
                        menus::in_parent(parent, window, cx, |window, cx| {
                            let _ = archive_owner.update(cx, |this, cx| {
                                if draft {
                                    this.discard_session_draft(id, window, cx);
                                } else {
                                    this.set_archived(id, !archived, cx);
                                }
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
                    PopupMenuItem::new(crate::i18n::tr!("session-delete"))
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
}

#[cfg(test)]
mod tests;
