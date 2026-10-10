use super::*;
use gpui_kit::KeyDownEvent;
use gpui_kit::component::menu::PopupMenuItem;

pub(super) const TOOLBAR_AGENT_ICON_SIZE: f32 = 12.;

impl Workspace {
    pub(super) fn begin_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing_title || self.renaming || self.current().is_none() {
            return;
        }
        let text = self.current().map(title).unwrap_or_default();
        self.editing_title = true;
        self.rename.update(cx, |input, cx| {
            input.set_value(text, window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    pub(super) fn save_title(&mut self, cx: &mut Context<Self>) {
        if !self.editing_title || self.renaming {
            return;
        }
        let title = self.rename.read(cx).value().trim().to_owned();
        let Some(session) = self.active.filter(|_| !title.is_empty()) else {
            return;
        };
        self.renaming = true;
        self.error = None;
        self.command(Command::Rename { session, title });
        cx.notify();
    }

    pub(super) fn cancel_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editing_title = false;
        self.focus_input(window, cx);
        cx.notify();
    }

    fn title_slot(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("thread-title-slot")
            .test_support()
            .flex_1()
            .min_w_0()
            .h_8()
            .flex()
            .items_center()
            .gap_2()
            .when(self.editing_title, |slot| {
                slot.on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    if event.keystroke.key == "escape" {
                        this.cancel_rename(window, cx);
                        cx.stop_propagation();
                    }
                }))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(Input::new(&self.rename).small().disabled(self.renaming)),
                )
                .child(
                    Button::new("save-title")
                        .primary()
                        .small()
                        .label("保存")
                        .disabled(self.renaming || self.rename.read(cx).value().trim().is_empty())
                        .on_click(cx.listener(|this, _, _, cx| this.save_title(cx))),
                )
                .child(
                    Button::new("cancel-rename")
                        .ghost()
                        .small()
                        .label("取消")
                        .on_click(
                            cx.listener(|this, _, window, cx| this.cancel_rename(window, cx)),
                        ),
                )
            })
            .when(!self.editing_title, |slot| {
                slot.child(
                    div()
                        .id("rename-session")
                        .test_support()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_sm()
                        .cursor_pointer()
                        .child(
                            self.current()
                                .map(title)
                                .unwrap_or_else(|| "新 thread".into()),
                        )
                        .on_click(cx.listener(|this, _, window, cx| this.begin_rename(window, cx))),
                )
            })
    }

    pub(super) fn render_header(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let current = self.current();
        let controls = window.window_controls();
        let client_controls = !cfg!(target_os = "macos");
        div()
            .id("thread-header")
            .test_support()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .child(
                div()
                    .id("conversation-header-row")
                    .test_support()
                    .flex()
                    .flex_shrink_0()
                    .h(px(layout::HEADER_HEIGHT))
                    .items_center()
                    .gap_1()
                    .px_3()
                    .when(
                        cfg!(target_os = "macos") && !self.sidebar_visible,
                        |header| header.pl(px(layout::MACOS_WINDOW_CONTROLS_WIDTH)),
                    )
                    .when(!self.sidebar_visible, |row| {
                        row.child(self.application_button(cx))
                    })
                    .child(
                        layout::icon_button("sidebar-toggle", IconName::PanelLeft)
                            .tooltip("Threads · Ctrl B")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.toggle_sidebar(window, cx)),
                            ),
                    )
                    .child(self.workspace_button(cx))
                    .child(caption("/", cx))
                    .child(self.title_slot(cx))
                    .child(self.mode_button(cx))
                    .when(client_controls && controls.minimize, |row| {
                        row.child(
                            layout::icon_button("window-minimize", IconName::WindowMinimize)
                                .tooltip("最小化窗口")
                                .on_click(|_, window, _| window.minimize_window()),
                        )
                    })
                    .when(client_controls && controls.maximize, |row| {
                        row.child(
                            layout::icon_button(
                                "window-maximize",
                                if window.is_maximized() {
                                    IconName::WindowRestore
                                } else {
                                    IconName::WindowMaximize
                                },
                            )
                            .tooltip(if window.is_maximized() {
                                "还原窗口"
                            } else {
                                "最大化窗口"
                            })
                            .on_click(cx.listener(
                                |this, _, window, _| {
                                    if !window.is_maximized() {
                                        this.window_size = window.bounds().size;
                                    }
                                    window.zoom_window();
                                },
                            )),
                        )
                    })
                    .when(client_controls, |row| {
                        row.child(
                            layout::icon_button("window-close", IconName::WindowClose)
                                .tooltip("关闭窗口")
                                .on_click(
                                    cx.listener(|this, _, window, cx| {
                                        this.close_window(window, cx)
                                    }),
                                ),
                        )
                    }),
            )
            .when_some(
                current.filter(|session| session.archived),
                |header, session| {
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
                                    .child("已归档 · 历史已保留，恢复后可发送；当前任务仍可停止。"),
                            )
                            .child(self.archive_button(session, "restore-archived", cx)),
                    )
                },
            )
    }

    pub(super) fn workspace_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let sessions = self.visible_sessions();
        let groups = self.grouped_threads(&sessions, "");
        let directories: Vec<_> = groups.into_iter().map(|group| group.directory).collect();
        let selected = self.selected_directory.clone();
        let hint = groups::path_label(selected.as_deref());
        let view = cx.entity().downgrade();
        let parent = self.window;
        let button = Button::new("workspace-picker")
            .ghost()
            .xsmall()
            .h(px(layout::ICON_BUTTON_SIZE))
            .icon(IconName::Folder)
            .when(self.compact, |button| {
                button
                    .w(px(layout::ICON_BUTTON_SIZE))
                    .h(px(layout::ICON_BUTTON_SIZE))
                    .p_0()
            })
            .when(!self.compact, |button| {
                button.label(groups::name(selected.as_deref()))
            });
        let menu = self.menu_button(
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
        );
        crate::tooltip::bounded("workspace-picker-hint", menu, hint)
    }

    pub(super) fn agent_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let agents: Vec<_> = self.agents.iter().map(|agent| agent.id.clone()).collect();
        let selected = self
            .current()
            .map(|session| session.agent.to_string())
            .unwrap_or_else(|| self.selected_agent.clone());
        let disabled = !self.can_select_agent();
        let hint = if disabled {
            format!("Agent · {selected}")
        } else if selected.is_empty() {
            "选择或安装 Agent".into()
        } else {
            format!("选择 Agent · {selected}")
        };
        let view = cx.entity().downgrade();
        let metadata = self.agent_metadata.clone();
        let server = self.server;
        let parent = self.window;
        let button = Button::new("agent-picker")
            .ghost()
            .xsmall()
            .h(px(layout::ICON_BUTTON_SIZE))
            .p_0()
            .when(!self.waiting_for_agent_selection, |button| {
                button.w(px(layout::ICON_BUTTON_SIZE))
            })
            .flex_shrink_0()
            .disabled(disabled)
            .accessibility_label(hint.clone())
            .when(self.waiting_for_agent_selection, |button| {
                button.label("选择 Agent")
            })
            .child(
                self.agent_icon("agent-picker-icon", &selected, cx)
                    .size(px(TOOLBAR_AGENT_ICON_SIZE))
                    .text_color(if disabled {
                        cx.theme().muted_foreground
                    } else {
                        cx.theme().foreground
                    }),
            );
        let menu = if disabled {
            button.into_any_element()
        } else {
            self.menu_button(
                button,
                agents.len() + 2,
                false,
                move |menu, _, _| {
                    let mut menu = menu.check_side(Side::Right);
                    for id in &agents {
                        let id = id.clone();
                        let label = id.clone();
                        let metadata = metadata.clone();
                        let view = view.clone();
                        menu = menu.item(
                            PopupMenuItem::element(move |_, cx| {
                                let artwork = metadata
                                    .as_ref()
                                    .and_then(|metadata| metadata.read(cx).icon(server, &label))
                                    .unwrap_or_default();
                                div()
                                    .id(format!("agent-menu-label-{label}"))
                                    .test_support()
                                    .role(gpui_kit::Role::Label)
                                    .aria_label(label.clone())
                                    .flex()
                                    .flex_1()
                                    .min_w_0()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        crate::agent_icon::AgentIcon::new(
                                            format!("agent-menu-icon-{label}"),
                                            artwork,
                                        )
                                        .size(px(AGENT_ICON_SIZE)),
                                    )
                                    .child(div().flex_1().truncate().child(label.clone()))
                            })
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
                    let view = view.clone();
                    menu.separator().item(
                        PopupMenuItem::element(|_, _| {
                            div()
                                .id("agent-menu-install-label")
                                .test_support()
                                .role(gpui_kit::Role::Label)
                                .aria_label("安装 Agent…")
                                .flex()
                                .flex_1()
                                .items_center()
                                .gap_2()
                                .child(Icon::new(IconName::Plus).size(px(AGENT_ICON_SIZE)))
                                .child("安装 Agent…")
                        })
                        .on_click(move |_, window, cx| {
                            menus::in_parent(parent, window, cx, |window, cx| {
                                let _ =
                                    view.update(cx, |this, cx| this.open_agent_catalog(window, cx));
                            });
                        }),
                    )
                },
                cx,
            )
        };
        crate::tooltip::bounded("agent-picker-hint", menu, hint)
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
                    let hint = option.description.as_ref().map_or_else(
                        || option.name.clone(),
                        |description| format!("{} · {description}", option.name),
                    );
                    row = row.child(crate::tooltip::bounded(
                        SharedString::from(format!("config-hint-{}", option.id)),
                        self.menu_button(
                            Button::new(id)
                                .flex_shrink_0()
                                .disabled(self.configuring.contains(&session))
                                .ghost()
                                .xsmall()
                                .label(name)
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
                        hint,
                    ));
                }
                acp::SessionConfigKind::Boolean(boolean) => {
                    let hint = settings::config_option_hint(option);
                    row = row.child(
                        div()
                            .id(SharedString::from(format!("config-field-{}", option.id)))
                            .test_support()
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .gap_2()
                            .px_1()
                            .hoverable_tooltip(move |window, cx| {
                                crate::tooltip::view(hint.clone(), window, cx)
                            })
                            .child(
                                div()
                                    .id(SharedString::from(format!("config-label-{}", option.id)))
                                    .test_support()
                                    .text_xs()
                                    .text_color(if self.configuring.contains(&session) {
                                        cx.theme().muted_foreground
                                    } else {
                                        cx.theme().foreground
                                    })
                                    .child(option.name.clone()),
                            )
                            .child(
                                gpui_kit::component::switch::Switch::new(id)
                                    .flex_shrink_0()
                                    .checked(boolean.current_value)
                                    .disabled(self.configuring.contains(&session))
                                    .xsmall()
                                    .accessibility_label(option.name.clone())
                                    .on_change(move |checked, window, cx| {
                                        let request = acp::SetSessionConfigOptionRequest::new(
                                            session.to_string(),
                                            config_id.clone(),
                                            *checked,
                                        );
                                        let _ = view.update(cx, |this, cx| {
                                            this.configure(request, cx);
                                            this.focus_input(window, cx);
                                        });
                                    }),
                            ),
                    );
                }
                _ => {}
            }
        }
        row
    }
}
