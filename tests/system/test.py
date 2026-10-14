"""Black-box tests against the installed binaries and real guest OS services."""

import re
import shlex


def cli(*args: str) -> str:
    return shlex.join(["runuser", "-u", "bas", "--", "bast", *args])


def create(agent: str, title: str) -> str:
    output = machine.succeed(cli(
        "session", "create", "--agent", agent, "--title", title,
        "--cwd", "/home/bas",
    ))
    match = re.search(r"^id\s+([0-9a-f-]{36})$", output, re.MULTILINE)
    assert match is not None, output
    return match.group(1)


def wait_for_turn(session: str, reason: str = "end_turn") -> str:
    command = cli("history", session)
    machine.wait_until_succeeds(
        command + " | grep -F " + shlex.quote(f"finished ({reason})"), timeout=60
    )
    return machine.succeed(command)


def user_service(action: str) -> str:
    return shlex.join([
        "runuser", "-u", "bas", "--", "env", "XDG_RUNTIME_DIR=/run/user/1000",
        "systemctl", "--user", action, "bas-test-notifications.service",
    ])


def save_diagnostics() -> None:
    try:
        machine.succeed(
            "journalctl --no-pager -b > /tmp/bas-system-journal.log"
        )
        machine.copy_from_machine("/tmp/bas-system-journal.log")
        machine.succeed("systemctl cat bas > /tmp/bas-unit.txt")
        machine.copy_from_machine("/tmp/bas-unit.txt")
        if machine.execute("test -f /run/user/1000/bas-notifications.jsonl")[0] == 0:
            machine.copy_from_machine("/run/user/1000/bas-notifications.jsonl")
        if machine.execute("test -f /home/bas/attach.log")[0] == 0:
            machine.copy_from_machine("/home/bas/attach.log")
    except Exception as error:
        print(f"Unable to collect all guest diagnostics: {error}")


