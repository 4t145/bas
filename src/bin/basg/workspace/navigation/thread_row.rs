use crate::ui::session_status::Status as RowStatus;
use bas::model::Timestamp;
use gpui_kit::component::menu::PopupMenuItem;

use crate::ui::time::recent_time;
use crate::workspace::*;

pub(super) const ACTIONS_WIDTH: f32 = layout::ICON_BUTTON_SIZE * 2.;
pub(super) const ACTIONS_GAP: f32 = 4.;

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
                } [$state.render(format!("thread-status-{id}"), format!("thread-indicator-{id}"), cx)],
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
        let can_close = !archived
            && !matches!(
                session.status,
                SessionStatus::Stopped | SessionStatus::Failed
            );
        let stopped = !archived && session.status == SessionStatus::Stopped;
        let can_restart = self.can_restart_agent_session(id);
        let runtime_busy = self.sessions.pending.closing.contains(&id)
            || self.sessions.pending.restarting.contains(&id);
        let owner = cx.entity().downgrade();
        self.open_context_menu(
            position,
            (if state == RowStatus::Unread { 6 } else { 5 }) - usize::from(draft)
                + usize::from(can_close)
                + usize::from(stopped),
            std::rc::Rc::new(move |menu, window, _| {
                let parent = window.window_handle();
                let pin_owner = owner.clone();
                let rename_owner = owner.clone();
                let archive_owner = owner.clone();
                let delete_owner = owner.clone();
                let read_owner = owner.clone();
                let close_owner = owner.clone();
                let restart_owner = owner.clone();
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
                        (!draft).then(|| {
                            Box::new(shortcuts::ArchiveThread) as Box<dyn gpui_kit::Action>
                        }),
                    )
                    .icon(if draft {
                        IconName::Trash
                    } else if archived {
                        IconName::ArchiveRestore
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
                .when(can_close, |menu| {
                    menu.item(
                        shortcuts::menu_item(
                            crate::i18n::tr!("agent-session-close"),
                            Some(Box::new(shortcuts::ToggleSession)),
                        )
                        .icon(IconName::Square)
                        .disabled(disabled || runtime_busy)
                        .on_click(move |_, window, cx| {
                            menus::in_parent(parent, window, cx, |_, cx| {
                                let _ = close_owner.update(cx, |this, cx| {
                                    this.request_close_agent_session(id, cx);
                                });
                            });
                        }),
                    )
                })
                .when(stopped, |menu| {
                    menu.item(
                        shortcuts::menu_item(
                            crate::i18n::tr!("agent-session-start"),
                            Some(Box::new(shortcuts::ToggleSession)),
                        )
                        .icon(IconName::Play)
                        .disabled(!can_restart)
                        .on_click(move |_, window, cx| {
                            menus::in_parent(parent, window, cx, |_, cx| {
                                let _ = restart_owner.update(cx, |this, cx| {
                                    this.request_restart_agent_session(id, cx);
                                });
                            });
                        }),
                    )
                })
                .when(!draft, |menu| {
                    menu.separator().item(
                        shortcuts::menu_item(
                            crate::i18n::tr!("session-delete"),
                            Some(Box::new(shortcuts::DeleteThread)),
                        )
                        .icon(IconName::Trash)
                        .disabled(disabled)
                        .on_click(move |_, window, cx| {
                            menus::in_parent(parent, window, cx, |window, cx| {
                                let _ = delete_owner
                                    .update(cx, |this, cx| this.request_delete(id, window, cx));
                            });
                        }),
                    )
                })
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
