{ pkgs }:
{ lib, ... }:
{
  extraPythonPackages = ps: [ ps.python-dotenv ];
  sshBackdoor.enable = true;
  nodes.machine.services.openssh.openFirewall = false;
  testScript = lib.mkForce (
    builtins.replaceStrings
      [ "@ssh@" "@ssh_config@" "@ssh_proxy@" ]
      [
        "${pkgs.openssh}/bin/ssh"
        "${pkgs.systemd}/lib/systemd/ssh_config.d/20-systemd-ssh-proxy.conf"
        "${pkgs.systemd}/lib/systemd/systemd-ssh-proxy"
      ]
      (builtins.readFile ../../tests/system/pi_config.py + builtins.readFile ../../tests/system/shell.py)
  );
  # A manual debugging session is not subject to the test suite's deadline.
  globalTimeout = lib.mkForce (24 * 60 * 60);
  extraDriverArgs = lib.mkForce [ ];
}
