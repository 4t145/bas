{
  description = "Bas — Basic Agent Service";

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
    in
    {
      packages = eachLinux (system: {
        default = packageFor system;
        bas = packageFor system;
        basg = windowPackageFor system;
        system-test-driver = (testFor system).driverInteractive;
        system-test-shell = (shellFor system).driver;
      });
      checks = eachLinux (system: {
        package = packageFor system;
        basg = windowPackageFor system;
        system = testFor system;
      });
      apps = eachLinux (system: {
        bas = {
          type = "app";
          program = "${packageFor system}/bin/bas";
        };
        bast = {
          type = "app";
          program = "${packageFor system}/bin/bast";
        };
        basg = {
          type = "app";
          program = "${windowPackageFor system}/bin/basg";
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
              echo "bas dev shell: $(rustc --version), $(cargo --version)"
            '';
          };
        }
      );
    };
}
