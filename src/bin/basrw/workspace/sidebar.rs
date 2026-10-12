use super::*;
use gpui_kit::component::menu::PopupMenuItem;

const WORKSPACE_MENU_ROWS: usize = 4;

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
                    .child(
                        layout::icon_button("add-workspace-button", IconName::FolderPlus)
                            .accessibility_label(crate::i18n::tr!("workspace-add"))
                            .tooltip(crate::i18n::tr!("workspace-add"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_workspace_add_menu(window, cx)
                            })),
                    )
                    .child(
                        layout::icon_button("new-session", IconName::Plus)
                            .accessibility_label(crate::i18n::tr!("session-new"))
                            .tooltip(crate::i18n::tr!("session-new-shortcut"))
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
                    .pt_2()
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
                                crate::i18n::tr!("session-show-active")
                            } else {
                                crate::i18n::tr!("session-show-archived")
                            })
                            .tooltip(if self.archived_list {
                                crate::i18n::tr!("session-show-active")
                            } else {
                                crate::i18n::tr!("session-show-archived")
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
                    .when(groups.is_empty(), |list| {
                        list.child(
                            div()
                                .p_3()
                                .child(caption(crate::i18n::tr!("session-no-results"), cx)),
                        )
                    })
                    .children(groups.into_iter().enumerate().map(|(index, group)| {
                        let collapsed = query.is_empty()
                            && self.collapsed_workspaces.contains(&group.directory);
                        let directory = group.directory.clone();
                        let draft_selected =
                            self.active.is_none() && self.selected_directory == directory;
                        div()
                            .child(self.workspace_header(index, directory, collapsed, cx))
                            .when(!collapsed, |column| {
                                column
                                    .when(draft_selected, |column| {
                                        column.child(self.draft_thread_row(index, cx))
                                    })
                                    .when(group.threads.is_empty() && !draft_selected, |column| {
                                        column.child(
                                            caption(crate::i18n::tr!("session-empty"), cx)
                                                .ml_5()
                                                .px_3()
                                                .py_2(),
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

    fn workspace_header(
        &self,
        index: usize,
        directory: Option<PathBuf>,
        collapsed: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let group = SharedString::from(format!("workspace-header-{index}"));
        let name = groups::name(directory.as_deref());
        let new_directory = directory.clone();
        let menu = self.workspace_menu(directory.clone(), collapsed, cx);
        let button_menu = menu.clone();
        div()
            .id(group.clone())
            .test_support()
            .group(group.clone())
            .role(gpui_kit::Role::Button)
            .aria_label(name.clone())
            .aria_expanded(!collapsed)
            .w_full()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_1()
            .cursor_pointer()
            .hover(|row| row.bg(cx.theme().sidebar_accent))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.toggle_workspace_collapsed(directory.clone(), cx)
            }))
            .on_mouse_down(
                gpui_kit::MouseButton::Right,
                cx.listener(move |this, event: &gpui_kit::MouseDownEvent, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                    this.open_context_menu(
                        event.position,
                        WORKSPACE_MENU_ROWS,
                        menu.clone(),
                        window,
                        cx,
                    );
                }),
            )
            .child(
                div()
                    .id(SharedString::from(format!("workspace-{index}")))
                    .test_support()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Icon::new(if collapsed {
                            IconName::ChevronRight
                        } else {
                            IconName::ChevronDown
                        })
                        .xsmall()
                        .flex_shrink_0(),
                    )
                    .child(div().text_size(crate::ui::SMALL).truncate().child(name))
                    .hoverable_tooltip(move |window, cx| {
                        crate::tooltip::view(
                            if collapsed {
                                crate::i18n::tr!("workspace-expand")
                            } else {
                                crate::i18n::tr!("workspace-collapse")
                            },
                            window,
                            cx,
                        )
                    }),
            )
            .child(
                div()
                    .id(SharedString::from(format!("workspace-actions-{index}")))
                    .test_support()
                    .flex()
                    .items_center()
                    .flex_shrink_0()
                    .invisible()
                    .group_hover(group, |style| style.visible())
                    .child(
                        layout::icon_button(
                            SharedString::from(format!("new-thread-{index}")),
                            IconName::Plus,
                        )
                        .accessibility_label(crate::i18n::tr!("workspace-new-session"))
                        .tooltip(crate::i18n::tr!("workspace-new-session"))
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.new_thread(new_directory.clone(), window, cx);
                            },
                        )),
                    )
                    .child(
                        self.menu_button(
                            layout::icon_button(
                                SharedString::from(format!("workspace-menu-{index}")),
                                IconName::Ellipsis,
                            )
                            .accessibility_label(crate::i18n::tr!("workspace-more"))
                            .tooltip(crate::i18n::tr!("workspace-more"))
                            .on_click(|_, _, cx| cx.stop_propagation()),
                            WORKSPACE_MENU_ROWS,
                            false,
                            move |menu, window, cx| button_menu(menu, window, cx),
                            cx,
                        ),
                    ),
            )
            .into_any_element()
    }

    fn toggle_workspace_collapsed(&mut self, directory: Option<PathBuf>, cx: &mut Context<Self>) {
        if !self.collapsed_workspaces.remove(&directory) {
            self.collapsed_workspaces.insert(directory);
        }
        cx.notify();
    }

    fn workspace_menu(
        &self,
        directory: Option<PathBuf>,
        collapsed: bool,
        cx: &mut Context<Self>,
    ) -> menus::Builder {
        let owner = cx.entity().downgrade();
        let parent = self.window;
        std::rc::Rc::new(move |menu, _, _| {
            let new_owner = owner.clone();
            let new_directory = directory.clone();
            let collapse_owner = owner.clone();
            let collapse_directory = directory.clone();
            let add_owner = owner.clone();
            let path = directory
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned());
            menu.item(
                PopupMenuItem::new(crate::i18n::tr!("session-new"))
                    .icon(IconName::Plus)
                    .on_click(move |_, window, cx| {
                        menus::in_parent(parent, window, cx, |window, cx| {
                            let _ = new_owner.update(cx, |this, cx| {
                                this.new_thread(new_directory.clone(), window, cx)
                            });
                        });
                    }),
            )
            .item(
                PopupMenuItem::new(if collapsed {
                    crate::i18n::tr!("workspace-expand")
                } else {
                    crate::i18n::tr!("workspace-collapse")
                })
                .icon(if collapsed {
                    IconName::ChevronRight
                } else {
                    IconName::ChevronDown
                })
                .on_click(move |_, window, cx| {
                    menus::in_parent(parent, window, cx, |_, cx| {
                        let _ = collapse_owner.update(cx, |this, cx| {
                            this.toggle_workspace_collapsed(collapse_directory.clone(), cx)
                        });
                    });
                }),
            )
            .item(
                PopupMenuItem::new(crate::i18n::tr!("workspace-copy-path"))
                    .icon(IconName::Copy)
                    .disabled(path.is_none())
                    .on_click(move |_, _, cx| {
                        if let Some(path) = &path {
                            cx.write_to_clipboard(ClipboardItem::new_string(path.clone()));
                        }
                    }),
            )
            .item(
                PopupMenuItem::new(crate::i18n::tr!("workspace-add"))
                    .icon(IconName::FolderPlus)
                    .on_click(move |_, window, cx| {
                        menus::in_parent(parent, window, cx, |window, cx| {
                            let _ = add_owner
                                .update(cx, |this, cx| this.open_workspace_add_menu(window, cx));
                        });
                    }),
            )
        })
    }
}
