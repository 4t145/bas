use super::*;

pub(super) use basr::config::appearance::ComposerEnterBehavior as Mode;

actions!(basr, [SendComposer, ExpandMessageEditor]);

pub(super) fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new(
            "ctrl-enter",
            SendComposer,
            Some("BasrWorkspace > BasrComposer > Input"),
        ),
        KeyBinding::new(
            "alt-shift-escape",
            ExpandMessageEditor,
            Some("BasrWorkspace > BasrComposer > Input"),
        ),
    ]);
    if cfg!(target_os = "macos") {
        cx.bind_keys([KeyBinding::new(
            "cmd-enter",
            SendComposer,
            Some("BasrWorkspace > BasrComposer > Input"),
        )]);
    }
}

impl Workspace {
    pub(super) fn composer_expansion_button(&self, cx: &mut Context<Self>) -> Button {
        message_actions::action_button(
            "expand-message-editor",
            if self.composer_expanded {
                IconName::Minimize2
            } else {
                IconName::Maximize2
            },
            cx,
        )
        .tooltip(if self.composer_expanded {
            "收起消息编辑器 · Alt Shift Escape"
        } else {
            "展开消息编辑器 · Alt Shift Escape"
        })
        .on_click(cx.listener(|this, _, window, cx| this.toggle_composer_expansion(window, cx)))
    }

    pub(super) fn capture_composer_enter(
        &mut self,
        action: &Enter,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active.is_some_and(|session| self.sending(session))
            || (!action.shift && self.complete_command(window, cx))
        {
            cx.stop_propagation();
        } else {
            cx.propagate();
        }
    }

    pub(super) fn submit_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.active.is_some_and(|session| self.sending(session)) {
            return;
        }
        if !self.complete_command(window, cx) {
            self.send_prompt(window, cx);
        }
    }

    pub(super) fn sync_composer_mode(&mut self, cx: &mut Context<Self>) {
        let mode = if self.composer_expanded {
            Mode::Newline
        } else {
            cx.global::<theme::Appearance>().composer_enter_behavior
        };
        if mode == self.composer_mode {
            return;
        }
        self.composer_mode = mode;
        self.composer.update(cx, |input, cx| {
            input.set_submit_on_enter(mode == Mode::Send, cx);
        });
        cx.notify();
    }

    pub(super) fn toggle_composer_expansion(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.composer_expanded = !self.composer_expanded;
        self.sync_composer_mode(cx);
        self.measure_composer(cx);
        if self.compact {
            self.resize_window(window, cx);
        }
        self.composer
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    pub(super) fn composer_send_shortcut(&self) -> &'static str {
        match self.composer_mode {
            Mode::Send => "Enter",
            Mode::Newline if cfg!(target_os = "macos") => "⌘ Enter",
            Mode::Newline => "Ctrl Enter",
        }
    }

    pub(super) fn composer_newline_shortcut(&self) -> &'static str {
        match self.composer_mode {
            Mode::Send => "Shift Enter",
            Mode::Newline => "Enter",
        }
    }
}
