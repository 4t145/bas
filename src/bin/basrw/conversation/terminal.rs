use agent_client_protocol::schema::{MaybeUndefined, v2 as acp2};
use base64::Engine as _;
use basr::acp;
use std::{collections::HashMap, path::PathBuf};

#[cfg(test)]
#[path = "terminal/tests.rs"]
mod tests;

const ROWS: u16 = 48;
const COLS: u16 = 160;
const SCROLLBACK: usize = 1000;

pub(crate) struct Terminal {
    parser: vt100::Parser,
    pub output: String,
    pub exit: Option<acp::TerminalExitStatus>,
    pub cwd: Option<PathBuf>,
    pub label: Option<String>,
    pub meta: Option<acp2::Meta>,
    pub output_meta: Option<acp2::Meta>,
}

#[derive(Debug, thiserror::Error)]
#[error("invalid terminal output base64: {0}")]
pub(super) struct Error(#[from] base64::DecodeError);

impl Default for Terminal {
    fn default() -> Self {
        Self {
            parser: vt100::Parser::new(ROWS, COLS, SCROLLBACK),
            output: String::new(),
            exit: None,
            cwd: None,
            label: None,
            meta: None,
            output_meta: None,
        }
    }
}

impl Terminal {
    fn push(&mut self, data: &str) {
        self.push_bytes(data.replace('\n', "\r\n").as_bytes());
    }

    fn push_bytes(&mut self, data: &[u8]) {
        self.parser.process(data);
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

    fn clear_output(&mut self) {
        self.parser = vt100::Parser::new(ROWS, COLS, SCROLLBACK);
        self.output.clear();
        self.output_meta = None;
    }

    /// Applies an agent-owned terminal patch, preserving omitted fields.
    ///
    /// # Errors
    /// Returns an error if the output snapshot contains invalid base64.
    pub(super) fn apply_update(&mut self, update: &acp2::TerminalUpdate) -> Result<(), Error> {
        // Decode first so malformed output cannot partially apply the patch.
        let output = update
            .output
            .value()
            .map(|output| base64::engine::general_purpose::STANDARD.decode(&output.data))
            .transpose()?;
        if !update.command.is_undefined() {
            self.label = update.command.clone().take();
        }
        if !update.cwd.is_undefined() {
            self.cwd = update.cwd.clone().take().map(|path| path.0);
        }
        match &update.output {
            MaybeUndefined::Undefined => {}
            MaybeUndefined::Null => self.clear_output(),
            MaybeUndefined::Value(snapshot) => {
                self.clear_output();
                self.push_bytes(output.as_deref().expect("snapshot was decoded"));
                self.output_meta = snapshot.meta.clone();
            }
        }
        if !update.exit_status.is_undefined() {
            self.exit = update.exit_status.value().map(|status| {
                acp::TerminalExitStatus::new()
                    .exit_code(status.exit_code)
                    .signal(status.signal.clone())
                    .meta(status.meta.clone())
            });
        }
        if !update.meta.is_undefined() {
            self.meta = update.meta.clone().take();
        }
        Ok(())
    }

    /// Appends independently encoded bytes without losing split UTF-8 sequences.
    ///
    /// # Errors
    /// Returns an error if the output chunk contains invalid base64.
    pub(super) fn push_chunk(&mut self, chunk: &acp2::TerminalOutputChunk) -> Result<(), Error> {
        let bytes = base64::engine::general_purpose::STANDARD.decode(&chunk.data)?;
        self.push_bytes(&bytes);
        Ok(())
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
            "terminal_info" => {
                if let Some(cwd) = value.get("cwd") {
                    terminal.cwd = cwd.as_str().map(PathBuf::from);
                }
                if let Some(label) = value
                    .get("command")
                    .or_else(|| value.get("label"))
                    .or_else(|| value.get("title"))
                {
                    terminal.label = label.as_str().map(str::to_owned);
                }
            }
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
