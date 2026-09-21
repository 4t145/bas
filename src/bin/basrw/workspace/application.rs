use super::*;
use gpui_kit::component::menu::PopupMenuItem;

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
                        PopupMenuItem::new("关于 BASR").on_click(move |_, window, cx| {
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
                        PopupMenuItem::new("查看更新").on_click(move |_, window, cx| {
                            menus::in_parent(parent, window, cx, |_, cx| {
                                crate::application::open_releases(cx)
                            });
                        }),
                    )
                    .separator()
                    .item(
                        PopupMenuItem::new("设置")
                            .action(Box::new(crate::application::OpenSettings))
                            .on_click(move |_, window, cx| {
                                menus::in_parent(parent, window, cx, |window, cx| {
                                    let _ = settings
                                        .update(cx, |this, cx| this.open_settings(window, cx));
                                });
                            }),
                    )
                    .separator()
                    .item(
                        PopupMenuItem::new("退出 BASR")
                            .action(Box::new(crate::application::Quit))
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
