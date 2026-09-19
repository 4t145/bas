use basr::acp;
use std::collections::HashMap;

const ROWS: u16 = 48;
const COLS: u16 = 160;
const SCROLLBACK: usize = 1000;

pub(crate) struct Terminal {
    parser: vt100::Parser,
    pub output: String,
    pub exit: Option<acp::TerminalExitStatus>,
}

impl Default for Terminal {
    fn default() -> Self {
        Self {
            parser: vt100::Parser::new(ROWS, COLS, SCROLLBACK),
            output: String::new(),
            exit: None,
        }
    }
}

impl Terminal {
    fn push(&mut self, data: &str) {
        self.parser.process(data.replace('\n', "\r\n").as_bytes());
        let screen = self.parser.screen_mut();
        screen.set_scrollback(usize::MAX);
        let retained = screen.scrollback();
        let mut lines = Vec::new();
        for offset in (1..=retained).rev() {
            screen.set_scrollback(offset);
            lines.push(screen.rows(0, COLS).next().unwrap_or_default());
        }
        screen.set_scrollback(0);
        lines.extend(screen.rows(0, COLS));
        while lines.last().is_some_and(|line| line.is_empty()) {
            lines.pop();
        }
        self.output = lines.join("\n");
    }
}

/// Zed's display-only terminal extension. The original ACP updates remain in history.
/// https://github.com/zed-industries/zed/blob/main/crates/agent_servers/src/acp.rs
pub(super) fn apply(terminals: &mut HashMap<String, Terminal>, meta: Option<&acp::Meta>) {
    let Some(meta) = meta else {
        return;
    };
    for key in ["terminal_info", "terminal_output", "terminal_exit"] {
        let Some(value) = meta.get(key) else {
            continue;
        };
        let Some(id) = value.get("terminal_id").and_then(|id| id.as_str()) else {
            continue;
        };
        let terminal = terminals.entry(id.to_owned()).or_default();
        match key {
            "terminal_output" => {
                if let Some(data) = value.get("data").and_then(|data| data.as_str()) {
                    terminal.push(data);
                }
            }
            "terminal_exit" => {
                terminal.exit = Some(
                    acp::TerminalExitStatus::new()
                        .exit_code(
                            value
                                .get("exit_code")
                                .and_then(|code| code.as_u64())
                                .and_then(|code| u32::try_from(code).ok()),
                        )
                        .signal(
                            value
                                .get("signal")
                                .and_then(|signal| signal.as_str())
                                .map(str::to_owned),
                        ),
                );
            }
            _ => {}
        }
    }
}
