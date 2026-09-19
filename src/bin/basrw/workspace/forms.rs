use super::*;
use gpui_kit::component::menu::PopupMenuItem;

impl Workspace {
    pub(super) fn render_header(&self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self.current();
        div()
            .flex()
            .flex_col()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .child(
                        Button::new("sidebar-toggle")
                            .ghost()
                            .small()
                            .icon(IconName::PanelLeft)
                            .tooltip("Threads · Ctrl B")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.toggle_sidebar(window, cx)),
                            ),
                    )
                    .child(self.workspace_button(cx))
                    .child(caption("/", cx))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .child(current.map(title).unwrap_or_else(|| "新 thread".into())),
                    )
                    .when_some(current, |row, session| {
                        row.child(
                            Button::new("copy-session-id")
                                .ghost()
                                .xsmall()
                                .icon(IconName::Copy)
                                .tooltip(format!("复制 thread ID · {}", session.id))
                                .on_click({
                                    let id = session.id.to_string();
                                    move |_, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            id.clone(),
                                        ));
                                    }
                                }),
                        )
                    })
                    .when(self.active.is_some(), |row| {
                        row.child(
                            Button::new("rename-session")
                                .ghost()
                                .xsmall()
                                .icon(IconName::Pencil)
                                .tooltip("重命名 thread")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    let text = this.current().map(title).unwrap_or_default();
                                    this.editing_title = !this.editing_title;
                                    if this.editing_title {
                                        this.rename.update(cx, |input, cx| {
                                            input.set_value(text, window, cx);
                                            input.focus(window, cx);
                                        });
                                    } else {
                                        this.focus_input(window, cx);
                                    }
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("delete-session")
                                .ghost()
                                .xsmall()
                                .icon(IconName::Trash)
                                .tooltip("删除 thread")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.deleting = this.active;
                                    cx.notify();
                                })),
                        )
                    })
                    .child(
                        Button::new("refresh")
                            .ghost()
                            .xsmall()
                            .icon(IconName::RefreshCw)
                            .tooltip("刷新 · F5")
                            .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                    )
                    .child(self.mode_button(cx)),
            )
            .when(self.editing_title, |header| {
                header.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_5()
                        .pb_3()
                        .child(Input::new(&self.rename))
                        .child(
                            Button::new("save-title")
                                .primary()
                                .small()
                                .label("保存")
                                .disabled(
                                    self.renaming || self.rename.read(cx).value().trim().is_empty(),
                                )
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(session) = this.active {
                                        this.renaming = true;
                                        this.command(Command::Rename {
                                            session,
                                            title: this.rename.read(cx).value().trim().to_owned(),
                                        });
                                        cx.notify();
                                    }
                                })),
                        )
                        .child(
                            Button::new("cancel-rename")
                                .ghost()
                                .small()
                                .label("取消")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.editing_title = false;
                                    this.focus_input(window, cx);
                                    cx.notify();
                                })),
                        ),
                )
            })
            .when_some(self.deleting, |header, session| {
                header.child(
                    div()
                        .px_5()
                        .pb_3()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(
                            div()
                                .flex_1()
                                .text_color(cx.theme().danger)
                                .child("删除会话会停止 Agent，并移除 basrd 中的会话记录。"),
                        )
                        .child(
                            Button::new("confirm-delete")
                                .danger()
                                .small()
                                .label("确认删除")
                                .disabled(self.delete_pending)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.delete_pending = true;
                                    this.command(Command::Delete(session));
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("cancel-delete")
                                .ghost()
                                .small()
                                .label("取消")
                                .disabled(self.delete_pending)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.deleting = None;
                                    cx.notify();
                                })),
                        ),
                )
            })
    }

    pub(super) fn workspace_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let groups = groups::group_threads(&self.sessions, &self.directories, "");
        let directories: Vec<_> = groups.into_iter().map(|group| group.directory).collect();
        let selected = self.selected_directory.clone();
        let view = cx.entity().downgrade();
        let parent = self.window;
        let button = Button::new("workspace-picker")
            .ghost()
            .xsmall()
            .icon(IconName::Folder)
            .when(!self.compact, |button| {
                button.label(groups::name(selected.as_deref()))
            })
            .tooltip(groups::path_label(selected.as_deref()));
        self.menu_button(
            button,
            directories.len() + 2,
            false,
            move |mut menu, _, _| {
                for directory in &directories {
                    let directory = directory.clone();
                    let view = view.clone();
                    menu = menu.item(
                        PopupMenuItem::new(groups::path_label(directory.as_deref()))
                            .checked(directory == selected)
                            .on_click(move |_, window, cx| {
                                menus::in_parent(parent, window, cx, |window, cx| {
                                    let _ = view.update(cx, |this, cx| {
                                        this.new_thread(directory.clone(), window, cx)
                                    });
                                });
                            }),
                    );
                }
                menu.separator().item(
                    PopupMenuItem::new("添加 workspace…")
                        .icon(IconName::Plus)
                        .on_click({
                            let view = view.clone();
                            move |_, window, cx| {
                                menus::in_parent(parent, window, cx, |window, cx| {
                                    let _ =
                                        view.update(cx, |this, cx| this.edit_workspace(window, cx));
                                });
                            }
                        }),
                )
            },
            cx,
        )
    }

    pub(super) fn agent_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let agents: Vec<_> = self.agents.iter().map(|agent| agent.id.clone()).collect();
        let selected = self
            .current()
            .map(|session| session.agent.to_string())
            .unwrap_or_else(|| self.selected_agent.clone());
        let view = cx.entity().downgrade();
        let parent = self.window;
        let disabled = self.active.is_some()
            && (self
                .conversation
                .as_ref()
                .is_none_or(|c| !c.items.is_empty())
                || self.working()
                || self.active.is_some_and(|id| self.sending.contains(&id)));
        let button = Button::new("agent-picker")
            .ghost()
            .xsmall()
            .icon(IconName::Bot)
            .tooltip(if selected.is_empty() {
                "无可用 Agent".into()
            } else {
                format!("切换 Agent · {selected}")
            })
            .disabled(disabled);
        self.menu_button(
            button,
            agents.len(),
            disabled,
            move |mut menu, _, _| {
                for id in &agents {
                    let id = id.clone();
                    let view = view.clone();
                    menu = menu.item(
                        PopupMenuItem::new(id.clone())
                            .checked(id == selected)
                            .on_click(move |_, window, cx| {
                                menus::in_parent(parent, window, cx, |window, cx| {
                                    let _ = view.update(cx, |this, cx| {
                                        this.select_agent(id.clone(), window, cx);
                                    });
                                });
                            }),
                    );
                }
                menu
            },
            cx,
        )
    }

    pub(super) fn edit_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.compact {
            self.toggle_mode(window, cx);
        }
        self.editing_workspace = true;
        let path = self
            .selected_directory
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.cwd.update(cx, |input, cx| {
            input.set_value(path, window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    pub(super) fn render_workspace_editor(&self, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(caption("Workspace · basrd 所在机器上的绝对目录路径", cx))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(Input::new(&self.cwd).id("workspace-directory"))
                    .child(
                        Button::new("add-workspace")
                            .primary()
                            .small()
                            .label("打开")
                            .on_click(cx.listener(|this, _, window, cx| {
                                let path = PathBuf::from(this.cwd.read(cx).value().trim());
                                if !path.is_absolute() {
                                    this.error = Some("请输入绝对目录路径。".into());
                                    cx.notify();
                                    return;
                                }
                                this.new_thread(Some(groups::normalize(&path)), window, cx);
                            })),
                    )
                    .child(
                        Button::new("cancel-workspace")
                            .ghost()
                            .small()
                            .label("取消")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.editing_workspace = false;
                                this.focus_input(window, cx);
                                cx.notify();
                            })),
                    ),
            )
    }
}

