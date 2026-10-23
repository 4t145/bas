use std::io::{self, Read, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::Console::{
    AllocConsole, ENABLE_ECHO_INPUT, ENABLE_LINE_INPUT, FreeConsole, GetConsoleMode, GetStdHandle,
    INPUT_RECORD, INPUT_RECORD_0, KEY_EVENT, KEY_EVENT_RECORD, KEY_EVENT_RECORD_0,
    LEFT_CTRL_PRESSED, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, SetConsoleMode, SetStdHandle,
    WriteConsoleInputW,
};
use windows_sys::Win32::System::Threading::DETACHED_PROCESS;

const CHILD_MODE: &str = "BAST_TEST_PRIVATE_CONSOLE";
const TIMEOUT: Duration = Duration::from_secs(3);

#[test]
fn windows_console_cancel_restore_and_redirect_regressions() {
    // Never alter the developer's console or share an input queue with other tests.
    for mode in ["console", "redirected"] {
        let mut child = Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "command::agents::terminal::platform::tests::private_console_child",
                "--ignored",
                "--nocapture",
            ])
            .env(CHILD_MODE, mode)
            .creation_flags(DETACHED_PROCESS)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("isolated console test process");
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(b"untouched\n")
            .expect("redirected input");
        let deadline = Instant::now() + Duration::from_secs(15);
        while child.try_wait().expect("child status").is_none() {
            if Instant::now() >= deadline {
                child.kill().expect("stop timed-out test");
                panic!("Windows {mode} test timed out");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let output = child.wait_with_output().expect("child output");
        assert!(
            output.status.success(),
            "{mode}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
#[ignore = "launched in a detached child by windows_console_cancel_restore_and_redirect_regressions"]
fn private_console_child() {
    let mode = std::env::var(CHILD_MODE).expect("only run via the isolated parent test");
    if mode == "redirected" {
        let error = super::line(
            Arc::new(AtomicBool::new(false)),
            Arc::new(super::Gate::default()),
            reedline::ExternalPrinter::default(),
        )
        .expect_err("redirected stdin is not an interactive console");
        assert_eq!(error.kind(), io::ErrorKind::Unsupported);
        let mut input = String::new();
        std::io::stdin()
            .read_to_string(&mut input)
            .expect("untouched stdin");
        assert_eq!(input, "untouched\n");
        return;
    }

    let console = PrivateConsole::new();
    let input = console.input.as_ref().expect("input").as_raw_handle();
    // Use a non-default cooked mode, so restoring crossterm's defaults fails.
    let original = mode_of(input) & !ENABLE_ECHO_INPUT;
    // SAFETY: this child owns a private console.
    assert_ne!(unsafe { SetConsoleMode(input, original) }, 0);
    let output = console.output.as_ref().expect("output").as_raw_handle();
    let original_output = mode_of(output);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("runtime");
    runtime.block_on(async {
        for _ in 0..3 {
            let session = super::super::Session::acquire().await.expect("session");
            let read = tokio::spawn(async move { session.read_line().await });
            tokio::time::timeout(TIMEOUT, async {
                while mode_of(input) & ENABLE_LINE_INPUT != 0 {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            })
            .await
            .expect("editor entered raw mode");
            read.abort();
            assert!(read.await.expect_err("aborted").is_cancelled());
            let next = tokio::time::timeout(TIMEOUT, super::super::Session::acquire())
                .await
                .expect("reader drains without keyboard input")
                .expect("next session");
            assert_eq!(mode_of(input), original);
            assert_eq!(mode_of(output), original_output);
            push_text(input, "新的回答🙂\rsecond\r");
            assert_eq!(
                next.read_line().await.expect("Unicode after cancellation"),
                "新的回答🙂"
            );
            assert_eq!(
                next.read_line().await.expect("no read-ahead loss"),
                "second"
            );
        }
        let session = super::super::Session::acquire().await.expect("EOF session");
        for key in ['d', 'z'] {
            push_control(input, key);
            assert_eq!(
                session.read_line().await.expect_err("empty EOF").kind(),
                io::ErrorKind::UnexpectedEof
            );
        }
        push_text(input, "partial");
        push_control(input, 'z');
        assert_eq!(session.read_line().await.expect("partial EOF"), "partial");
        push_control(input, 'c');
        assert_eq!(
            session.read_line().await.expect_err("Ctrl-C").kind(),
            io::ErrorKind::Interrupted
        );
        assert_eq!(mode_of(input), original);
        assert_eq!(mode_of(output), original_output);
    });
}

fn mode_of(handle: HANDLE) -> u32 {
    let mut mode = 0;
    // SAFETY: handle belongs to the isolated test console; mode is writable.
    assert_ne!(unsafe { GetConsoleMode(handle, &mut mode) }, 0);
    mode
}

fn push_text(handle: HANDLE, text: &str) {
    for unit in text.encode_utf16() {
        push_key(handle, unit, if unit == 13 { 13 } else { 0 }, 0);
    }
}

fn push_control(handle: HANDLE, key: char) {
    push_key(
        handle,
        key as u16 - b'a' as u16 + 1,
        key.to_ascii_uppercase() as u16,
        LEFT_CTRL_PRESSED,
    );
}

fn push_key(handle: HANDLE, unit: u16, virtual_key: u16, modifiers: u32) {
    let event = INPUT_RECORD {
        EventType: KEY_EVENT as u16,
        Event: INPUT_RECORD_0 {
            KeyEvent: KEY_EVENT_RECORD {
                bKeyDown: 1,
                wRepeatCount: 1,
                wVirtualKeyCode: virtual_key,
                wVirtualScanCode: 0,
                uChar: KEY_EVENT_RECORD_0 { UnicodeChar: unit },
                dwControlKeyState: modifiers,
            },
        },
    };
    let mut written = 0;
    // SAFETY: one valid input record, writable count and a live console handle.
    assert_ne!(
        unsafe { WriteConsoleInputW(handle, &event, 1, &mut written) },
        0
    );
    assert_eq!(written, 1);
}

struct PrivateConsole {
    input: Option<OwnedHandle>,
    output: Option<OwnedHandle>,
    previous_input: HANDLE,
    previous_output: HANDLE,
}

impl PrivateConsole {
    fn new() -> Self {
        // SAFETY: only this detached test subprocess allocates/changes its console.
        let previous_input = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
        let previous_output = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
        assert_ne!(unsafe { AllocConsole() }, 0);
        let input = open_console("CONIN$");
        let output = open_console("CONOUT$");
        // SAFETY: owned handles remain live until restored in Drop.
        unsafe {
            assert_ne!(SetStdHandle(STD_INPUT_HANDLE, input.as_raw_handle()), 0);
            assert_ne!(SetStdHandle(STD_OUTPUT_HANDLE, output.as_raw_handle()), 0);
        }
        Self {
            input: Some(input),
            output: Some(output),
            previous_input,
            previous_output,
        }
    }
}

impl Drop for PrivateConsole {
    fn drop(&mut self) {
        // SAFETY: restore the still-live inherited handles before closing the
        // private handles and detaching this child from its console.
        unsafe {
            SetStdHandle(STD_INPUT_HANDLE, self.previous_input);
            SetStdHandle(STD_OUTPUT_HANDLE, self.previous_output);
        }
        drop(self.input.take());
        drop(self.output.take());
        unsafe { FreeConsole() };
    }
}

fn open_console(name: &str) -> OwnedHandle {
    let name: Vec<_> = name.encode_utf16().chain(Some(0)).collect();
    // SAFETY: terminated name; no optional pointer arguments; returned handle
    // is checked before transferring ownership.
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    };
    assert_ne!(handle, INVALID_HANDLE_VALUE);
    unsafe { OwnedHandle::from_raw_handle(handle) }
}
