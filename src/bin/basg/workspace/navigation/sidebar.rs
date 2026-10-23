use super::{View, groups, history};
use crate::ui::input::placeholder;
use crate::workspace::*;
use gpui_kit::component::menu::PopupMenuItem;

const WORKSPACE_MENU_ROWS: usize = 5;

impl Workspace {
    pub(in crate::workspace) fn render_sidebar(
        &self,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let query = self.navigation.search.read(cx).value().to_lowercase();
        let sessions = self.visible_sessions();
        let groups = if self.navigation.view.flat() {
            Vec::new()
        } else {
            self.grouped_threads(&sessions, &query)
        };
        let header = el! {
            .div {
                # "sidebar-header"; test_support; flex; flex_shrink_0;
                h: layout::HEADER_HEIGHT.px; items_center; gap_1; px_3;
                @if (cfg!(target_os = "macos"))
                    pl: layout::MACOS_WINDOW_CONTROLS_WIDTH.px;
                @end;
            } [
                .div {
                    # "application-menu-slot"; test_support; flex_1; min_w_0; flex; items_center;
                } [$self.application_button(cx)],
                ("agent-history-open", crate::history::ICON).icon_button {
                    accessibility_label: tr!("agent-history-title");
                    hoverable_tooltip: $shortcuts::tooltip(tr!("agent-history-title"), shortcuts::OpenSessionManager);
                    disabled: $!self.backend.connected;
                    on_click: $cx.listener(|this, _, window, cx| {
                        let agent = this.current().map(|meta| meta.agent.to_string())
                            .unwrap_or_else(|| this.selected.agent.clone());
                        this.open_agent_history(agent, window, cx);
                    });
                },
                ("add-workspace-button", IconName::FolderPlus).icon_button {
                    accessibility_label: tr!("workspace-add");
                    hoverable_tooltip: $shortcuts::tooltip(tr!("workspace-add"), shortcuts::AddWorkspace);
                    on_click: $cx.listener(Self::on_sidebar_add_workspace);
                },
                ("new-session", IconName::Plus).icon_button {
                    accessibility_label: tr!("session-new");
                    hoverable_tooltip: $shortcuts::tooltip(tr!("session-new"), NewSession);
                    on_click: $cx.listener(Self::on_sidebar_new_session);
                },
            ]
        };
        let archived_hint = if self.navigation.view == View::Archived {
            crate::i18n::tr!("session-show-active")
        } else {
            crate::i18n::tr!("session-show-archived")
        };
        let search_icon = el! { IconName::Search.Icon::new { small; } };
        let search = el! {
            .div { # "thread-search-row"; test_support; px_3; pt_2; pb_3; flex; items_center; gap_2; } [
                .div { # "thread-search"; test_support; flex_1; min_w_0; } [
                    ($(&self.navigation.search), $(&self.navigation.search).Input::new {
                        small; prefix: search_icon; cleanable: true;
                    }).placeholder,
                ],
                .div { flex; items_center; gap_1; flex_shrink_0; } [
                    ("bookmarked-list", IconName::Bookmark).icon_button {
                        selected: $(self.navigation.view == View::Bookmarks);
                        accessibility_label: tr!("sidebar-bookmarks"); tooltip: tr!("sidebar-bookmarks");
                        on_click: $cx.listener(|this, _, _, cx| this.toggle_sidebar_view(View::Bookmarks, cx));
                    },
                    ("history-list", IconName::Clock).icon_button {
                        selected: $(self.navigation.view == View::History);
                        accessibility_label: tr!("sidebar-history"); tooltip: tr!("sidebar-history");
                        on_click: $cx.listener(|this, _, _, cx| this.toggle_sidebar_view(View::History, cx));
                    },
                    ("archived-list", IconName::Archive).icon_button {
                        selected: $(self.navigation.view == View::Archived);
                        accessibility_label: archived_hint; tooltip: archived_hint;
                        on_click: $cx.listener(|this, _, _, cx| this.toggle_sidebar_view(View::Archived, cx));
                    },
                ],
            ]
        };
        let empty = if self.navigation.view.flat() {
            !sessions
                .iter()
                .any(|session| matches_query(session, &query))
        } else {
            groups.is_empty()
        };
        let columns = if self.navigation.view == View::History {
            let now = jiff::Zoned::now();
            history::groups(&sessions, &query, &now)
                .into_iter()
                .map(|group| {
                    let label = group.period.label(now.date());
                    el! { .div { flex; flex_col; } [
                        .div {
                            # $group.period.id(); test_support; role: gpui_kit::Role::Label;
                            aria_label: ~label; px_3; pt_3; pb_1;
                            text_size: ui::SMALL; text_color: $cx.theme().muted_foreground;
                        } [label],
                        ..$(group.sessions) { into_iter; map: session => $self.thread_row(session, cx); },
                    ] }
                    .into_any_element()
                })
                .collect::<Vec<_>>()
        } else if self.navigation.view == View::Bookmarks {
            let threads: Vec<_> = history::sorted(&sessions)
                .into_iter()
                .filter(|session| matches_query(session, &query))
                .collect();
            let label = crate::i18n::tr!("sidebar-bookmarks-count", count = threads.len());
            vec![
                el! { .div { flex; flex_col; } [
                    .div {
                        # "thread-bookmarks-title"; test_support; role: gpui_kit::Role::Label;
                        aria_label: ~label; px_3; pt_2; pb_1;
                        text_size: ui::SMALL; text_color: $cx.theme().muted_foreground;
                    } [label],
                    ..threads { into_iter; map: session => $self.thread_row(session, cx); },
                ] }
                .into_any_element(),
            ]
        } else {
            groups.into_iter().enumerate().map(|(index, group)| {
            let collapsed = query.is_empty() && self.navigation.collapsed.contains(&group.directory);
            let directory = group.directory.clone();
            let draft_selected = self.navigation.view == View::Workspaces && self.selected.id.is_none()
                && self.navigation.selected_directory == directory;
            let header = self.workspace_header(index, directory, collapsed, cx);
            el! { .div [
                header,
                @if (!collapsed && draft_selected)
                    $self.draft_thread_row(index, cx),
                @end,
                @if (!collapsed && group.threads.is_empty() && !draft_selected)
                    (tr!("session-empty"), cx).caption { ml_5; px_3; py_2; },
                @end,
                @if (!collapsed)
                    ..$(group.threads) { into_iter; map: session => $self.thread_row(session, cx); },
                @end,
            ] }.into_any_element()
        }).collect::<Vec<_>>()
        };
        el! { .div {
            flex; flex_col; w: SIDEBAR_WIDTH.px; flex_shrink_0; h_full;
            bg: $cx.theme().sidebar; border_r_1; border_color: $cx.theme().border;
        } [
            header, search,
            .div { # "session-list"; test_support; flex_1; min_h_0; overflow_y_scroll; } [
                ..columns,
                @if (empty)
                    .div { p_3; } [(tr!("session-no-results"), cx).caption],
                @end,
            ],
            $self.server_status_footer(cx),
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

    fn toggle_sidebar_view(&mut self, view: View, cx: &mut Context<Self>) {
        cx.stop_propagation();
        self.save_title(cx);
        self.navigation.view = if self.navigation.view == view {
            View::Workspaces
        } else {
            view
        };
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
        let more_button = el! {
            (format!("workspace-menu-{index}").SharedString::from, IconName::Ellipsis).icon_button {
                accessibility_label: tr!("workspace-more");
                tooltip: tr!("workspace-more");
                on_click: (_, _, cx) => $cx.stop_propagation();
            }
        };
        let more = self.menu_button(
            more_button,
            WORKSPACE_MENU_ROWS,
            false,
            move |menu, window, cx| button_menu(menu, window, cx),
            cx,
        );
        let actions = el! {
            .div {
                # format!("workspace-actions-{index}").SharedString::from;
                test_support; flex; items_center; flex_shrink_0; invisible;
                group_hover: ~group, { visible; };
            } [
                (format!("new-thread-{index}").SharedString::from, IconName::Plus).icon_button {
                    accessibility_label: tr!("workspace-new-session");
                    tooltip: tr!("workspace-new-session");
                    on_click: $cx.listener(move |this, _, window, cx| {
                        this.on_sidebar_new_thread(new_directory.clone(), window, cx);
                    });
                },
                more,
            ]
        };
        el! {
            .div {
                # ~group; test_support; group: group;
                role: gpui_kit::Role::Button; aria_label: ~name; aria_expanded: $!collapsed;
                w_full; flex; items_center; gap_2; px_3; py_1; cursor_pointer;
                hover: { bg: $cx.theme().sidebar_accent; };
                on_click: $cx.listener(move |this, _, _, cx| {
                this.toggle_workspace_collapsed(directory.clone(), cx)
                });
                on_mouse_down: gpui_kit::MouseButton::Right,
                    $cx.listener(move |this, event, window, cx| {
                        this.on_sidebar_context_menu(menu.clone(), event, window, cx);
                    });
            } [
                .div {
                    # format!("workspace-{index}").SharedString::from;
                    test_support; flex_1; min_w_0; flex; items_center; gap_2;
                    hoverable_tooltip: move (window, cx) => (
                        @if (collapsed) tr!("workspace-expand") @else tr!("workspace-collapse") @end,
                        window, cx,
                    ).ui::tooltip::view;
                } [
                    (@if (collapsed) IconName::ChevronRight @else IconName::ChevronDown @end).Icon::new {
                        xsmall; flex_shrink_0;
                    },
                    .div { text_size: ui::SMALL; truncate; } [name],
                ],
                actions,
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
        let removable = self.can_remove_workspace(&directory);
        std::rc::Rc::new(move |menu, _, _| {
            let new_owner = owner.clone();
            let new_directory = directory.clone();
            let collapse_owner = owner.clone();
            let collapse_directory = directory.clone();
            let add_owner = owner.clone();
            let remove_owner = owner.clone();
            let remove_directory = directory.clone();
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
            .item(
                PopupMenuItem::new(crate::i18n::tr!("workspace-remove"))
                    .icon(IconName::Archive)
                    .disabled(!removable)
                    .on_click(move |_, window, cx| {
                        menus::in_parent(parent, window, cx, |_, cx| {
                            let _ = remove_owner.update(cx, |this, cx| {
                                this.request_remove_workspace(remove_directory.clone(), cx);
                            });
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
