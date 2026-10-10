use basr::model::Timestamp;
use gpui_kit::component::menu::PopupMenuItem;

use super::*;

pub(super) struct Metadata {
    pub title: Option<String>,
    pub updated_at: Timestamp,
}

impl Default for Metadata {
    fn default() -> Self {
        Self {
            title: None,
            updated_at: Timestamp::now(),
        }
    }
}

impl Workspace {
    pub(super) fn draft_title(&self) -> String {
        self.new_draft_metadata
            .get(&self.selected_directory)
            .and_then(|draft| draft.title.clone())
            .unwrap_or_else(|| "新 thread".into())
    }

    pub(super) fn discard_new_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.active.is_some() || self.creating || self.paste_pending {
            return;
        }
        let directory = self.selected_directory.clone();
        if self
            .title_edit
            .as_ref()
            .is_some_and(|edit| edit.target == rename::Target::Draft(directory.clone()))
        {
            self.title_edit = None;
        } else {
            self.save_title(cx);
        }
        self.composer_generation += 1;
        self.attachments.clear();
        self.error = None;
        self.composer.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        self.new_drafts.remove(&directory);
        self.new_draft_metadata.remove(&directory);
        let next = self
            .sessions
            .iter()
            .filter(|session| !session.archived)
            .min_by_key(|session| session.cwd.as_deref().map(groups::normalize) != directory)
            .map(|session| session.id);
        if let Some(next) = next {
            self.select(next, window, cx);
            self.new_drafts.remove(&directory);
            self.new_draft_metadata.remove(&directory);
        } else {
            self.waiting_for_agent_selection = true;
            self.focus_input(window, cx);
        }
        cx.notify();
    }

    pub(super) fn draft_thread_row(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let editing = self.renaming_draft();
        let group = SharedString::from(format!("draft-thread-{index}"));
        let now = Timestamp::now();
        let updated_at = self
            .new_draft_metadata
            .get(&self.selected_directory)
            .map_or(now, |draft| draft.updated_at);
        let disabled = self.creating || self.paste_pending;
        let directory = self.selected_directory.clone();
        div()
            .id(group.clone())
            .test_support()
            .group(group.clone())
            .w_full()
            .px_3()
            .py_2()
            .bg(cx.theme().sidebar_accent)
            .hover(|row| row.bg(cx.theme().sidebar_accent))
            .flex()
            .items_start()
            .gap_2()
            .child(
                div()
                    .h(px(layout::ICON_BUTTON_SIZE))
                    .flex()
                    .items_center()
                    .flex_shrink_0()
                    .child(self.agent_icon(
                        format!("draft-thread-agent-{index}"),
                        &self.selected_agent,
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
                            .id(SharedString::from(format!("draft-thread-title-{index}")))
                            .test_support()
                            .h(px(layout::ICON_BUTTON_SIZE))
                            .flex()
                            .items_center()
                            .min_w_0()
                            .when(!editing, |title| {
                                title.group_hover(group.clone(), |title| {
                                    title
                                        .pr(px(thread_row::ACTIONS_WIDTH + thread_row::ACTIONS_GAP))
                                })
                            })
                            .when(editing, |title| title.child(self.render_title_editor(cx)))
                            .when(!editing, |title| {
                                title.child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .text_sm()
                                        .child(self.draft_title()),
                                )
                            }),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("draft-thread-time-{index}")))
                            .test_support()
                            .h(px(layout::ICON_BUTTON_SIZE))
                            .flex()
                            .items_center()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(thread_row::recent_time(updated_at, now)),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("draft-thread-actions-{index}")))
                            .test_support()
                            .absolute()
                            .top_0()
                            .right_0()
                            .flex()
                            .invisible()
                            .when(!editing, |actions| {
                                actions.group_hover(group, |actions| actions.visible())
                            })
                            .child(
                                layout::icon_button(
                                    SharedString::from(format!("rename-draft-{index}")),
                                    IconName::Pencil,
                                )
                                .accessibility_label("修改标题")
                                .tooltip("修改标题")
                                .disabled(disabled)
                                .on_click(cx.listener(
                                    |this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.begin_draft_rename(window, cx);
                                    },
                                )),
                            )
                            .child(
                                layout::icon_button(
                                    SharedString::from(format!("discard-draft-{index}")),
                                    IconName::Trash,
                                )
                                .accessibility_label("废弃草稿")
                                .tooltip("废弃草稿")
                                .disabled(disabled)
                                .on_click(cx.listener(
                                    |this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.discard_new_draft(window, cx);
                                    },
                                )),
                            ),
                    ),
            )
            .on_mouse_down(
                gpui_kit::MouseButton::Right,
                cx.listener(move |this, event: &gpui_kit::MouseDownEvent, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                    this.open_context_menu(
                        event.position,
                        2,
                        this.draft_menu(directory.clone(), disabled, cx),
                        window,
                        cx,
                    );
                }),
            )
            .into_any_element()
    }

    fn draft_menu(
        &self,
        directory: Option<PathBuf>,
        disabled: bool,
        cx: &Context<Self>,
    ) -> menus::Builder {
        let owner = cx.entity().downgrade();
        let generation = self.composer_generation;
        std::rc::Rc::new(move |menu, window, _| {
            let parent = window.window_handle();
            let rename_owner = owner.clone();
            let rename_directory = directory.clone();
            let discard_owner = owner.clone();
            let discard_directory = directory.clone();
            menu.item(
                PopupMenuItem::new("修改标题")
                    .icon(IconName::Pencil)
                    .disabled(disabled)
                    .on_click(move |_, window, cx| {
                        menus::in_parent(parent, window, cx, |window, cx| {
                            let _ = rename_owner.update(cx, |this, cx| {
                                if this.active.is_none()
                                    && this.selected_directory == rename_directory
                                    && this.composer_generation == generation
                                {
                                    this.begin_draft_rename(window, cx);
                                }
                            });
                        });
                    }),
            )
            .item(
                PopupMenuItem::new("废弃草稿")
                    .icon(IconName::Trash)
                    .disabled(disabled)
                    .on_click(move |_, window, cx| {
                        menus::in_parent(parent, window, cx, |window, cx| {
                            let _ = discard_owner.update(cx, |this, cx| {
                                if this.selected_directory == discard_directory
                                    && this.composer_generation == generation
                                {
                                    this.discard_new_draft(window, cx);
                                }
                            });
                        });
                    }),
            )
        })
    }
}
