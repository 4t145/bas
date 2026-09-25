# Pinned fixture for local development and VM tests, not a BASR product package.
{
  lib,
  buildNpmPackage,
  nodejs,
  makeWrapper,
  pi-coding-agent,
  bash,
  coreutils,
  findutils,
  gnugrep,
  gnused,
  git,
}:
buildNpmPackage {
  pname = "pi-acp";
  version = "0.0.32";
  src = lib.fileset.toSource {
    root = ./.;
    fileset = lib.fileset.unions [
      ./package.json
      ./package-lock.json
    ];
  };
  npmDepsHash = "sha256-RCDbMrUiX9GttG9z6bPtLPz3Tx4G0t34MD9D2Wg2LpI=";
  dontNpmBuild = true;
  npmRebuildFlags = [ "--ignore-scripts" ];
  nativeBuildInputs = [ makeWrapper ];

  installPhase = ''
    runHook preInstall
    mkdir -p "$out/lib/pi-acp"
    cp -r node_modules "$out/lib/pi-acp/"
    makeWrapper ${nodejs}/bin/node "$out/bin/pi-acp" \
      --add-flags "$out/lib/pi-acp/node_modules/pi-acp/dist/index.js" \
      --set PI_ACP_PI_COMMAND ${lib.getExe pi-coding-agent} \
      --prefix PATH : ${
        lib.makeBinPath [
          bash
          coreutils
          findutils
          gnugrep
          gnused
          git
        ]
      }
    runHook postInstall
  '';

  meta = {
    description = "ACP adapter for the pi coding agent";
    homepage = "https://github.com/svkozak/pi-acp";
    license = lib.licenses.mit;
    mainProgram = "pi-acp";
    platforms = lib.platforms.linux;
  };
}
