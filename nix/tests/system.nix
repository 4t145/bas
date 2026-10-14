{ pkgs, package }:
let
  python = pkgs.python3.withPackages (ps: [ ps.dbus-next ]);
  notificationService = pkgs.writeShellScript "bas-test-notifications" ''
    exec ${python}/bin/python ${../../tests/system/notifications.py}
  '';
  fixture =
    name: mode:
    pkgs.writeShellScriptBin name ''
      export BAS_FIXTURE_MODE=${mode}
      export BAS_FIXTURE_PID="$PWD/${name}.pid"
      exec ${pkgs.bash}/bin/bash ${../../src/bin/bas/agent/acp/assets/fixture.sh}
    '';
  normal = fixture "bas-test-normal" "normal";
  cancellable = fixture "bas-test-cancel" "wait_cancel";
  stuck = fixture "bas-test-stuck" "ignore_cancel";
in
{
  name = "bas-system";
  hostPkgs = pkgs;
  requiredFeatures.kvm = false;
  qemu.forceAccel = false;
  globalTimeout = 15 * 60;
  extraDriverArgs = [
    "--junit-xml"
    "junit.xml"
  ];
  interactive.qemu.package = pkgs.lib.mkForce pkgs.qemu_test;

  nodes.machine = { ... }: {
    virtualisation = {
      memorySize = 1536;
      cores = 2;
      graphics = false;
    };
    users.groups.bas = { };
    users.users.bas = {
      isNormalUser = true;
      uid = 1000;
      group = "bas";
      home = "/home/bas";
      createHome = true;
      linger = true;
    };
    services.dbus.enable = true;
    environment.systemPackages = [
      package
      pkgs.jq
      pkgs.procps
      pkgs.util-linux
    ];

    systemd.user.services.bas-test-notifications = {
      description = "Recording desktop notification service for Bas tests";
      wantedBy = [ "default.target" ];
      serviceConfig = {
        Type = "dbus";
        BusName = "org.freedesktop.Notifications";
        ExecStart = notificationService;
      };
    };

    systemd.services.bas = {
      description = "Bas system test daemon";
      after = [ "user@1000.service" ];
      requires = [ "user@1000.service" ];
      path = [
        pkgs.coreutils
        pkgs.gnused
      ];
      environment = {
        BAS_LISTEN = "127.0.0.1:7337";
        BAS_AGENTS = "embedded;normal=acp:${normal}/bin/bas-test-normal;cancel=acp:${cancellable}/bin/bas-test-cancel;stuck=acp:${stuck}/bin/bas-test-stuck";
        BAS_STORAGE = "memory";
        BAS_NOTIFY = "always";
        BAS_LOG_BACKEND = "journald";
        BAS_STARTUP_TIMEOUT = "60";
        DBUS_SESSION_BUS_ADDRESS = "unix:path=/run/user/1000/bus";
      };
      serviceConfig = {
        Type = "notify";
        ExecStart = "${package}/bin/bas";
        User = "bas";
        Group = "bas";
        WorkingDirectory = "/home/bas";
        WatchdogSec = "20s";
        TimeoutStartSec = "90s";
        TimeoutStopSec = "15s";
        NoNewPrivileges = true;
        PrivateTmp = true;
        ProtectSystem = "strict";
        ProtectHome = "read-only";
        ReadWritePaths = [ "/home/bas" ];
      };
    };
  };
  testScript = builtins.readFile ../../tests/system/test.py;
}
