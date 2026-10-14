use bas::acp::{AvailableCommand, AvailableCommandInput};

use super::*;

pub(super) const VISIBLE_COMMANDS: usize = 5;

impl Workspace {
    pub(super) fn command_candidates(&self, cx: &App) -> Vec<&AvailableCommand> {
        if self.composer.commands.dismissed {
            return Vec::new();
        }
        let value = self.composer.input.read(cx).value();
        let Some(query) = value.strip_prefix('/') else {
            return Vec::new();
        };
        if query.contains(char::is_whitespace) {
            return Vec::new();
        }
        let query = query.to_lowercase();
        self.selected
            .timeline
            .conversation
            .as_ref()
            .into_iter()
            .flat_map(Conversation::commands)
            .filter(|command| command.name.to_lowercase().starts_with(&query))
            .collect()
    }

    pub(super) fn complete_command(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let candidates = self.command_candidates(cx);
        let Some(command) = candidates.get(
            self.composer
                .commands
                .selection
                .min(candidates.len().saturating_sub(1)),
        ) else {
            return false;
        };
        let name = command.name.clone();
        self.insert_command(&name, window, cx);
        true
    }

    fn insert_command(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.composer.input.update(cx, |input, cx| {
            let text = format!("/{name} ");
            let end = Position::new(0, text.encode_utf16().count() as u32);
            input.replace_all(text, window, cx);
            input.set_cursor_position(end, window, cx);
        });
        self.composer.commands.selection = 0;
        if self.window.compact {
            self.resize_window(window, cx);
        }
        cx.notify();
    }

    pub(super) fn command_action(
        &mut self,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if key == "escape" && self.clear_feedback() {
            cx.stop_propagation();
            cx.notify();
            return;
        }
        let count = self.command_candidates(cx).len();
        if count == 0 {
            cx.propagate();
            return;
        }
        match key {
            "up" => {
                self.composer.commands.selection =
                    (self.composer.commands.selection.min(count - 1) + count - 1) % count
            }
            "down" => {
                self.composer.commands.selection = (self.composer.commands.selection + 1) % count
            }
            "tab" => {
                self.complete_command(window, cx);
            }
            "escape" => {
                self.composer.commands.dismissed = true;
                if self.window.compact {
                    self.resize_window(window, cx);
                }
            }
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    pub(super) fn render_commands(&self, cx: &mut Context<Self>) -> AnyElement {
        let candidates = self.command_candidates(cx);
        let selected = self
            .composer
            .commands
            .selection
            .min(candidates.len().saturating_sub(1));
        let start = selected.saturating_sub(VISIBLE_COMMANDS - 1);
        let has_candidates = !candidates.is_empty();
        let shortcuts = has_candidates.then(|| caption(crate::i18n::tr!("commands-shortcuts"), cx));
        let rows = candidates
            .into_iter()
            .enumerate()
            .skip(start)
            .take(VISIBLE_COMMANDS)
            .map(|(index, command)| {
                let name = command.name.clone();
                gds_uim::el! {
                    #div {
                        id: $SharedString::from(format!("slash-{}", command.name));
                        test_support; cursor_pointer; rounded_md; px_2; py_1;
                        @$index == selected => { bg: $cx.theme().accent; };
                        on_click: $cx.listener(move |this, _, window, cx| {
                            this.insert_command(&name, window, cx)
                        });
                    } [
                        #div {
                            text_size: $crate::ui::SMALL;
                            font_weight: FontWeight::SEMIBOLD;
                        } [$format!("/{}", command.name)],
                        #caption($command.description.clone(), cx) { truncate; },
                    ]
                }
            })
            .collect::<Vec<_>>();
        let hint = self.command_hint(cx).map(|hint| {
            gds_uim::el! { #caption(hint, cx) { px_3; py_1; } }
        });
        gds_uim::el! {
            #div {
                id: "command-suggestions"; flex; flex_col; flex_shrink_0;
                @has_candidates => { border_b_1; border_color: $cx.theme().border; p_2; };
            } [..shortcuts, ..rows, ..hint]
        }
        .into_any_element()
    }

    pub(super) fn command_hint(&self, cx: &App) -> Option<String> {
        let value = self.composer.input.read(cx).value();
        let (name, _) = value.strip_prefix('/')?.split_once(char::is_whitespace)?;
        let command = self
            .selected
            .timeline
            .conversation
            .as_ref()?
            .commands()
            .iter()
            .find(|c| c.name == name)?;
        match command.input.as_ref()? {
            AvailableCommandInput::Unstructured(input) => Some(input.hint.clone()),
            _ => None,
        }
    }
}
