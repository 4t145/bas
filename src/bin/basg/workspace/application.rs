use super::*;
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::menu::PopupMenuItem;

// Plain keybinding text inherits the same typography and baseline as its label.
fn application_item(
    label: &'static str,
    action: Option<Box<dyn gpui_kit::Action>>,
) -> PopupMenuItem {
    action_item(label, action, None)
}

pub(super) fn action_item(
    label: &'static str,
    action: Option<Box<dyn gpui_kit::Action>>,
    context: Option<&'static str>,
) -> PopupMenuItem {
    let hint_action = action.as_ref().map(|action| action.boxed_clone());
    let item = PopupMenuItem::element(move |window, cx| {
        let shortcut = hint_action.as_ref().and_then(|action| {
            Kbd::binding_for_action(action.as_ref(), context, window).map(|kbd| {
                el! {
                    .div {
                        # "shortcut";
                        test_support;
                        flex_shrink_0;
                        text_color: $cx.theme().muted_foreground;
                    } [$kbd.appearance(false)]
                }
            })
        });
        el! {
            .div {
                # label;
                test_support;
                role: gpui_kit::Role::Label;
                aria_label: label;
                flex; flex_1; items_baseline; gap_4;
                text_size: ui::SMALL;
            } [
                .div { # "label"; test_support; flex_1; } [label],
                ..shortcut,
            ]
        }
    });
    match action {
        Some(action) => item.action(action),
        None => item,
    }
}

impl Workspace {
    pub(super) fn application_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let view = cx.entity().downgrade();
        let connected = self.backend.connected;
        self.menu_button(
            el! {
                ("app-menu", IconName::Menu).icon_button {
                    accessibility_label: tr!("app-menu");
                    tooltip: tr!("app-menu");
                }
            },
            10,
            false,
            move |menu, window, _| {
                let parent = window.window_handle();
                let settings = view.clone();
                let about = view.clone();
                let quit = view.clone();
                let new_session = view.clone();
                let workspace = view.clone();
                let manager = view.clone();
                let logs = view.clone();
                menu.min_w(px(280.))
                    .max_w(px(320.))
                    .item(
                        application_item(crate::i18n::tr!("app-about"), None).on_click(
                            move |_, window, cx| {
                                menus::in_parent(parent, window, cx, |window, cx| {
                                    let _ = about.update(cx, |this, cx| {
                                        this.open_about_from_menu(window, cx);
                                    });
                                });
                            },
                        ),
                    )
                    .item(
                        application_item(crate::i18n::tr!("app-releases"), None).on_click(
                            move |_, window, cx| {
                                menus::in_parent(parent, window, cx, |_, cx| {
                                    crate::application::open_releases(cx)
                                });
                            },
                        ),
                    )
                    .separator()
                    .item(
                        shortcuts::menu_item(
                            crate::i18n::tr!("session-new"),
                            Some(Box::new(NewSession)),
                        )
                        .on_click(move |_, window, cx| {
                            menus::in_parent(parent, window, cx, |window, cx| {
                                let _ = new_session.update(cx, |this, cx| {
                                    this.close_menu(window, cx);
                                    this.new_session(window, cx);
                                });
                            });
                        }),
                    )
                    .item(
                        shortcuts::menu_item(
                            crate::i18n::tr!("workspace-add"),
                            Some(Box::new(shortcuts::AddWorkspace)),
                        )
                        .on_click(move |_, window, cx| {
                            menus::in_parent(parent, window, cx, |window, cx| {
                                let _ = workspace.update(cx, |this, cx| {
                                    this.close_menu(window, cx);
                                    this.open_workspace_add_menu(window, cx);
                                });
                            });
                        }),
                    )
                    .separator()
                    .item(
                        shortcuts::menu_item(
                            crate::i18n::tr!("agent-history-title"),
                            Some(Box::new(shortcuts::OpenSessionManager)),
                        )
                        .disabled(!connected)
                        .on_click(move |_, window, cx| {
                            menus::in_parent(parent, window, cx, |window, cx| {
                                let _ = manager.update(cx, |this, cx| {
                                    this.close_menu(window, cx);
                                    // Activate the manager after the borrowed popup finishes closing.
                                    cx.defer_in(window, |this, window, cx| {
                                        this.on_open_session_manager(
                                            &shortcuts::OpenSessionManager,
                                            window,
                                            cx,
                                        );
                                    });
                                });
                            });
                        }),
                    )
                    .item(
                        application_item(
                            crate::i18n::tr!("settings-title"),
                            Some(Box::new(crate::application::OpenSettings)),
                        )
                        .on_click(move |_, window, cx| {
                            menus::in_parent(parent, window, cx, |window, cx| {
                                let _ =
                                    settings.update(cx, |this, cx| this.open_settings(window, cx));
                            });
                        }),
                    )
                    .item(
                        application_item(
                            crate::i18n::tr!("logs-title"),
                            Some(Box::new(crate::diagnostics::OpenLogs)),
                        )
                        .on_click(move |_, window, cx| {
                            menus::in_parent(parent, window, cx, |window, cx| {
                                let _ = logs.update(cx, |this, cx| {
                                    this.close_menu(window, cx);
                                    cx.defer_in(window, |_, window, cx| {
                                        window.dispatch_action(
                                            Box::new(crate::diagnostics::OpenLogs),
                                            cx,
                                        );
                                    });
                                });
                            });
                        }),
                    )
                    .separator()
                    .item(
                        application_item(
                            crate::i18n::tr!("app-quit"),
                            Some(Box::new(crate::application::Quit)),
                        )
                        .on_click(move |_, window, cx| {
                            menus::in_parent(parent, window, cx, |window, cx| {
                                let _ = quit.update(cx, |this, cx| {
                                    this.open_quit_from_menu(window, cx);
                                });
                            });
                        }),
                    )
            },
            cx,
        )
    }

    fn open_about_from_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_menu(window, cx);
        if let Err(error) = crate::application::open_about(cx) {
            self.selected.error = Some(crate::i18n::tr!(
                "app-about-error",
                error = (error).to_string()
            ));
            cx.notify();
        }
    }

    fn open_quit_from_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_menu(window, cx);
        if let Err(error) = crate::application::open_quit(cx) {
            self.selected.error = Some(crate::i18n::tr!(
                "app-quit-error",
                error = (error).to_string()
            ));
            cx.notify();
        }
    }
}
