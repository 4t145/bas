use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::component::spinner::Spinner;

use super::*;

const STATUS_SIZE: f32 = 12.;
const UNREAD_SIZE: f32 = 6.;
const STOPPED_WIDTH: f32 = 8.;
const STOPPED_HEIGHT: f32 = 1.;

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
            Self::Starting => "启动中",
            Self::Working => "运行中",
            Self::Unread => "有未读回复",
            Self::Idle => "就绪",
            Self::Failed => "失败",
            Self::Stopped => "已停止",
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
        let archived = session.archived;
        let pinned = self.pinned.contains(&id);
        let disabled = !self.connected
            || self.archive_pending.contains(&id)
            || self.delete_pending.contains(&id);
        let state = RowStatus::for_session(
            session,
            self.seen_replies.get(&id).copied().unwrap_or(0),
            self.active == Some(id) && self.has_unread_reply(),
        );
        let group = SharedString::from(format!("thread-row-{id}"));
        let owner = cx.entity().downgrade();
        div()
            .id(SharedString::from(format!("session-{id}")))
            .test_support()
            .group(group.clone())
            .ml_5()
            .px_3()
            .py_2()
            .mb_1()
            .rounded_md()
            .cursor_pointer()
            .flex()
            .items_center()
            .gap_2()
            .when(self.active == Some(id), |row| {
                row.bg(cx.theme().sidebar_accent)
            })
            .hover(|row| row.bg(cx.theme().sidebar_accent))
            .on_click(cx.listener(move |this, _, window, cx| this.select(id, window, cx)))
            .child(
                div()
                    .id(SharedString::from(format!("thread-title-{id}")))
                    .test_support()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(div().min_w_0().truncate().text_sm().child(title(session)))
                    .when(pinned, |title| {
                        title.child(Icon::new(IconName::Pin).xsmall().flex_shrink_0())
                    }),
            )
            .child(
                div()
                    .id(SharedString::from(format!("thread-rail-{id}")))
                    .test_support()
                    .relative()
                    .flex_shrink_0()
                    .size(px(layout::ICON_BUTTON_SIZE))
                    .child(
                        div()
                            .group_hover(group.clone(), |style| style.invisible())
                            .child(state.slot(id, cx)),
                    )
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .invisible()
                            .group_hover(group, |style| style.visible())
                            .child(
                                self.archive_button(session, "archive", cx)
                                    .disabled(disabled),
                            ),
                    ),
            )
            .on_mouse_down(
                gpui_kit::MouseButton::Right,
                cx.listener(move |this, event: &gpui_kit::MouseDownEvent, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                    let owner = owner.clone();
                    this.open_context_menu(
                        event.position,
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
                                    "取消置顶"
                                } else {
                                    "置顶 thread"
                                })
                                .icon(IconName::Pin)
                                .on_click(move |_, window, cx| {
                                    menus::in_parent(parent, window, cx, |_, cx| {
                                        let _ = pin_owner
                                            .update(cx, |this, cx| this.toggle_pinned(id, cx));
                                    });
                                }),
                            )
                            .item(PopupMenuItem::new("重命名").disabled(disabled).on_click(
                                move |_, window, cx| {
                                    menus::in_parent(parent, window, cx, |window, cx| {
                                        let _ = rename_owner.update(cx, |this, cx| {
                                            this.select(id, window, cx);
                                            this.begin_rename(window, cx);
                                        });
                                    });
                                },
                            ))
                            .item(
                                PopupMenuItem::new(if archived {
                                    "恢复 thread"
                                } else {
                                    "归档 thread"
                                })
                                .disabled(disabled)
                                .on_click(move |_, window, cx| {
                                    menus::in_parent(parent, window, cx, |_, cx| {
                                        let _ = archive_owner.update(cx, |this, cx| {
                                            this.set_archived(id, !archived, cx)
                                        });
                                    });
                                }),
                            )
                            .when(state == RowStatus::Unread, |menu| {
                                menu.item(PopupMenuItem::new("标为已读").on_click(
                                    move |_, window, cx| {
                                        menus::in_parent(parent, window, cx, |_, cx| {
                                            let _ = read_owner.update(cx, |this, cx| {
                                                this.mark_thread_read(id, cx)
                                            });
                                        });
                                    },
                                ))
                            })
                            .separator()
                            .item(
                                PopupMenuItem::new("删除 thread…")
                                    .disabled(disabled)
                                    .on_click(move |_, window, cx| {
                                        menus::in_parent(parent, window, cx, |window, cx| {
                                            let _ = delete_owner.update(cx, |this, cx| {
                                                this.request_delete(id, window, cx)
                                            });
                                        });
                                    }),
                            )
                        }),
                        window,
                        cx,
                    );
                }),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests;
