{
  lib,
  rustPlatform,
}:
rustPlatform.buildRustPackage {
  pname = "acp-test-agent";
  version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;

  src = lib.fileset.toSource {
    root = ./.;
    fileset = lib.fileset.unions [
      ./Cargo.toml
      ./Cargo.lock
      ./src
      ./tests
      ./README.md
      ./LICENSE
    ];
  };
  cargoLock.lockFile = ./Cargo.lock;
  cargoTestFlags = [ "--all-targets" ];

  meta = {
    description = "Scripted ACP v1/v2 agent for testing clients without a model";
    license = lib.licenses.mit;
    platforms = lib.platforms.linux;
    mainProgram = "acp-test-agent";
  };
}
