//! One format for shortcut hints; bindings remain the source of truth.

use gpui_kit::{Action, AsKeystroke, KeyContext, Window, component::kbd::Kbd};

pub(crate) fn format(label: &str, keys: &str) -> String {
    format!("{label} · {keys}")
}

pub(crate) fn hint(label: &str, action: &dyn Action, context: &str, window: &Window) -> String {
    let context = KeyContext::parse(context).expect("static shortcut context");
    match window
        .highest_precedence_binding_for_action(action)
        .or_else(|| window.highest_precedence_binding_for_action_in_context(action, context))
    {
        Some(binding) => {
            let keys = binding
                .keystrokes()
                .iter()
                .map(|key| Kbd::format(key.as_keystroke()))
                .collect::<Vec<_>>()
                .join(" ");
            format(label, &keys)
        }
        None => label.to_owned(),
    }
}
