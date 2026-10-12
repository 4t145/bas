use super::*;
use nix::{
    sys::signal::{Signal, kill, killpg},
    unistd::Pid,
};
use std::{
    fs,
    io::Write as _,
    os::unix::process::{CommandExt as _, ExitStatusExt as _},
    path::PathBuf,
    process::{Child, Stdio},
    thread,
    time::{Duration, Instant},
};

const MODE: &str = "BASR_PROCESS_TEST_MODE";
const DIRECTORY: &str = "BASR_PROCESS_TEST_DIRECTORY";
const DEADLINE: Duration = Duration::from_secs(10);

// Every dangerous operation occurs in a separately spawned group/session.
// In particular, an ordinary invocation of this test does nothing.
#[test]
fn fixture() {
    let Ok(mode) = std::env::var(MODE) else {
        return;
    };
    let directory = PathBuf::from(std::env::var_os(DIRECTORY).expect("fixture directory"));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("fixture runtime");
    runtime.block_on(async {
        let mut command = Command::new("sh");
        command.env(DIRECTORY, &directory).env(MODE, &mode);
        match mode.as_str() {
            "exit" => {
                command.args(["-c", "exit 37"]);
            }
            "pty" => {
                command.args([
                    "-c",
                    "read -r answer; [ \"$answer\" = terminal-input ] && exit 37; exit 9",
                ]);
            }
            "reject" | "background" => {
                command.args(["-c", "touch \"$BASR_PROCESS_TEST_DIRECTORY/spawned\""]);
            }
            _ => {
                command.args([
                    "-c",
                    "trap '' TERM; \
                     if [ \"$BASR_PROCESS_TEST_MODE\" = responsive ]; then trap 'exit 0' TERM; fi; \
                     echo $$ > \"$BASR_PROCESS_TEST_DIRECTORY/root\"; \
                     sh -c 'trap \"\" TERM; echo $$ > \"$BASR_PROCESS_TEST_DIRECTORY/grandchild\"; \
                     while :; do sleep 1; done' & wait",
                ]);
            }
        }
        let mut background = if mode == "background" {
            let child = Command::new("sleep")
                .arg("30")
                .process_group(0)
                .spawn()
                .expect("background foreground-owner fixture");
            nix::unistd::tcsetpgrp(std::io::stdin(), Pid::from_raw(child.id() as i32))
                .expect("set other foreground owner");
            Some(child)
        } else {
            None
        };
        let cancellation = async {
            loop {
                if directory.join("cancel").exists() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        };
        if mode == "drop" {
            let future = run(command, std::future::pending());
            tokio::pin!(future);
            tokio::select! {
                _ = &mut future => panic!("login unexpectedly exited"),
                _ = cancellation => {}
            }
            // Dropping a polled run future must synchronously kill this group.
            return;
        }
        let result = run(command, cancellation).await;
        if mode == "reject" || mode == "background" {
            assert_eq!(
                result
                    .expect_err("must reject unowned group/terminal")
                    .kind(),
                io::ErrorKind::PermissionDenied
            );
            assert!(!directory.join("spawned").exists());
            if let Some(child) = background.as_mut() {
                child.kill().expect("kill temporary foreground owner");
                child.wait().expect("reap temporary foreground owner");
            }
            return;
        }
        match result.expect("supervise login") {
            Outcome::Exited(status) => std::process::exit(status.code().expect("real exit code")),
            Outcome::Cancelled => panic!("Unix cancellation should kill helper"),
        }
    });
}

struct Attempt {
    child: Child,
    directory: tempfile::TempDir,
    reaped: bool,
}

impl Attempt {
    fn spawn(mode: &str, pty: bool, own_group: bool) -> (Self, Option<fs::File>) {
        let directory = tempfile::tempdir().expect("fixture directory");
        let mut command = Command::new(std::env::current_exe().expect("test executable"));
        let test_module = module_path!()
            .split_once("::")
            .expect("test module is below crate root")
            .1;
        command
            .args(["--exact", &format!("{test_module}::fixture"), "--nocapture"])
            .env(MODE, mode)
            .env(DIRECTORY, directory.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());
        let master = if pty {
            let pair = nix::pty::openpty(None, None).expect("open real PTY");
            command.stdin(Stdio::from(pair.slave));
            // SAFETY: only async-signal-safe system calls run after fork.
            // stdin already refers to this new PTY slave, not a user's terminal.
            unsafe {
                command.pre_exec(|| {
                    if nix::libc::setsid() == -1 {
                        return Err(io::Error::last_os_error());
                    }
                    if nix::libc::ioctl(0, nix::libc::TIOCSCTTY, 0) == -1 {
                        return Err(io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
            Some(fs::File::from(pair.master))
        } else {
            if own_group {
                command.process_group(0);
            }
            None
        };
        (
            Self {
                child: command.spawn().expect("spawn isolated fixture"),
                directory,
                reaped: false,
            },
            master,
        )
    }

    fn ready(&self) -> [i32; 2] {
        let read_pid = |name| {
            wait_until(|| {
                fs::read_to_string(self.directory.path().join(name))
                    .ok()
                    .and_then(|value| value.trim().parse::<i32>().ok())
            })
        };
        [read_pid("root"), read_pid("grandchild")]
    }

    fn cancel(&self) {
        fs::write(self.directory.path().join("cancel"), "").expect("request cancellation");
    }

    fn wait(&mut self) -> ExitStatus {
        let status = wait_until(|| self.child.try_wait().expect("poll fixture"));
        self.reaped = true;
        status
    }
}

impl Drop for Attempt {
    fn drop(&mut self) {
        if !self.reaped {
            // The unreaped direct child pins its PID. Only signal an owned
            // fixture group, never the test runner's group.
            let pid = Pid::from_raw(self.child.id() as i32);
            if nix::unistd::getpgid(Some(pid)).ok() == Some(pid) {
                let _ = killpg(pid, Signal::SIGKILL);
            } else {
                let _ = self.child.kill();
            }
            let _ = self.child.wait();
        }
    }
}

fn wait_until<T>(mut check: impl FnMut() -> Option<T>) -> T {
    let start = Instant::now();
    loop {
        if let Some(value) = check() {
            return value;
        }
        assert!(start.elapsed() < DEADLINE, "fixture timed out");
        thread::sleep(Duration::from_millis(10));
    }
}

fn assert_terminated(pid: i32) {
    // Orphan zombies depend on the host's PID 1 reaper. They are terminated,
    // not running login processes; do not require a particular init system.
    wait_until(|| {
        let output = Command::new("ps")
            .args(["-o", "stat=", "-p", &pid.to_string()])
            .output()
            .expect("inspect fixture process state");
        let state = String::from_utf8_lossy(&output.stdout);
        (state.trim().is_empty() || state.trim_start().starts_with('Z')).then_some(())
    });
}

#[test]
fn cancellation_kills_term_ignoring_tree_without_touching_other_attempt() {
    let (mut first, _) = Attempt::spawn("cancel", false, true);
    let (mut other, _) = Attempt::spawn("cancel", false, true);
    let pids = first.ready();
    let other_pids = other.ready();
    first.cancel();
    assert_eq!(first.wait().signal(), Some(nix::libc::SIGKILL));
    for pid in pids {
        assert_terminated(pid);
    }
    assert!(
        other
            .child
            .try_wait()
            .expect("other helper alive")
            .is_none()
    );
    for pid in other_pids {
        kill(Pid::from_raw(pid), None).expect("other login tree still alive");
    }
    other.cancel();
    assert_eq!(other.wait().signal(), Some(nix::libc::SIGKILL));
    for pid in other_pids {
        assert_terminated(pid);
    }
}

#[test]
fn dropping_run_kills_tree() {
    let (mut attempt, _) = Attempt::spawn("drop", false, true);
    let pids = attempt.ready();
    attempt.cancel();
    assert_eq!(attempt.wait().signal(), Some(nix::libc::SIGKILL));
    for pid in pids {
        assert_terminated(pid);
    }
}

#[test]
fn cancellation_kills_descendants_even_when_root_obeys_term() {
    let (mut attempt, _) = Attempt::spawn("responsive", false, true);
    let pids = attempt.ready();
    attempt.cancel();
    assert_eq!(attempt.wait().signal(), Some(nix::libc::SIGKILL));
    for pid in pids {
        assert_terminated(pid);
    }
}

#[test]
fn helper_signals_cancel_tree() {
    for signal in [Signal::SIGINT, Signal::SIGHUP, Signal::SIGTERM] {
        let (mut attempt, _) = Attempt::spawn("signal", false, true);
        let pids = attempt.ready();
        kill(Pid::from_raw(attempt.child.id() as i32), signal).expect("signal helper");
        assert_eq!(attempt.wait().signal(), Some(nix::libc::SIGKILL));
        for pid in pids {
            assert_terminated(pid);
        }
    }
}

#[test]
fn rejects_group_not_owned_before_spawning() {
    let (mut attempt, _) = Attempt::spawn("reject", false, false);
    assert!(attempt.wait().success());
    assert!(!attempt.directory.path().join("spawned").exists());
}

#[test]
fn returns_real_exit_code_without_killing_helper() {
    let (mut attempt, _) = Attempt::spawn("exit", false, true);
    assert_eq!(attempt.wait().code(), Some(37));
}

#[test]
fn foreground_login_can_read_real_pty_without_sigttin() {
    let (mut attempt, master) = Attempt::spawn("pty", true, true);
    let mut master = master.expect("PTY master");
    master
        .write_all(b"terminal-input\n")
        .expect("terminal input");
    assert_eq!(attempt.wait().code(), Some(37));
}

#[test]
fn rejects_background_terminal_before_spawning() {
    let (mut attempt, _master) = Attempt::spawn("background", true, true);
    assert!(attempt.wait().success());
}
