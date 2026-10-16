use super::*;

mod view;

pub(super) const COMPOSER_SIDE_PADDING: f32 = 4.;
pub(super) const COMPOSER_TOOLBAR_HEIGHT: f32 = 24.;
pub(super) const COMPOSER_BOTTOM_PADDING: f32 = 2.;
pub(super) const COMPOSER_CONTROL_INSET: f32 = 4.;

pub(super) use bas::config::appearance::ComposerEnterBehavior as Mode;

actions!(bas, [SendComposer, ExpandMessageEditor]);

fn text_menu(
    menu: gpui_kit::component::native_menu::NativeMenu,
    editable: bool,
    copyable: bool,
    cx: &App,
) -> gpui_kit::component::native_menu::NativeMenu {
    use gpui_kit::base::input::{Copy, Cut, Paste, SelectAll};
    menu.menu_with_disabled(
        crate::i18n::tr!("edit-cut"),
        !(editable && copyable),
        Box::new(Cut),
    )
    .menu_with_disabled(crate::i18n::tr!("edit-copy"), !copyable, Box::new(Copy))
    .menu_with_disabled(
        crate::i18n::tr!("edit-paste"),
        !(editable && cx.read_from_clipboard().is_some()),
        Box::new(Paste),
    )
    .separator()
    .menu(crate::i18n::tr!("edit-select-all"), Box::new(SelectAll))
}

pub(super) fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new(
            "ctrl-enter",
            SendComposer,
            Some("BasWorkspace > BasComposer > Input"),
        ),
        KeyBinding::new(
            "alt-shift-escape",
            ExpandMessageEditor,
            Some("BasWorkspace > BasComposer > Input"),
        ),
    ]);
    if cfg!(target_os = "macos") {
        cx.bind_keys([KeyBinding::new(
            "cmd-enter",
            SendComposer,
            Some("BasWorkspace > BasComposer > Input"),
        )]);
    }
}

impl Workspace {
    pub(super) fn capture_composer_enter(
        &mut self,
        action: &Enter,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .selected
            .id
            .is_some_and(|session| self.composition_locked(session))
            || (!action.shift && self.complete_command(window, cx))
        {
            cx.stop_propagation();
        } else {
            cx.propagate();
        }
    }

    pub(super) fn submit_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .selected
            .id
            .is_some_and(|session| self.sending(session))
        {
            return;
        }
        if !self.complete_command(window, cx) {
            self.send_prompt(window, cx);
        }
    }

    pub(super) fn sync_composer_mode(&mut self, cx: &mut Context<Self>) {
        let mode = if self.composer.expanded {
            Mode::Newline
        } else {
            cx.global::<theme::Appearance>().composer_enter_behavior
        };
        if mode == self.composer.mode {
            return;
        }
        self.composer.mode = mode;
        self.composer.input.update(cx, |input, cx| {
            input.set_submit_on_enter(mode == Mode::Send, cx);
        });
        cx.notify();
    }

    pub(super) fn toggle_composer_expansion(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.composer.expanded = !self.composer.expanded;
        self.sync_composer_mode(cx);
        self.measure_composer(cx);
        if self.window.compact {
            self.resize_window(window, cx);
        }
        self.composer
            .input
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    pub(super) fn composer_send_shortcut(&self) -> &'static str {
        match self.composer.mode {
            Mode::Send => "Enter",
            Mode::Newline if cfg!(target_os = "macos") => "⌘ Enter",
            Mode::Newline => "Ctrl+Enter",
        }
    }

    pub(super) fn composer_newline_shortcut(&self) -> &'static str {
        match self.composer.mode {
            Mode::Send => "Shift+Enter",
            Mode::Newline => "Enter",
        }
    }
}
