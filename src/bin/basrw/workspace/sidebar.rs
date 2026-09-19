use super::*;

impl Workspace {
    pub(super) fn render_sidebar(
        &self,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let query = self.search.read(cx).value().to_lowercase();
        let groups = groups::group_threads(&self.sessions, &self.directories, &query);
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
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_4()
                    .py_3()
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child("Threads"),
                    )
                    .child(
                        Button::new("add-workspace-button")
                            .ghost()
                            .xsmall()
                            .icon(IconName::FolderPlus)
                            .tooltip("添加 workspace")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.edit_workspace(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("new-session")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Plus)
                            .tooltip("新 thread · Ctrl N")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.new_session(window, cx)),
                            ),
                    ),
            )
            .child(
                div().px_3().pb_3().child(
                    Input::new(&self.search)
                        .small()
                        .prefix(Icon::new(IconName::Search).small())
                        .cleanable(true),
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
                                    .flex()
                                    .items_center()
                                    .gap_1()
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
                                            .tooltip(move |window, cx| {
                                                gpui_kit::component::tooltip::Tooltip::new(
                                                    path_label.clone(),
                                                )
                                                .build(window, cx)
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
                                        Button::new(SharedString::from(format!(
                                            "new-thread-{index}"
                                        )))
                                        .ghost()
                                        .xsmall()
                                        .icon(IconName::Plus)
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
                                                .ml_5()
                                                .px_3()
                                                .py_2()
                                                .rounded_md()
                                                .bg(cx.theme().sidebar_accent)
                                                .text_sm()
                                                .child("新 thread"),
                                        )
                                    })
                                    .when(group.threads.is_empty() && !draft_selected, |column| {
                                        column.child(
                                            caption("还没有 thread", cx).ml_5().px_3().py_2(),
                                        )
                                    })
                                    .children(group.threads.into_iter().map(|session| {
                                        let id = session.id;
                                        let selected = self.active == Some(id);
                                        div()
                                            .id(SharedString::from(format!("session-{id}")))
                                            .ml_5()
                                            .px_3()
                                            .py_2()
                                            .mb_1()
                                            .rounded_md()
                                            .cursor_pointer()
                                            .when(selected, |row| row.bg(cx.theme().sidebar_accent))
                                            .hover(|row| row.bg(cx.theme().sidebar_accent))
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.select(id, window, cx)
                                            }))
                                            .child(
                                                div()
                                                    .flex()
                                                    .items_center()
                                                    .gap_2()
                                                    .child(
                                                        div()
                                                            .flex_1()
                                                            .min_w_0()
                                                            .truncate()
                                                            .text_sm()
                                                            .child(title(session)),
                                                    )
                                                    .when(
                                                        session.status == SessionStatus::Working,
                                                        |row| {
                                                            row.child(
                                                                div()
                                                                    .size_2()
                                                                    .rounded_full()
                                                                    .bg(cx.theme().primary),
                                                            )
                                                        },
                                                    ),
                                            )
                                            .when(
                                                session.status == SessionStatus::Failed
                                                    || session.status == SessionStatus::Stopped,
                                                |row| {
                                                    row.child(caption(status(session.status), cx))
                                                },
                                            )
                                    }))
                            })
                    })),
            )
            .child(
                div()
                    .px_4()
                    .py_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(self.connection_indicator("sidebar-connection", cx))
                    .child(div().flex_1())
                    .child(
                        Button::new("connection-details")
                            .ghost()
                            .xsmall()
                            .label(
                                cx.try_global::<theme::Appearance>()
                                    .map(|theme| theme.name.replace("DankShell ", ""))
                                    .unwrap_or_else(|| "System".into()),
                            )
                            .tooltip(format!("{} · 自动跟随 DMS / Zed 主题", self.server)),
                    ),
            )
    }
}
