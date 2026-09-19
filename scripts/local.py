"""Run BASR and the pinned pi backend on the local desktop."""

import argparse
import json
import os
from pathlib import Path
import re
import secrets
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import time

from dotenv import dotenv_values

ROOT = Path(__file__).resolve().parent.parent
STARTUP_TIMEOUT = 30
TURN_TIMEOUT = 180
SHUTDOWN_TIMEOUT = 15
POLL_INTERVAL = 0.2


def arguments():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", nargs="?", choices=("gui", "daemon", "smoke"), default="gui")
    parser.add_argument("--port", type=int, help="7337 by default; smoke uses an automatic port")
    parser.add_argument("--cwd", type=Path, help="default GUI session directory")
    layout = parser.add_mutually_exclusive_group()
    layout.add_argument("--compact", action="store_true", help="minimal popup (default)")
    layout.add_argument("--window", action="store_true", help="workspace and thread window")
    parser.add_argument("--no-build", action="store_true")
    args = parser.parse_args()
    if args.port is None:
        args.port = 0 if args.mode == "smoke" else 7337
    if not 0 <= args.port <= 65535:
        parser.error("--port must be between 0 and 65535")
    if args.cwd is not None and not args.cwd.is_dir():
        parser.error("--cwd must be an existing directory")
    return args


def load_key():
    key = os.environ.get("DEEPSEEK_API_KEY")
    if not key:
        key = dotenv_values(ROOT / ".env", interpolate=False).get("DEEPSEEK_API_KEY")
    if not key or not key.strip():
        raise RuntimeError("Set DEEPSEEK_API_KEY in .env or the environment; see .env.example.")
    return key.strip()


def build(args, env):
    # Keep Cargo outputs in the workspace even when the caller uses a different target directory.
    if args.no_build:
        return
    command = ["cargo", "build", "--locked", "--target-dir", str(ROOT / "target"),
               "--bin", "basrd", "--bin", "basrctl"]
    if args.mode == "gui":
        command += ["--features", "gui", "--bin", "basrw"]
    subprocess.run(command, cwd=ROOT, env=env, check=True)


def configure_pi(run_dir):
    agent_dir = run_dir / "pi-agent"
    agent_dir.mkdir(mode=0o700)
    for name in ("settings.json", "models.json"):
        shutil.copyfile(ROOT / "tests/system/pi" / name, agent_dir / name)
    # Use the environment resolver explicitly; no credential is written to disk.
    models_path = agent_dir / "models.json"
    models = json.loads(models_path.read_text())
    models["providers"]["deepseek"]["apiKey"] = "DEEPSEEK_API_KEY"
    models_path.write_text(json.dumps(models, indent=2) + "\n")
    return agent_dir


def stop(process):
    """Gracefully stop our process group, including any surviving pi children."""
    if process is None:
        return
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        return
    try:
        process.wait(timeout=SHUTDOWN_TIMEOUT)
    except subprocess.TimeoutExpired:
        pass
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    process.wait()


def cli(env, server, *args, timeout=90):
    result = subprocess.run(
        [str(ROOT / "target/debug/basrctl"), "--server", server, *args],
        env=env, text=True, capture_output=True, timeout=timeout,
    )
    if result.returncode:
        raise RuntimeError(result.stderr.strip() or "basrctl failed")
    return result.stdout


def wait_ready(process, log_path, env):
    deadline = time.monotonic() + STARTUP_TIMEOUT
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError(f"basrd exited; check {log_path} (the port may already be in use).")
        # Only accept the address reported by the daemon we started, never an existing service.
        match = re.search(r"serving the basr\.v1\.Basr service.*address=(127\.0\.0\.1:\d+)",
                          log_path.read_text())
        if match:
            server = match[1]
            cli(env, server, "agents", timeout=5)
            return server
        time.sleep(POLL_INTERVAL)
    raise RuntimeError(f"basrd did not become ready; check {log_path}")


