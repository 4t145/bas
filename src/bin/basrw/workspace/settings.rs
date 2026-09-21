use super::*;

impl Workspace {
    pub(super) fn settings_button(&self, cx: &mut Context<Self>) -> Button {
        layout::icon_button("open-settings", IconName::Settings)
            .accessibility_label("设置")
            .tooltip("设置 · Ctrl ,")
            .on_click(cx.listener(|this, _, window, cx| this.open_settings(window, cx)))
    }

    pub(super) fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_reply_preview(window, cx);
        self.close_menu(window, cx);
        if let Err(error) = crate::settings_window::open(cx) {
            self.error = Some(format!("无法打开设置窗口：{error}"));
        }
        cx.notify();
    }
}
