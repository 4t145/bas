{ lib, ... }:
{
  extraPythonPackages = ps: [ ps.python-dotenv ];
  testScript = lib.mkForce (
    builtins.readFile ../../tests/system/pi_config.py + builtins.readFile ../../tests/system/pi.py
  );
  globalTimeout = lib.mkForce (10 * 60);
  extraDriverArgs = lib.mkForce [ ];
}
