{
  description = "BASR — Basic Agent Service in Rust";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      nixpkgs,
      rust-overlay,
      ...
    }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];

      eachSystem = nixpkgs.lib.genAttrs systems;
      eachLinux = nixpkgs.lib.genAttrs [
        "x86_64-linux"
        "aarch64-linux"
      ];
      pkgsFor =
        system:
        import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };
      toolchainFor = pkgs: pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
      packageFor =
        system:
        let
          pkgs = pkgsFor system;
          toolchain = toolchainFor pkgs;
        in
        pkgs.callPackage ./nix/package.nix {
          rustPlatform = pkgs.makeRustPlatform {
            cargo = toolchain;
            rustc = toolchain;
          };
        };
      windowPackageFor = system: (packageFor system).override { gui = true; };
      testFor =
        system:
        nixpkgs.lib.nixos.runTest (
          import ./nix/tests/system.nix {
            pkgs = pkgsFor system;
            package = packageFor system;
          }
        );
      shellFor =
        system:
        (testFor system).extend {
          modules = [ (import ./nix/tests/shell.nix { pkgs = pkgsFor system; }) ];
        };
      piTestFor = system: (testFor system).extend { modules = [ ./nix/tests/pi.nix ]; };
    in
    {
      packages = eachLinux (system: {
        default = packageFor system;
        basr = packageFor system;
        basrd = packageFor system;
        basrw = windowPackageFor system;
        system-test-driver = (testFor system).driverInteractive;
        system-test-shell = (shellFor system).driver;
        system-test-pi = (piTestFor system).driver;
        pi-acp = (pkgsFor system).callPackage ./nix/pi-acp { };
        codex-acp = (pkgsFor system).callPackage ./nix/codex-acp { };
      });
      checks = eachLinux (system: {
        package = packageFor system;
        basrw = windowPackageFor system;
        system = testFor system;
      });
      apps = eachLinux (system: {
        basrd = {
          type = "app";
          program = "${packageFor system}/bin/basrd";
        };
        basrw = {
          type = "app";
          program = "${windowPackageFor system}/bin/basrw";
        };
        system-test = {
          type = "app";
          program = "${(testFor system).driverInteractive}/bin/nixos-test-driver";
        };
      });
      formatter = eachSystem (system: (pkgsFor system).nixfmt);
      devShells = eachSystem (
        system:
        let
          pkgs = pkgsFor system;

          toolchain = toolchainFor pkgs;
          desktopLibraries = import ./nix/desktop-libraries.nix { inherit pkgs; };
          guiShell = pkgs.mkShell {
            packages = [
              toolchain
              pkgs.pkg-config
            ];
            buildInputs = desktopLibraries;
            LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath desktopLibraries;
            RUST_SRC_PATH = "${toolchain}/lib/rustlib/src/rust/library";
          };
        in
        {
          gui = guiShell;
          local = guiShell.overrideAttrs (old: {
            nativeBuildInputs = old.nativeBuildInputs ++ [
              (pkgs.callPackage ./nix/pi-acp { })
              (pkgs.python3.withPackages (ps: [ ps.python-dotenv ]))
            ];
          });
          default = pkgs.mkShell {
            packages = [
              toolchain
              pkgs.direnv
            ];

            env = {
              # rust-analyzer resolves std sources from here when `rust-src` is installed.
              RUST_SRC_PATH = "${toolchain}/lib/rustlib/src/rust/library";
            };

            shellHook = ''
              echo "basr dev shell: $(rustc --version), $(cargo --version)"
            '';
          };
        }
      );
    };
}
