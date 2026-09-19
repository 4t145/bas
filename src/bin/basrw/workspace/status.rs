use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ConnectionStatus {
    Connecting,
    Connected,
    Disconnected,
}

impl Workspace {
    pub(super) fn connection_status(&self) -> ConnectionStatus {
        if self.connection_error.is_some() {
            return ConnectionStatus::Disconnected;
        }
        if !self.connected {
            return ConnectionStatus::Connecting;
        }
        if self.active.is_none() {
            return ConnectionStatus::Connected;
        }
        match self.stream {
            StreamState::Loading | StreamState::Reconnecting(_) => ConnectionStatus::Connecting,
            StreamState::Live => ConnectionStatus::Connected,
            StreamState::Unavailable(_) | StreamState::Closed => ConnectionStatus::Disconnected,
        }
    }

    pub(super) fn connection_indicator(&self, id: &'static str, cx: &App) -> Button {
        let (label, color) = match self.connection_status() {
            ConnectionStatus::Connecting => ("正在连接…", cx.theme().warning),
            ConnectionStatus::Connected => ("已连接", cx.theme().success),
            ConnectionStatus::Disconnected => ("连接已断开", cx.theme().danger),
        };
        let unread = self.has_unread_reply();
        let label = if unread {
            format!("{label} · 有未读回复")
        } else {
            label.to_owned()
        };
        Button::new(id)
            .ghost()
            .xsmall()
            .accessibility_label(label.clone())
            .tooltip(format!(
                "{label} · {}\n{}",
                self.server,
                self.stream_label()
            ))
            .child(
                div()
                    .relative()
                    .size_3()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(div().size_2().rounded_full().bg(color))
                    .when(unread, |light| {
                        light.child(
                            div()
                                .id("unread-reply")
                                .test_support()
                                .absolute()
                                .inset_0()
                                .rounded_full()
                                .border_2()
                                .border_color(cx.theme().warning),
                        )
                    }),
            )
    }

    pub(super) fn activity_label(&self) -> Option<String> {
        let active = self.creating
            || self.working()
            || self
                .active
                .is_some_and(|id| self.sending.contains(&id) || self.configuring.contains(&id));
        (active && self.connection_status() == ConnectionStatus::Connected)
            .then(|| self.stream_label())
    }
}
