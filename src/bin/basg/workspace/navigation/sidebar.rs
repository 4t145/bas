use super::groups;
use crate::workspace::*;
use gpui_kit::component::menu::PopupMenuItem;

const WORKSPACE_MENU_ROWS: usize = 4;

impl Workspace {
    pub(in crate::workspace) fn render_sidebar(
        &self,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let query = self.navigation.search.read(cx).value().to_lowercase();
        let sessions = self.visible_sessions();
        let groups = self.grouped_threads(&sessions, &query);
        let header = gds_uim::el! {
            #div {
                id: "sidebar-header"; test_support; flex; flex_shrink_0;
                h: layout::HEADER_HEIGHT.px; items_center; gap_1; px_3;
                @$cfg!(target_os = "macos") => { pl: layout::MACOS_WINDOW_CONTROLS_WIDTH.px; };
            } [
                #div {
                    id: "application-menu-slot"; test_support; flex_1; min_w_0; flex; items_center;
                } [$self.application_button(cx)],
                #layout::icon_button("add-workspace-button", IconName::FolderPlus) {
                    accessibility_label: $crate::i18n::tr!("workspace-add");
                    hoverable_tooltip: $shortcuts::tooltip(crate::i18n::tr!("workspace-add"), shortcuts::AddWorkspace);
                    on_click: $cx.listener(Self::on_sidebar_add_workspace);
                },
                #layout::icon_button("new-session", IconName::Plus) {
                    accessibility_label: $crate::i18n::tr!("session-new");
                    tooltip: $crate::i18n::tr!("session-new-shortcut");
                    on_click: $cx.listener(Self::on_sidebar_new_session);
                },
            ]
        };
        let archived_hint = if self.navigation.archived {
            crate::i18n::tr!("session-show-active")
        } else {
            crate::i18n::tr!("session-show-archived")
        };
        let search_icon = gds_uim::el! { Icon(IconName::Search) { small; } };
        let search = gds_uim::el! {
            #div { id: "thread-search-row"; test_support; px_3; pt_2; pb_3; flex; items_center; gap_2; } [
                #div { id: "thread-search"; test_support; flex_1; min_w_0; } [
                    Input($&self.navigation.search) { small; prefix: search_icon; cleanable: true; }
                ],
                #layout::icon_button("archived-list", IconName::Archive) {
                    selected: $self.navigation.archived;
                    accessibility_label: archived_hint; tooltip: archived_hint;
                    on_click: $cx.listener(Self::on_sidebar_archived);
                },
            ]
        };
        let empty = groups.is_empty().then(|| {
            gds_uim::el! { #div { p_3; } [#caption($crate::i18n::tr!("session-no-results"), cx)] }
        });
        let columns = groups.into_iter().enumerate().map(|(index, group)| {
            let collapsed = query.is_empty() && self.navigation.collapsed.contains(&group.directory);
            let directory = group.directory.clone();
            let draft_selected = self.selected.id.is_none()
                && self.navigation.selected_directory == directory;
            let header = self.workspace_header(index, directory, collapsed, cx);
            let draft = (!collapsed && draft_selected).then(|| self.draft_thread_row(index, cx));
            let empty = (!collapsed && group.threads.is_empty() && !draft_selected).then(|| {
                gds_uim::el! { #caption($crate::i18n::tr!("session-empty"), cx) { ml_5; px_3; py_2; } }
            });
            let threads = group.threads.into_iter().filter(|_| !collapsed)
                .map(|session| self.thread_row(session, cx));
            gds_uim::el! { #div [$header, ..draft, ..empty, ..threads] }
        }).collect::<Vec<_>>();
        gds_uim::el! { #div {
            flex; flex_col; w: SIDEBAR_WIDTH.px; flex_shrink_0; h_full;
            bg: $cx.theme().sidebar; border_r_1; border_color: $cx.theme().border;
        } [
            $header, $search,
            #div { id: "session-list"; flex_1; min_h_0; overflow_y_scroll; } [..empty, ..columns],
        ] }
    }

    fn on_sidebar_add_workspace(
        &mut self,
        _: &gpui_kit::ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_workspace_add_menu(window, cx);
    }

    fn on_sidebar_new_session(
        &mut self,
        _: &gpui_kit::ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.new_session(window, cx);
    }

    fn on_sidebar_archived(
        &mut self,
        _: &gpui_kit::ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        self.navigation.archived = !self.navigation.archived;
        cx.notify();
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
        let more_button = gds_uim::el! {
            #layout::icon_button($SharedString::from(format!("workspace-menu-{index}")), IconName::Ellipsis) {
                accessibility_label: $crate::i18n::tr!("workspace-more");
                tooltip: $crate::i18n::tr!("workspace-more");
                on_click: $|_, _, cx| cx.stop_propagation();
            }
        };
        let more = self.menu_button(
            more_button,
            WORKSPACE_MENU_ROWS,
            false,
            move |menu, window, cx| button_menu(menu, window, cx),
            cx,
        );
        let actions = gds_uim::el! {
            #div {
                id: $SharedString::from(format!("workspace-actions-{index}"));
                test_support; flex; items_center; flex_shrink_0; invisible;
                group_hover: $group.clone(), $gds_uim::m! { visible; };
            } [
                #layout::icon_button($SharedString::from(format!("new-thread-{index}")), IconName::Plus) {
                    accessibility_label: $crate::i18n::tr!("workspace-new-session");
                    tooltip: $crate::i18n::tr!("workspace-new-session");
                    on_click: $cx.listener(move |this, _, window, cx| {
                        this.on_sidebar_new_thread(new_directory.clone(), window, cx);
                    });
                },
                $more,
            ]
        };
        gds_uim::el! {
            #div {
                id: $group.clone(); test_support; group: group;
                role: gpui_kit::Role::Button; aria_label: $name.clone(); aria_expanded: $!collapsed;
                w_full; flex; items_center; gap_2; px_3; py_1; cursor_pointer;
                hover => { bg: $cx.theme().sidebar_accent; };
                on_click: $cx.listener(move |this, _, _, cx| {
                this.toggle_workspace_collapsed(directory.clone(), cx)
                });
                on_mouse_down: gpui_kit::MouseButton::Right,
                    $cx.listener(move |this, event, window, cx| {
                        this.on_sidebar_context_menu(menu.clone(), event, window, cx);
                    });
            } [
                #div {
                    id: $SharedString::from(format!("workspace-{index}"));
                    test_support; flex_1; min_w_0; flex; items_center; gap_2;
                    hoverable_tooltip: $move |window, cx| {
                        crate::ui::tooltip::view(
                            if collapsed {
                                crate::i18n::tr!("workspace-expand")
                            } else {
                                crate::i18n::tr!("workspace-collapse")
                            },
                            window,
                            cx,
                        )
                    };
                } [
                    Icon($(if collapsed { IconName::ChevronRight } else { IconName::ChevronDown })) {
                        xsmall; flex_shrink_0;
                    },
                    #div { text_size: $crate::ui::SMALL; truncate; } [$name],
                ],
                $actions,
            ]
        }.into_any_element()
    }

    fn on_sidebar_new_thread(
        &mut self,
        directory: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        self.new_thread(directory, window, cx);
    }

    fn on_sidebar_context_menu(
        &mut self,
        menu: menus::Builder,
        event: &gpui_kit::MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.prevent_default();
        cx.stop_propagation();
        self.open_context_menu(event.position, WORKSPACE_MENU_ROWS, menu, window, cx);
    }

    fn toggle_workspace_collapsed(&mut self, directory: Option<PathBuf>, cx: &mut Context<Self>) {
        if !self.navigation.collapsed.remove(&directory) {
            self.navigation.collapsed.insert(directory);
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
        let parent = self.window.handle;
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
                shortcuts::menu_item(crate::i18n::tr!("session-new"), Some(Box::new(NewSession)))
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
                        Self::on_sidebar_copy_path(path.as_deref(), cx);
                    }),
            )
            .item(
                shortcuts::menu_item(
                    crate::i18n::tr!("workspace-add"),
                    Some(Box::new(shortcuts::AddWorkspace)),
                )
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

    fn on_sidebar_copy_path(path: Option<&str>, cx: &mut App) {
        if let Some(path) = path {
            cx.write_to_clipboard(ClipboardItem::new_string(path.to_owned()));
        }
    }
}
