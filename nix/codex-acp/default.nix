{
  lib,
  stdenv,
  buildNpmPackage,
  nodejs,
  makeWrapper,
  autoPatchelfHook,
  ncurses,
}:
buildNpmPackage {
  pname = "basr-codex-acp";
  version = "1.12.0";
  src = lib.fileset.toSource {
    root = ./.;
    fileset = lib.fileset.unions [
      ./package.json
      ./package-lock.json
    ];
  };
  npmDepsHash = "sha256-9if7EhqgvZ5OQqZX1J1ew6cMaM8u6/g+LdmdKI7U+7Q=";
  dontNpmBuild = true;
  npmRebuildFlags = [ "--ignore-scripts" ];
  nativeBuildInputs = [
    makeWrapper
    autoPatchelfHook
  ];
  buildInputs = [
    stdenv.cc.cc.lib
    ncurses
  ];
  installPhase = ''
    runHook preInstall
    mkdir -p "$out/lib/codex-acp"
    cp -r node_modules "$out/lib/codex-acp/"
    makeWrapper ${nodejs}/bin/node "$out/bin/codex-acp" \
      --add-flags "$out/lib/codex-acp/node_modules/@agentclientprotocol/codex-acp/dist/index.js" \
      --prefix PATH : "$out/lib/codex-acp/node_modules/.bin:${lib.makeBinPath [ nodejs ]}"
    runHook postInstall
  '';
  meta = {
    description = "ACP adapter for Codex with a compatible bundled Codex CLI";
    homepage = "https://github.com/agentclientprotocol/codex-acp";
    license = lib.licenses.asl20;
    platforms = lib.platforms.linux;
    mainProgram = "codex-acp";
  };
}
