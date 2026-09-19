{
  lib,
  pkgs,
  rustPlatform,
  pkg-config,
  makeWrapper,
  makeDesktopItem,
  gui ? false,
}:
let
  desktopLibraries = import ./desktop-libraries.nix { inherit pkgs; };
  desktopItem = makeDesktopItem {
    name = "org.basr.Gui";
    desktopName = "BASR";
    genericName = "Agent Workspace";
    comment = "Manage BASR sessions and chat with agents";
    exec = "basrw";
    terminal = false;
    categories = [
      "Development"
      "Utility"
    ];
    startupWMClass = "org.basr.Gui";
  };
in
rustPlatform.buildRustPackage {
  pname = if gui then "basrw" else "basr";
  version = (builtins.fromTOML (builtins.readFile ../Cargo.toml)).package.version;

  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../build.rs
      ../src
      ../vendor
      ../proto
      (lib.fileset.difference ../tests ../tests/system)
    ];
  };
  cargoLock.lockFile = ../Cargo.lock;
  buildFeatures = lib.optionals gui [ "gui" ];
  cargoBuildFlags = lib.optionals gui [
    "--bin"
    "basrw"
  ];
  cargoTestFlags =
    if gui then
      [
        "--bin"
        "basrw"
      ]
    else
      [ "--all-targets" ];
  nativeBuildInputs = lib.optionals gui [
    pkg-config
    makeWrapper
  ];
  buildInputs = lib.optionals gui desktopLibraries;
  postInstall =
    if gui then
      ''
        wrapProgram "$out/bin/basrw" \
          --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath desktopLibraries}
        install -Dm644 ${desktopItem}/share/applications/org.basr.Gui.desktop \
          "$out/share/applications/org.basr.Gui.desktop"
      ''
    else
      ''
        install -Dm644 ${../contrib/basrd-user.service} "$out/share/systemd/user/basrd.service"
        substituteInPlace "$out/share/systemd/user/basrd.service" \
          --replace-fail '@basrd@' "$out/bin/basrd"
        install -Dm644 ${../contrib/agents.json} "$out/share/basr/agents.example.json"
      '';

  meta = {
    description = if gui then "BASR graphical agent workspace" else "Basic Agent Service in Rust";
    license = lib.licenses.mit;
    platforms = lib.platforms.linux;
    mainProgram = if gui then "basrw" else "basrctl";
  };
}
