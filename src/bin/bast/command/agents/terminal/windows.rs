//! Reedline 0.51.0 uses crossterm's timed console-event polling when a break
//! signal is installed. There is no background input thread or cooked read.
//! Do not replace this with ReadConsoleW + CancelSynchronousIo: conhost can keep
//! the cancelled cooked read alive and discard the next request's input.
//! https://github.com/microsoft/terminal/issues/12143

use std::borrow::Cow;
use std::io::{self, Write};
use std::os::windows::io::{AsRawHandle, BorrowedHandle, OwnedHandle};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use reedline::{
    Emacs, ExternalPrinter, FileBackedHistory, KeyCode, KeyModifiers, Prompt, PromptEditMode,
    PromptHistorySearch, Reedline, ReedlineEvent, Signal, default_emacs_keybindings,
};
use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
use windows_sys::Win32::System::Console::{
    ENABLE_VIRTUAL_TERMINAL_PROCESSING, GetConsoleMode, GetStdHandle, STD_ERROR_HANDLE,
    STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, SetConsoleMode,
};

use super::Gate;

const END_OF_INPUT: &str = "bast-end-of-input";

/// # Errors
/// Reports EOF, cancellation, unavailable consoles, mode restoration or editor errors.
pub(super) fn line(
    cancelled: Arc<AtomicBool>,
    gate: Arc<Gate>,
    printer: ExternalPrinter<String>,
) -> io::Result<String> {
    super::check_cancelled(&cancelled)?;
    let mut modes = ConsoleModes::capture(gate)?;
    let result = edit_line(cancelled, printer);
    // Reedline has already been dropped, including its raw-mode cleanup.
    // Restore the exact original flags, not crossterm's cooked-mode defaults.
    modes.restore()?;
    result
}

/// # Errors
/// Reports EOF, interruption or an editor error.
fn edit_line(cancelled: Arc<AtomicBool>, printer: ExternalPrinter<String>) -> io::Result<String> {
    let mut bindings = default_emacs_keybindings();
    // Keep Windows' Ctrl-Z EOF convention, including a final unterminated line.
    // HostCommand is an opaque event, not a shell command; nothing is executed.
    bindings.add_binding(
        KeyModifiers::CONTROL,
        KeyCode::Char('z'),
        ReedlineEvent::ExecuteHostCommand(END_OF_INPUT.into()),
    );
    let mut editor = Reedline::create()
        .with_edit_mode(Box::new(Emacs::new(bindings)))
        .with_history(Box::new(
            FileBackedHistory::new(0).expect("zero-capacity in-memory history is valid"),
        ))
        .with_ansi_colors(false)
        .with_external_printer(printer)
        .with_break_signal(cancelled);
    // No hinter or file-backed history is installed, and this editor is never
    // reused: authentication input and partially cancelled buffers stay local.
    let signal = editor.read_line(&EmptyPrompt)?;
    if matches!(&signal, Signal::ExternalBreak(_) | Signal::HostCommand(_)) {
        // These signals suspend rather than submit in Reedline. We discard the
        // editor, so move below its partial line before progress/status output.
        std::io::stderr().write_all(b"\r\n")?;
    }
    match signal {
        Signal::Success(line) => Ok(line.trim_end_matches(['\r', '\n']).to_owned()),
        Signal::HostCommand(command) if command == END_OF_INPUT => {
            let line = editor.current_buffer_contents();
            if line.is_empty() {
                Err(eof())
            } else {
                Ok(line.trim_end_matches(['\r', '\n']).to_owned())
            }
        }
        Signal::CtrlD => Err(eof()),
        Signal::CtrlC | Signal::ExternalBreak(_) => Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Input request cancelled",
        )),
        _ => Err(io::Error::other("Unexpected terminal editor event")),
    }
}

fn eof() -> io::Error {
    io::Error::new(io::ErrorKind::UnexpectedEof, "Terminal input closed")
}

struct EmptyPrompt;