def smoke(env, server, work_dir, run_dir):
    content = f"basr-pi-{secrets.token_hex(12)}\n"
    (work_dir / "input.txt").write_text(content)
    output = cli(env, server, "session", "create", "--agent", "pi",
                 "--title", "local-pi-smoke", "--cwd", str(work_dir))
    match = re.search(r"^id\s+([0-9a-f-]{36})$", output, re.MULTILINE)
    if not match:
        raise RuntimeError("basrctl did not return a session ID")
    session = match[1]
    print(f"Pi session: {session}. Running a real DeepSeek turn…", flush=True)
    try:
        cli(env, server, "prompt", session,
            "Work only in the current directory. Use the read tool to read input.txt. "
            "Use the write tool to create output.txt containing exactly the input converted "
            "to uppercase, preserving its final newline. Read output.txt to verify it. "
            "Finally reply with BASR_PI_OK.")
        deadline = time.monotonic() + TURN_TIMEOUT
        while time.monotonic() < deadline:
            history = cli(env, server, "history", session, "--limit", "4096", timeout=5)
            (run_dir / "history.log").write_text(history)
            if "---- failed:" in history or "finished (" in history:
                break
            time.sleep(1)
        else:
            raise RuntimeError(f"Pi turn timed out; check {run_dir / 'history.log'}")
        target = work_dir / "output.txt"
        if ("finished (end_turn)" not in history
                or not re.search(r"^\s*\d+\s+agent\s+.*BASR_PI_OK", history, re.MULTILINE)
                or len(re.findall(r"tool\s+.*\[completed\]", history)) < 2
                or not target.is_file() or target.read_text() != content.upper()):
            raise RuntimeError(f"Pi smoke test failed; check {run_dir / 'history.log'}")
        print("PASS: basrctl → basrd → pi-acp → pi → DeepSeek; file tools and reply verified.", flush=True)
    finally:
        cli(env, server, "session", "remove", session, timeout=15)


def run(args):
    key = load_key()
    # Only the daemon and its backend inherit the key, not Cargo, CLI, or GUI.
    client_env = os.environ.copy()
    client_env.pop("DEEPSEEK_API_KEY", None)
    build(args, client_env)
    results = ROOT / ".test-results"
    results.mkdir(exist_ok=True)
    run_dir = Path(tempfile.mkdtemp(prefix="local.", dir=results))
    work_dir = run_dir / "work"
    work_dir.mkdir()
    cwd = args.cwd.resolve() if args.cwd and args.mode != "smoke" else work_dir
    agent_dir = configure_pi(run_dir)
    daemon_env = client_env | {
        "DEEPSEEK_API_KEY": key,
        "PI_CODING_AGENT_DIR": str(agent_dir),
        "PI_OFFLINE": "1",
        "NO_COLOR": "1",
    }
    # A dev process must not notify or inherit a parent systemd unit's watchdog.
    for name in ("NOTIFY_SOCKET", "WATCHDOG_USEC", "WATCHDOG_PID"):
        daemon_env.pop(name, None)
    print(f"Logs and test files: {run_dir}", flush=True)
    daemon = gui = None
    try:
        with (run_dir / "daemon.log").open("w") as log:
            daemon = subprocess.Popen(
                [str(ROOT / "target/debug/basrd"), "--listen", f"127.0.0.1:{args.port}",
                 "--agents", "pi=acp:pi-acp", "--notify", "never", "--startup-timeout", "60",
                 "--log-backend", "stdout", "--log-filter", "info"],
                cwd=work_dir, env=daemon_env, stdout=log, stderr=subprocess.STDOUT,
                start_new_session=True,
            )
            server = wait_ready(daemon, run_dir / "daemon.log", client_env)
            print(f"basrd ready: {server}; backend: pi / DeepSeek", flush=True)
            print(shlex.join([str(ROOT / "target/debug/basrctl"), "--server", server,
                              "session", "create", "--agent", "pi", "--cwd", str(cwd)]), flush=True)
            if args.mode == "smoke":
                smoke(client_env, server, work_dir, run_dir)
            elif args.mode == "daemon":
                print("Ctrl-C stops this daemon and its pi sessions.", flush=True)
                if daemon.wait():
                    raise RuntimeError(f"basrd exited; check {run_dir / 'daemon.log'}")
            else:
                command = [str(ROOT / "target/debug/basrw"), "--server", server, "--cwd", str(cwd)]
                if args.compact:
                    command.append("--compact")
                if args.window:
                    command.append("--window")
                with (run_dir / "gui.log").open("w") as gui_log:
                    gui = subprocess.Popen(command, env=client_env, stdout=gui_log,
                                           stderr=subprocess.STDOUT, start_new_session=True)
                    print("Close the GUI or press Ctrl-C to stop this local test environment.", flush=True)
                    while gui.poll() is None:
                        if daemon.poll() is not None:
                            raise RuntimeError(f"basrd exited; check {run_dir / 'daemon.log'}")
                        time.sleep(POLL_INTERVAL)
                    if gui.returncode:
                        raise RuntimeError(f"GUI exited; check {run_dir / 'gui.log'}")
    finally:
        stop(gui)
        stop(daemon)


def interrupted(_signal, _frame):
    raise KeyboardInterrupt


if __name__ == "__main__":
    signal.signal(signal.SIGTERM, interrupted)
    try:
        run(arguments())
    except KeyboardInterrupt:
        print("Local BASR stopped.", flush=True)
        sys.exit(130)
    except (RuntimeError, OSError, subprocess.SubprocessError) as error:
        print(f"Error: {error}", file=sys.stderr)
        sys.exit(1)
