"""Boot the system-test guest and hand its shell to the user's terminal."""

import subprocess

serial_stdout_off()
start_all()
machine.wait_for_unit("multi-user.target")
machine.wait_for_unit("bas-test-notifications.service", user="bas")
machine.succeed("systemctl start bas")
machine.wait_for_unit("bas.service")
print("Guest root shell; bas is running as user bas.")
print("Try: bast agents")
print("Try: bast session create --agent embedded --cwd /home/bas --title manual")
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
