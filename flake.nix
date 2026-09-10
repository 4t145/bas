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
    in
    {
      devShells = eachSystem (
        system:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ rust-overlay.overlays.default ];
          };

          toolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
        in
        {
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
