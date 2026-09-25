use super::*;

impl Workspace {
    pub(super) fn authentication_button(&self, cx: &mut Context<Self>) -> AnyElement {
        crate::tooltip::bounded(
            "authentication-hint",
            Button::new("agent-authenticate")
                .xsmall()
                .label(if self.server.ip().is_loopback() {
                    "认证"
                } else {
                    "认证说明"
                })
                .on_click(cx.listener(|this, _, window, cx| this.open_agent_auth(window, cx))),
            self.authentication_hint().unwrap_or_default(),
        )
        .into_any_element()
    }

    pub(super) fn authentication_hint(&self) -> Option<String> {
        self.authentication_required.as_ref().map(|agent| {
            if self.server.ip().is_loopback() {
                format!("{agent} 的 ACP 已连接，需要认证。打开认证设置完成登录后，按 F5 重试；草稿已保留。")
            } else {
                format!("{agent} 需要认证。请在远端 daemon（{}）所在主机完成登录后按 F5 重试；不会在本机运行远端登录命令。", self.server)
            }
        })
    }

    pub(super) fn open_agent_auth(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(agent) = self.authentication_required.clone() else {
            return;
        };
        self.close_reply_preview(window, cx);
        self.close_menu(window, cx);
        let result = if self.server.ip().is_loopback() {
            crate::settings_window::open_agent(&agent, cx)
        } else {
            crate::application::open_message(
                "远端 Agent 需要认证",
                &self.authentication_hint().unwrap_or_default(),
                cx,
            )
        };
        if let Err(error) = result {
            self.error = Some(format!("无法打开认证说明：{error}"));
        }
        cx.notify();
    }

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
