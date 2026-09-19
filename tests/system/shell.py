"""Boot the system-test guest and hand its shell to the user's terminal."""

import subprocess

serial_stdout_off()
start_all()
machine.wait_for_unit("multi-user.target")
machine.wait_for_unit("basr-test-notifications.service", user="basr")
configure_pi()
machine.succeed("systemctl start basrd")
machine.wait_for_unit("basrd.service")
print("Guest root shell; basrd is running as user basr.")
print("Try: basrctl agents")
print("Try: basrctl session create --agent pi --cwd /home/basr --title manual")
print("Use exit or Ctrl-D to leave and close this disposable VM.")
try:
    shell = subprocess.run(
        [
            "@ssh@", "-F", "@ssh_config@", "-t",
            "-o", "User=root", "-o", "ConnectTimeout=10",
            # %n preserves the socket path's case; the default %h lowercases it.
            "-o", "ProxyCommand=@ssh_proxy@ %n %p",
            f"vsock-mux/{machine.vsock_host}",
        ],
        check=False,
    )
    # A shell may exit with its last command's status, including after Ctrl-C.
    if shell.returncode == 255:
        raise RuntimeError("The SSH connection to the VM failed.")
finally:
    try:
        machine.shutdown()
    finally:
        (machine.out_dir / "console.log").write_text(machine.get_console_log())