start_all()
try:
    with subtest("systemd readiness, unprivileged service, and journald"):
        machine.wait_for_unit("multi-user.target")
        machine.wait_for_unit("bas-test-notifications.service", user="bas")
        machine.succeed("systemctl start bas")
        machine.wait_for_unit("bas.service")
        machine.wait_for_open_port(7337)
        assert machine.succeed("systemctl show bas -p Type --value").strip() == "notify"
        pid = machine.succeed("systemctl show bas -p MainPID --value").strip()
        assert machine.succeed(f"ps -o user= -p {pid}").strip() == "bas"
        agents = machine.succeed(cli("agents"))
        assert "embedded (embedded)" in agents and "normal (acp)" in agents, agents
        machine.succeed(
            "journalctl -u bas -o json --no-pager | "
            "jq -e 'select(.MESSAGE | contains(\"serving the bas.v1.Bas service\"))'"
        )

    with subtest("periodic watchdog notifications"):
        before = int(machine.succeed(
            "systemctl show bas -p WatchdogTimestampMonotonic --value"
        ).strip())
        assert before > 0
        machine.wait_until_succeeds(
            f'test "$(systemctl show bas -p WatchdogTimestampMonotonic --value)" -gt {before}',
            timeout=45,
        )
        assert machine.succeed("systemctl show bas -p MainPID --value").strip() == pid
        machine.succeed("systemctl is-active bas")

    with subtest("CLI sessions, history pagination, and actual D-Bus delivery"):
        echo = create("embedded", "vm-echo")
        machine.succeed(cli("prompt", echo, "hello from the virtual machine"))
        history = wait_for_turn(echo)
        assert "hello from the virtual machine" in history and "echo agent:" in history, history
        first_page = machine.succeed(cli("history", echo, "--limit", "2"))
        cursor = re.search(r"--after (\d+)", first_page)
        assert cursor is not None, first_page
        second_page = machine.succeed(cli("history", echo, "--after", cursor.group(1)))
        assert "hello from the virtual machine" in second_page, second_page
        machine.wait_until_succeeds(
            "jq -e 'select(.app == \"bas\" and .title == \"vm-echo\" and .urgency == 1)' "
            "/run/user/1000/bas-notifications.jsonl",
            timeout=30,
        )
        machine.succeed(cli("session", "rename", echo, "renamed in VM"))
        assert "renamed in VM" in machine.succeed(cli("session", "show", echo))
        machine.succeed(cli("session", "remove", echo))
        machine.fail(cli("session", "show", echo))

    with subtest("ACP process, normalized messages, and cancellation"):
        normal = create("normal", "vm-acp")
        machine.succeed(cli("prompt", normal, "test ACP"))
        assert "thinkinganswer" in wait_for_turn(normal)
        child = int(machine.succeed("cat /home/bas/bas-test-normal.pid"))
        machine.succeed(cli("session", "remove", normal))
        machine.wait_until_succeeds(f"test ! -e /proc/{child}", timeout=30)

        cancellable = create("cancel", "vm-cancel")
        machine.succeed(cli("prompt", cancellable, "wait for cancellation"))
        machine.wait_for_file("/home/bas/bas-test-cancel.pid.prompt")
        machine.succeed(cli("cancel", cancellable))
        wait_for_turn(cancellable, "cancelled")
        machine.succeed(cli("session", "remove", cancellable))

    with subtest("graceful systemd stop closes subscribers and removes ACP children"):
        stuck = create("stuck", "vm-shutdown")
        attached = (
            f"bast attach {stuck} --cursor 0 > /home/bas/attach.log 2>&1; "
            "echo $? > /home/bas/attach.exit"
        )
        machine.succeed(
            "runuser -u bas -- sh -c " + shlex.quote(attached)
            + " </dev/null >/dev/null 2>&1 &"
        )
        machine.wait_until_succeeds(
            "pgrep -u bas -f 'bast attach'", timeout=30
        )
        machine.succeed(cli("prompt", stuck, "keep running until shutdown"))
        machine.wait_for_file("/home/bas/bas-test-stuck.pid.prompt")
        machine.wait_until_succeeds(
            "grep -F 'started' /home/bas/attach.log", timeout=30
        )
        child = int(machine.succeed("cat /home/bas/bas-test-stuck.pid"))
        machine.succeed("systemctl stop bas", timeout=30)
        assert machine.succeed("systemctl show bas -p Result --value").strip() == "success"
        assert machine.succeed("systemctl show bas -p ExecMainStatus --value").strip() == "0"
        machine.wait_until_succeeds(f"test ! -e /proc/{child}", timeout=30)
        machine.wait_for_file("/home/bas/attach.exit")
        assert machine.succeed("cat /home/bas/attach.exit").strip() == "0"
        assert "status stopped" in machine.succeed("cat /home/bas/attach.log")
        machine.wait_for_closed_port(7337)

    with subtest("required notifications fail without a desktop service"):
        machine.succeed(user_service("stop"))
        machine.fail("systemctl start bas", timeout=30)
        machine.succeed("systemctl is-failed bas")
        machine.succeed(
            "journalctl -u bas --no-pager | grep -F 'notifications were required'"
        )

    with subtest("headless startup and clean restart"):
        override = (
            "[Service]\nEnvironment=BAS_NOTIFY=auto\n"
            "Environment=DBUS_SESSION_BUS_ADDRESS=unix:path=/run/nonexistent-bas-bus\n"
        )
        machine.succeed("mkdir -p /run/systemd/system/bas.service.d")
        machine.succeed(
            "printf %s " + shlex.quote(override)
            + " > /run/systemd/system/bas.service.d/headless.conf"
        )
        machine.succeed("systemctl daemon-reload; systemctl reset-failed bas; systemctl start bas")
        machine.wait_for_unit("bas.service")
        session = create("embedded", "headless")
        machine.succeed(cli("prompt", session, "no desktop required"))
        wait_for_turn(session)
        machine.succeed("systemctl restart bas")
        assert machine.succeed(cli("session", "list")).strip() == "no sessions"
        machine.succeed("systemctl stop bas")
finally:
    save_diagnostics()
