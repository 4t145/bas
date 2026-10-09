use super::*;

impl Workspace {
    pub(super) fn render_sidebar(
        &self,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let query = self.search.read(cx).value().to_lowercase();
        let sessions = self.visible_sessions();
        let groups = self.grouped_threads(&sessions, &query);
        div()
            .flex()
            .flex_col()
            .w(px(SIDEBAR_WIDTH))
            .flex_shrink_0()
            .h_full()
            .bg(cx.theme().sidebar)
            .border_r_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .id("sidebar-header")
                    .test_support()
                    .flex()
                    .flex_shrink_0()
                    .h(px(layout::HEADER_HEIGHT))
                    .items_center()
                    .gap_1()
                    .px_3()
                    .when(cfg!(target_os = "macos"), |header| {
                        header.pl(px(layout::MACOS_WINDOW_CONTROLS_WIDTH))
                    })
                    .child(
                        div()
                            .id("application-menu-slot")
                            .test_support()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .child(self.application_button(cx)),
                    )
                    .child(self.settings_button(cx))
                    .child(
                        layout::icon_button("add-workspace-button", IconName::FolderPlus)
                            .accessibility_label("添加 workspace")
                            .tooltip("添加 workspace")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.edit_workspace(window, cx)),
                            ),
                    )
                    .child(
                        layout::icon_button("new-session", IconName::Plus)
                            .accessibility_label("新 thread")
                            .tooltip("新 thread · Ctrl N")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.new_session(window, cx)),
                            ),
                    ),
            )
            .child(
                div()
                    .id("thread-search-row")
                    .test_support()
                    .px_3()
                    .pb_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .id("thread-search")
                            .test_support()
                            .flex_1()
                            .min_w_0()
                            .child(
                                Input::new(&self.search)
                                    .small()
                                    .prefix(Icon::new(IconName::Search).small())
                                    .cleanable(true),
                            ),
                    )
                    .child(
                        layout::icon_button("archived-list", IconName::Archive)
                            .selected(self.archived_list)
                            .accessibility_label(if self.archived_list {
                                "返回普通列表"
                            } else {
                                "查看已归档"
                            })
                            .tooltip(if self.archived_list {
                                "已归档 · 点击返回普通列表"
                            } else {
                                "查看已归档"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.archived_list = !this.archived_list;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .id("session-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_2()
                    .when(groups.is_empty(), |list| {
                        list.child(div().p_3().child(caption("没有匹配的 thread", cx)))
                    })
                    .children(groups.into_iter().enumerate().map(|(index, group)| {
                        let collapsed = query.is_empty()
                            && self.collapsed_workspaces.contains(&group.directory);
                        let directory = group.directory.clone();
                        let new_directory = directory.clone();
                        let path_label = groups::path_label(directory.as_deref());
                        let draft_selected =
                            self.active.is_none() && self.selected_directory == directory;
                        div()
                            .mb_3()
                            .child(
                                div()
                                    .id(SharedString::from(format!("workspace-header-{index}")))
                                    .test_support()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .pr_3()
                                    .child(
                                        div()
                                            .id(SharedString::from(format!("workspace-{index}")))
                                            .flex()
                                            .flex_1()
                                            .min_w_0()
                                            .items_center()
                                            .gap_2()
                                            .px_2()
                                            .py_2()
                                            .rounded_md()
                                            .cursor_pointer()
                                            .hover(|row| row.bg(cx.theme().sidebar_accent))
                                            .child(
                                                Icon::new(if collapsed {
                                                    IconName::ChevronRight
                                                } else {
                                                    IconName::ChevronDown
                                                })
                                                .xsmall(),
                                            )
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .truncate()
                                                    .child(groups::name(directory.as_deref())),
                                            )
                                            .hoverable_tooltip(move |window, cx| {
                                                crate::tooltip::view(path_label.clone(), window, cx)
                                            })
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                if !this.collapsed_workspaces.remove(&directory) {
                                                    this.collapsed_workspaces
                                                        .insert(directory.clone());
                                                }
                                                cx.notify();
                                            })),
                                    )
                                    .child(
                                        layout::icon_button(
                                            SharedString::from(format!("new-thread-{index}")),
                                            IconName::Plus,
                                        )
                                        .tooltip("在此 workspace 新建 thread")
                                        .on_click(
                                            cx.listener(move |this, _, window, cx| {
                                                this.new_thread(new_directory.clone(), window, cx)
                                            }),
                                        ),
                                    ),
                            )
                            .when(!collapsed, |column| {
                                column
                                    .when(draft_selected, |column| {
                                        column.child(
                                            div()
                                                .id(SharedString::from(format!(
                                                    "draft-thread-{index}"
                                                )))
                                                .test_support()
                                                .ml_5()
                                                .px_3()
                                                .py_2()
                                                .rounded_md()
                                                .bg(cx.theme().sidebar_accent)
                                                .flex()
                                                .items_center()
                                                .gap_2()
                                                .text_sm()
                                                .child(self.agent_icon(
                                                    format!("draft-thread-agent-{index}"),
                                                    &self.selected_agent,
                                                    cx,
                                                ))
                                                .child(
                                                    div()
                                                        .id(SharedString::from(format!(
                                                            "draft-thread-title-{index}"
                                                        )))
                                                        .test_support()
                                                        .flex_1()
                                                        .min_w_0()
                                                        .truncate()
                                                        .child("新 thread"),
                                                ),
                                        )
                                    })
                                    .when(group.threads.is_empty() && !draft_selected, |column| {
                                        column.child(
                                            caption("还没有 thread", cx).ml_5().px_3().py_2(),
                                        )
                                    })
                                    .children(
                                        group
                                            .threads
                                            .into_iter()
                                            .map(|session| self.thread_row(session, cx)),
                                    )
                            })
                    })),
            )
    }
}
