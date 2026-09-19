{ pkgs, package }:
let
  python = pkgs.python3.withPackages (ps: [ ps.dbus-next ]);
  notificationService = pkgs.writeShellScript "basr-test-notifications" ''
    exec ${python}/bin/python ${../../tests/system/notifications.py}
  '';
  fixture =
    name: mode:
    pkgs.writeShellScriptBin name ''
      export BASR_FIXTURE_MODE=${mode}
      export BASR_FIXTURE_PID="$PWD/${name}.pid"
      exec ${pkgs.bash}/bin/bash ${../../src/bin/basrd/agent/acp/assets/fixture.sh}
    '';
  normal = fixture "basr-test-normal" "normal";
  cancellable = fixture "basr-test-cancel" "wait_cancel";
  stuck = fixture "basr-test-stuck" "ignore_cancel";
  piAcp = pkgs.callPackage ../pi-acp { };
in
{
  name = "basr-system";
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
    users.groups.basr = { };
    users.users.basr = {
      isNormalUser = true;
      uid = 1000;
      group = "basr";
      home = "/home/basr";
      createHome = true;
      linger = true;
    };
    services.dbus.enable = true;
    environment.systemPackages = [
      package
      pkgs.jq
      pkgs.procps
      pkgs.util-linux
      pkgs.pi-coding-agent
      piAcp
    ];
    systemd.tmpfiles.rules = [
      "d /home/basr/.pi 0700 basr basr -"
      "d /home/basr/.pi/agent 0700 basr basr -"
      "C /home/basr/.pi/agent/settings.json 0600 basr basr - ${../../tests/system/pi/settings.json}"
      "C /home/basr/.pi/agent/models.json 0600 basr basr - ${../../tests/system/pi/models.json}"
    ];

    systemd.user.services.basr-test-notifications = {
      description = "Recording desktop notification service for BASR tests";
      wantedBy = [ "default.target" ];
      serviceConfig = {
        Type = "dbus";
        BusName = "org.freedesktop.Notifications";
        ExecStart = notificationService;
      };
    };

    systemd.services.basrd = {
      description = "BASR system test daemon";
      after = [ "user@1000.service" ];
      requires = [ "user@1000.service" ];
      path = [
        pkgs.coreutils
        pkgs.gnused
      ];
      environment = {
        BASRD_LISTEN = "127.0.0.1:7337";
        BASRD_AGENTS = "embedded;pi=acp:${piAcp}/bin/pi-acp;normal=acp:${normal}/bin/basr-test-normal;cancel=acp:${cancellable}/bin/basr-test-cancel;stuck=acp:${stuck}/bin/basr-test-stuck";
        BASRD_NOTIFY = "always";
        BASRD_LOG_BACKEND = "journald";
        BASRD_STARTUP_TIMEOUT = "60";
        PI_OFFLINE = "1";
        DBUS_SESSION_BUS_ADDRESS = "unix:path=/run/user/1000/bus";
      };
      serviceConfig = {
        Type = "notify";
        ExecStart = "${package}/bin/basrd";
        User = "basr";
        Group = "basr";
        WorkingDirectory = "/home/basr";
        WatchdogSec = "20s";
        TimeoutStartSec = "90s";
        TimeoutStopSec = "15s";
        NoNewPrivileges = true;
        PrivateTmp = true;
        ProtectSystem = "strict";
        ProtectHome = "read-only";
        ReadWritePaths = [ "/home/basr" ];
      };
    };
  };
  testScript = builtins.readFile ../../tests/system/test.py;
}