impl Prompt for EmptyPrompt {
    fn render_prompt_left(&self) -> Cow<'_, str> {
        "".into()
    }
    fn render_prompt_right(&self) -> Cow<'_, str> {
        "".into()
    }
    fn render_prompt_indicator(&self, _mode: PromptEditMode) -> Cow<'_, str> {
        "".into()
    }
    fn render_prompt_multiline_indicator(&self) -> Cow<'_, str> {
        "".into()
    }
    fn render_prompt_history_search_indicator(&self, _search: PromptHistorySearch) -> Cow<'_, str> {
        "".into()
    }
}

struct ConsoleMode {
    handle: OwnedHandle,
    original: u32,
}

struct ConsoleModes {
    modes: Vec<ConsoleMode>,
    gate: Arc<Gate>,
}

impl ConsoleModes {
    /// # Errors
    /// Rejects redirected stdin; reports unavailable consoles or mode/handle errors.
    fn capture(gate: Arc<Gate>) -> io::Result<Self> {
        let input = console_mode(STD_INPUT_HANDLE)?.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                "Interactive authentication requires a Windows console; redirected stdin is not read",
            )
        })?;
        let mut modes = vec![input];
        for id in [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            if let Some(mode) = console_mode(id)? {
                modes.push(mode);
            }
        }
        // Crossterm uses CONOUT$ even when stdout/stderr are redirected.
        let output: OwnedHandle = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("CONOUT$")?
            .into();
        let mut original = 0;
        // SAFETY: output owns a live console handle; original is writable.
        if unsafe { GetConsoleMode(output.as_raw_handle(), &mut original) } == 0 {
            return Err(io::Error::last_os_error());
        }
        modes.push(ConsoleMode {
            handle: output,
            original,
        });
        let captured = Self { modes, gate };
        let output = captured.modes.last().expect("captured the active output");
        // Crossterm caches ANSI support, not the current mode. Re-enable VT for
        // each editor after the previous editor restored the original flags.
        // SAFETY: the handle stays live in captured, whose Drop also rolls back
        // on a failed mode change.
        if unsafe {
            SetConsoleMode(
                output.handle.as_raw_handle(),
                output.original | ENABLE_VIRTUAL_TERMINAL_PROCESSING,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(captured)
    }

    /// # Errors
    /// Reports restoration failure and permanently faults stdin before releasing it.
    fn restore(&mut self) -> io::Result<()> {
        let mut failure = None;
        for mode in self.modes.drain(..) {
            // SAFETY: each duplicated handle is live; only its captured mode is
            // restored. The stdin lease excludes other bast readers/mode owners.
            if unsafe { SetConsoleMode(mode.handle.as_raw_handle(), mode.original) } == 0 {
                failure.get_or_insert_with(io::Error::last_os_error);
            }
        }
        if let Some(error) = failure {
            let message = format!("Could not restore console mode: {error}");
            self.gate.fail(io::Error::other(message.clone()));
            Err(io::Error::other(message))
        } else {
            Ok(())
        }
    }
}

impl Drop for ConsoleModes {
    fn drop(&mut self) {
        // Also restore after unwinding an editor panic.
        let _ = self.restore();
    }
}

/// # Errors
/// Reports failure duplicating a live console handle.
fn console_mode(id: u32) -> io::Result<Option<ConsoleMode>> {
    // SAFETY: scalar-only API. Standard handles remain process-owned; bast
    // neither replaces nor closes them during authentication.
    let raw = unsafe { GetStdHandle(id) };
    if raw.is_null() || raw == INVALID_HANDLE_VALUE {
        return Ok(None);
    }
    let mut original = 0;
    // SAFETY: raw is a process-owned handle and original is writable.
    if unsafe { GetConsoleMode(raw, &mut original) } == 0 {
        return Ok(None);
    }
    // SAFETY: borrow only to duplicate; never take ownership of the std handle.
    let handle = unsafe { BorrowedHandle::borrow_raw(raw) }.try_clone_to_owned()?;
    Ok(Some(ConsoleMode { handle, original }))
}

#[cfg(test)]
#[path = "windows/tests.rs"]
mod tests;
