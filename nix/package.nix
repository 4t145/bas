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
    name = "org.bas.Gui";
    desktopName = "BasG";
    genericName = "Agent Workspace";
    comment = "Manage Bas sessions and chat with agents";
    exec = "basg";
    icon = "org.bas.Gui";
    terminal = false;
    categories = [
      "Development"
      "Utility"
    ];
    startupWMClass = "org.bas.Gui";
  };
in
rustPlatform.buildRustPackage {
  pname = if gui then "basg" else "bas";
  version = (builtins.fromTOML (builtins.readFile ../Cargo.toml)).package.version;

  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../build.rs
      ../src
      ../proto
      ../assets
      (lib.fileset.fileFilter (file: file.name != ".git") ../vendor/gds-uim)
      ../tests
    ];
  };
  cargoLock.lockFile = ../Cargo.lock;
  buildFeatures = lib.optionals gui [ "gui" ];
  cargoBuildFlags = lib.optionals gui [
    "--bin"
    "basg"
  ];
  cargoTestFlags =
    if gui then
      [
        "--bin"
        "basg"
      ]
    else
      [ "--all-targets" ];
  nativeBuildInputs = [ makeWrapper ] ++ lib.optionals gui [
    pkg-config
  ];
  buildInputs = lib.optionals gui desktopLibraries;
  # GUI process-tree tests inspect orphaned children with ps.
  nativeCheckInputs = lib.optionals gui [ pkgs.procps ];
  postInstall =
    if gui then
      ''
        wrapProgram "$out/bin/basg" \
          --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath desktopLibraries}
        install -Dm644 ${desktopItem}/share/applications/org.bas.Gui.desktop \
          "$out/share/applications/org.bas.Gui.desktop"
        install -Dm644 assets/branding/ferris-dimensional.svg \
          "$out/share/icons/hicolor/scalable/apps/org.bas.Gui.svg"
        install -Dm644 assets/branding/ferris-dimensional-32.svg \
          "$out/share/icons/hicolor/32x32/apps/org.bas.Gui.svg"
        # Nix profile icons may be absent from a running launcher's theme index.
        # Desktop Entry permits absolute Icon paths; bind both icon and executable
        # to this immutable installation instead of relying on session caches/PATH.
        substituteInPlace "$out/share/applications/org.bas.Gui.desktop" \
          --replace-fail "Icon=org.bas.Gui" "Icon=$out/share/icons/hicolor/scalable/apps/org.bas.Gui.svg" \
          --replace-fail "Exec=basg" "Exec=$out/bin/basg"
      ''
    else
      ''
        wrapProgram "$out/bin/bas" \
          --set-default BAS_ACP_PATCHELF ${pkgs.patchelf}/bin/patchelf \
          --set-default BAS_ACP_ELF_LOADER ${pkgs.stdenv.cc.bintools.dynamicLinker} \
          --set-default BAS_ACP_ELF_LIBRARY_PATH ${lib.makeLibraryPath [ pkgs.stdenv.cc.cc.lib pkgs.glibc pkgs.zlib pkgs.openssl ]}
        install -Dm644 ${../contrib/bas-user.service} "$out/share/systemd/user/bas.service"
        substituteInPlace "$out/share/systemd/user/bas.service" \
          --replace-fail '@bas@' "$out/bin/bas"
        install -Dm644 ${../contrib/settings.json} "$out/share/bas/settings.example.json"
      '';

  meta = {
    description = if gui then "BasG graphical agent workspace" else "Basic Agent Service";
    license = lib.licenses.mit;
    platforms = lib.platforms.linux;
    mainProgram = if gui then "basg" else "bast";
  };
}
