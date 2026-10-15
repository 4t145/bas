//! Thread shortcuts route through the same guarded operations as the visible controls.

use super::*;
use gpui_kit::{
    Action, AnyView, AsKeystroke, KeyContext,
    component::{kbd::Kbd, menu::PopupMenuItem},
};

actions!(
    bas,
    [
        RenameThread,
        ArchiveThread,
        ToggleThreadPin,
        DeleteThread,
        AddWorkspace,
        ScrollConversationToTop,
        PageConversationUp,
        PageConversationDown,
        PreviousConversationTurn,
        NextConversationTurn
    ]
);

pub(super) fn bind_keys(cx: &mut App) {
    for context in [
        "BasWorkspace",
        "BasWorkspace > Input",
        "BasWorkspace > BasComposer > Input",
    ] {
        cx.bind_keys([
            KeyBinding::new("f2", RenameThread, Some(context)),
            KeyBinding::new("ctrl-w", ArchiveThread, Some(context)),
            KeyBinding::new("ctrl-alt-p", ToggleThreadPin, Some(context)),
            KeyBinding::new("ctrl-alt-w", DeleteThread, Some(context)),
            KeyBinding::new("ctrl-alt-n", AddWorkspace, Some(context)),
            KeyBinding::new("ctrl-home", ScrollConversationToTop, Some(context)),
            KeyBinding::new("pageup", PageConversationUp, Some(context)),
            KeyBinding::new("pagedown", PageConversationDown, Some(context)),
            KeyBinding::new("ctrl-pageup", PreviousConversationTurn, Some(context)),
            KeyBinding::new("ctrl-pagedown", NextConversationTurn, Some(context)),
        ]);
    }
}

pub(super) fn menu_item(label: &'static str, action: Option<Box<dyn Action>>) -> PopupMenuItem {
    application::action_item(label, action, Some("BasWorkspace"))
}

pub(super) fn hint(label: &str, action: &dyn Action, window: &Window) -> String {
    let context = KeyContext::parse("BasWorkspace").expect("workspace key context");
    match window.highest_precedence_binding_for_action_in_context(action, context) {
        Some(binding) => {
            let keys = binding
                .keystrokes()
                .iter()
                .map(|key| Kbd::format(key.as_keystroke()))
                .collect::<Vec<_>>()
                .join(" ");
            format!("{label} ({keys})")
        }
        None => label.to_owned(),
    }
}

pub(super) fn tooltip(
    label: &'static str,
    action: impl Action,
) -> impl Fn(&mut Window, &mut App) -> AnyView {
    move |window, cx| crate::ui::tooltip::view(hint(label, &action, window), window, cx)
}

impl Workspace {
    pub(super) fn on_conversation_top(
        &mut self,
        _: &ScrollConversationToTop,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.scroll_conversation_to_top(cx);
    }

    pub(super) fn on_conversation_page_up(
        &mut self,
        _: &PageConversationUp,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.page_conversation(true, cx);
    }

    pub(super) fn on_conversation_page_down(
        &mut self,
        _: &PageConversationDown,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.page_conversation(false, cx);
    }

    pub(super) fn on_conversation_previous_turn(
        &mut self,
        _: &PreviousConversationTurn,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.jump_conversation_turn(true, cx);
    }

    pub(super) fn on_conversation_next_turn(
        &mut self,
        _: &NextConversationTurn,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.jump_conversation_turn(false, cx);
    }

    pub(super) fn on_rename_thread(
        &mut self,
        _: &RenameThread,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_menu(window, cx);
        if self.window.compact {
            self.toggle_mode(window, cx);
        }
        self.begin_rename(window, cx);
    }

    pub(super) fn on_archive_thread(
        &mut self,
        _: &ArchiveThread,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = self.selected.id
            && !self.archive_blocked(id)
            && self.sessions.entries.iter().any(|session| session.id == id)
        {
            self.set_archived(id, true, cx);
        }
    }

    pub(super) fn on_toggle_thread_pin(
        &mut self,
        _: &ToggleThreadPin,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = self.selected.id
            && !self.sessions.pending.deleting.contains(&id)
            && self.sessions.entries.iter().any(|session| session.id == id)
        {
            self.toggle_pinned(id, cx);
        }
    }

    pub(super) fn on_delete_thread(
        &mut self,
        _: &DeleteThread,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = self.selected.id {
            self.request_delete(id, window, cx);
        }
    }

    pub(super) fn on_add_workspace(
        &mut self,
        _: &AddWorkspace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.choose_local_workspace(window, cx);
    }
}
