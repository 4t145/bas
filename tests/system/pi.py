"""Live model test: basrctl -> basrd -> pi-acp -> pi -> DeepSeek."""

import re
import secrets
import shlex


def pi_cli(*args: str) -> str:
    return shlex.join(["runuser", "-u", "basr", "--", "basrctl", *args])


serial_stdout_off()
session: str | None = None
start_all()
try:
    with subtest("pi authentication and ACP session startup"):
        machine.wait_for_unit("multi-user.target")
        machine.wait_for_unit("basr-test-notifications.service", user="basr")
        assert configure_pi(), "Set DEEPSEEK_API_KEY in .env or the environment"
        machine.succeed("systemctl start basrd")
        machine.wait_for_unit("basrd.service")
        machine.succeed("runuser -u basr -- pi --version")
        machine.succeed("install -d -o basr -g basr /home/basr/pi-smoke")
        output = machine.succeed(pi_cli(
            "session", "create", "--agent", "pi", "--title", "pi-live",
            "--cwd", "/home/basr/pi-smoke",
        ), timeout=90)
        match = re.search(r"^id\s+([0-9a-f-]{36})$", output, re.MULTILINE)
        assert match is not None, output
        session = match.group(1)
        daemon = int(machine.succeed("systemctl show basrd -p MainPID --value"))
        children = {
            int(pid) for pid in machine.succeed(
                "cat /sys/fs/cgroup/system.slice/basrd.service/cgroup.procs"
            ).split()
        } - {daemon}
        assert len(children) >= 2, "Expected both pi-acp and the real pi process"

    with subtest("DeepSeek generation and real pi file tools"):
        content = f"basr-pi-{secrets.token_hex(12)}\n"
        machine.succeed(
            "printf %s " + shlex.quote(content) + " > /home/basr/pi-smoke/input.txt; "
            "chown basr:basr /home/basr/pi-smoke/input.txt"
        )
        prompt = (
            "Work only in the current directory. Use the read tool to read input.txt. "
            "Use the write tool to create output.txt with exactly the input content "
            "converted to uppercase, preserving the final newline. "
            "Read output.txt to verify it. Finally reply with BASR_PI_OK."
        )
        machine.succeed(pi_cli("prompt", session, prompt))
        # Tool argument streaming produces many updates; the default 50-event
        # page may end before TurnFinished. Read the full test retention window.
        history_command = pi_cli("history", session, "--limit", "4096")
        machine.wait_until_succeeds(
            history_command + " | grep -E 'finished \\(|---- failed:'",
            timeout=180,
        )
        history = machine.succeed(history_command)
        (machine.out_dir / "pi-history.log").write_text(history)
        assert "finished (end_turn)" in history, history
        assert re.search(r"^\s*\d+\s+agent\s+.*BASR_PI_OK", history, re.MULTILINE), history
        assert len(re.findall(r"tool\s+.*\[completed\]", history)) >= 2, history
        assert machine.succeed("cat /home/basr/pi-smoke/output.txt") == content.upper()
        machine.copy_from_machine("/home/basr/pi-smoke/output.txt")

    with subtest("removing the session reaps pi and its adapter"):
        machine.succeed(pi_cli("session", "remove", session))
        for pid in children:
            machine.wait_until_succeeds(f"test ! -e /proc/{pid}", timeout=30)
        machine.succeed("systemctl stop basrd")
        print("Live pi + DeepSeek test passed, including file tools and process cleanup.")
finally:
    if machine.is_up():
        try:
            if session is not None:
                status, history = machine.execute(
                    pi_cli("history", session, "--limit", "4096"), timeout=15
                )
                if status == 0:
                    (machine.out_dir / "pi-history.log").write_text(history)
            machine.succeed("journalctl -u basrd --no-pager > /tmp/pi-daemon.log")
            machine.copy_from_machine("/tmp/pi-daemon.log")
        finally:
            machine.shutdown()
    (machine.out_dir / "console.log").write_text(machine.get_console_log())