impl Workspace {
    fn configure(
        &mut self,
        request: basr::acp::SetSessionConfigOptionRequest,
        cx: &mut Context<Self>,
    ) {
        let session = request
            .session_id
            .to_string()
            .parse()
            .expect("GUI uses daemon session ids");
        if self.configuring.insert(session) {
            self.command(Command::SetConfig(request));
            cx.notify();
        }
    }

    pub(super) fn render_config_options(&self, cx: &mut Context<Self>) -> impl IntoElement {
        use basr::acp;
        let mut row = div()
            .id("session-options")
            .flex()
            .items_center()
            .w_full()
            .min_w_0()
            .gap_1()
            .overflow_x_scroll()
            .child(div().flex_1());
        if self.creating {
            return row.child(
                Button::new("options-loading")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Loader)
                    .tooltip("正在初始化 Agent 选项和命令…"),
            );
        }
        let Some(session) = self.current() else {
            return row;
        };
        let Some(options) = session
            .acp
            .as_ref()
            .and_then(|info| info.config_options.as_ref())
        else {
            return row;
        };
        for option in options {
            let session = session.id;
            let config_id = option.id.clone();
            let view = cx.entity().downgrade();
            let id = SharedString::from(format!("config-{}", option.id));
            match &option.kind {
                acp::SessionConfigKind::Select(select) => {
                    let choices: Vec<_> = match &select.options {
                        acp::SessionConfigSelectOptions::Ungrouped(options) => options.clone(),
                        acp::SessionConfigSelectOptions::Grouped(groups) => groups
                            .iter()
                            .flat_map(|group| group.options.clone())
                            .collect(),
                        _ => continue,
                    };
                    let current = select.current_value.clone();
                    let name = choices
                        .iter()
                        .find(|value| value.value == current)
                        .map(|value| value.name.clone())
                        .unwrap_or_else(|| current.to_string());
                    row = row.child(
                        self.menu_button(
                            Button::new(id)
                                .flex_shrink_0()
                                .disabled(self.configuring.contains(&session))
                                .ghost()
                                .xsmall()
                                .label(name)
                                .tooltip(
                                    option
                                        .description
                                        .as_ref()
                                        .map(|description| {
                                            format!("{} · {description}", option.name)
                                        })
                                        .unwrap_or_else(|| option.name.clone()),
                                )
                                .max_w(px(130.))
                                .icon(IconName::ChevronDown),
                            choices.len(),
                            self.configuring.contains(&session),
                            move |mut menu, _, _| {
                                for choice in &choices {
                                    let view = view.clone();
                                    let request = acp::SetSessionConfigOptionRequest::new(
                                        session.to_string(),
                                        config_id.clone(),
                                        choice.value.clone(),
                                    );
                                    menu = menu.item(
                                        PopupMenuItem::new(choice.name.clone())
                                            .checked(choice.value == current)
                                            .on_click(move |_, _, cx| {
                                                let _ = view.update(cx, |this, cx| {
                                                    this.configure(request.clone(), cx);
                                                });
                                            }),
                                    );
                                }
                                menu
                            },
                            cx,
                        ),
                    );
                }
                acp::SessionConfigKind::Boolean(boolean) => {
                    let request = acp::SetSessionConfigOptionRequest::new(
                        session.to_string(),
                        config_id,
                        !boolean.current_value,
                    );
                    row = row.child(
                        Button::new(id)
                            .flex_shrink_0()
                            .disabled(self.configuring.contains(&session))
                            .ghost()
                            .xsmall()
                            .icon(match option.category.as_ref() {
                                Some(acp::SessionConfigOptionCategory::ThoughtLevel) => {
                                    IconName::Brain
                                }
                                Some(acp::SessionConfigOptionCategory::Model) => IconName::Cpu,
                                Some(acp::SessionConfigOptionCategory::Mode) => IconName::Shield,
                                _ => IconName::Settings,
                            })
                            .when(boolean.current_value, |button| {
                                button.text_color(cx.theme().primary)
                            })
                            .tooltip(format!(
                                "{} · {}",
                                option.name,
                                if boolean.current_value {
                                    "开启"
                                } else {
                                    "关闭"
                                }
                            ))
                            .on_click(move |_, _, cx| {
                                let _ = view.update(cx, |this, cx| {
                                    this.configure(request.clone(), cx);
                                });
                            }),
                    );
                }
                _ => {}
            }
        }
        row
    }
}
