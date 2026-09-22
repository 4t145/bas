use super::*;
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::menu::PopupMenuItem;

// Plain keybinding text inherits the same typography and baseline as its label.
fn application_item(
    label: &'static str,
    action: Option<Box<dyn gpui_kit::Action>>,
) -> PopupMenuItem {
    let hint_action = action.as_ref().map(|action| action.boxed_clone());
    let item = PopupMenuItem::element(move |window, cx| {
        div()
            .id(label)
            .test_support()
            .role(gpui_kit::Role::Label)
            .aria_label(label)
            .flex()
            .flex_1()
            .items_baseline()
            .gap_4()
            .text_sm()
            .child(div().id("label").test_support().flex_1().child(label))
            .children(hint_action.as_ref().and_then(|action| {
                Kbd::binding_for_action(action.as_ref(), None, window).map(|kbd| {
                    div()
                        .id("shortcut")
                        .test_support()
                        .flex_shrink_0()
                        .text_color(cx.theme().muted_foreground)
                        .child(kbd.appearance(false))
                })
            }))
    });
    match action {
        Some(action) => item.action(action),
        None => item,
    }
}

impl Workspace {
    pub(super) fn application_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let view = cx.entity().downgrade();
        self.menu_button(
            layout::icon_button("app-menu", IconName::Menu)
                .accessibility_label("BASR 应用菜单")
                .tooltip("BASR 菜单 · 查看更新会打开发布页面，需手动查看"),
            6,
            false,
            move |menu, window, _| {
                let parent = window.window_handle();
                let settings = view.clone();
                let about = view.clone();
                let quit = view.clone();
                menu.min_w(px(280.))
                    .max_w(px(320.))
                    .item(
                        application_item("关于 BASR", None).on_click(move |_, window, cx| {
                            menus::in_parent(parent, window, cx, |window, cx| {
                                let _ = about.update(cx, |this, cx| {
                                    this.close_menu(window, cx);
                                    if let Err(error) = crate::application::open_about(cx) {
                                        this.error = Some(format!("无法打开关于窗口：{error}"));
                                        cx.notify();
                                    }
                                });
                            });
                        }),
                    )
                    .item(
                        application_item("查看更新", None).on_click(move |_, window, cx| {
                            menus::in_parent(parent, window, cx, |_, cx| {
                                crate::application::open_releases(cx)
                            });
                        }),
                    )
                    .separator()
                    .item(
                        application_item("设置", Some(Box::new(crate::application::OpenSettings)))
                            .on_click(move |_, window, cx| {
                                menus::in_parent(parent, window, cx, |window, cx| {
                                    let _ = settings
                                        .update(cx, |this, cx| this.open_settings(window, cx));
                                });
                            }),
                    )
                    .separator()
                    .item(
                        application_item("退出 BASR", Some(Box::new(crate::application::Quit)))
                            .on_click(move |_, window, cx| {
                                menus::in_parent(parent, window, cx, |window, cx| {
                                    let _ = quit.update(cx, |this, cx| {
                                        this.close_menu(window, cx);
                                        if let Err(error) = crate::application::open_quit(cx) {
                                            this.error =
                                                Some(format!("无法打开退出确认窗口：{error}"));
                                            cx.notify();
                                        }
                                    });
                                });
                            }),
                    )
            },
            cx,
        )
    }
}
