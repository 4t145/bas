{
  pkgs,
  binary,
}:
let
  inherit (pkgs) lib;
  desktopLibraries = import ./desktop-libraries.nix { inherit pkgs; };
  desktopItem = pkgs.makeDesktopItem {
    name = "org.bas.Gui.Dev";
    desktopName = "BasG (Development)";
    extraConfig."Name[zh_CN]" = "BasG（开发版）";
    genericName = "Agent Workspace";
    comment = "Locally compiled development build of BasG";
    exec = "basg-dev";
    icon = "org.bas.Gui.Dev";
    terminal = false;
    categories = [
      "Development"
      "Utility"
    ];
    startupWMClass = "org.bas.Gui";
  };
in
# Compilation happens in the checkout, not this sandbox. Keep only the compiled
# artifact and runtime assets as inputs, so source changes do not rebuild dependencies.
pkgs.runCommand "basg-dev" { nativeBuildInputs = [ pkgs.makeWrapper ]; } ''
  install -Dm755 ${binary} "$out/libexec/basg"
  makeWrapper "$out/libexec/basg" "$out/bin/basg-dev" \
    --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath desktopLibraries}
  install -Dm644 ${desktopItem}/share/applications/org.bas.Gui.Dev.desktop \
    "$out/share/applications/org.bas.Gui.Dev.desktop"
  install -Dm644 ${../assets/branding/ferris-dimensional.svg} \
    "$out/share/icons/hicolor/scalable/apps/org.bas.Gui.Dev.svg"
  substituteInPlace "$out/share/applications/org.bas.Gui.Dev.desktop" \
    --replace-fail "Exec=basg-dev" "Exec=$out/bin/basg-dev" \
    --replace-fail "Icon=org.bas.Gui.Dev" \
      "Icon=$out/share/icons/hicolor/scalable/apps/org.bas.Gui.Dev.svg"
''
