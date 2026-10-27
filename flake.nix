{
  description = "intake — a CLI diet tracker";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
    }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];

      forAllSystems =
        f:
        nixpkgs.lib.genAttrs systems (
          system:
          f (
            import nixpkgs {
              inherit system;
              overlays = [ rust-overlay.overlays.default ];
            }
          )
        );

      rustFor = pkgs: pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;

      mkPackage =
        pkgs: noAi:
        let
          rust = rustFor pkgs;
          rustPlatform = pkgs.makeRustPlatform {
            cargo = rust;
            rustc = rust;
          };
          featureFlags = nixpkgs.lib.optionals noAi [ "--no-default-features" ];
        in
        rustPlatform.buildRustPackage {
          pname = if noAi then "intake-no-ai" else "intake";
          version = (nixpkgs.lib.importTOML ./Cargo.toml).package.version;
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;

          cargoBuildFlags = featureFlags;
          cargoTestFlags = [ "--workspace" ] ++ featureFlags;
          doCheck = true;

          meta.mainProgram = "intake";
        };
    in
    {
      devShells = forAllSystems (
        pkgs:
        let
          rust = rustFor pkgs;
        in
        {
          default = pkgs.mkShell {
            packages = [
              rust
              pkgs.stdenv.cc
              pkgs.pkg-config
            ];
            RUST_SRC_PATH = "${rust}/lib/rustlib/src/rust/library";
          };
        }
      );

      packages = forAllSystems (pkgs: {
        default = mkPackage pkgs false;
        no-ai = mkPackage pkgs true;
      });

      checks = forAllSystems (pkgs: {
        default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
        no-ai = self.packages.${pkgs.stdenv.hostPlatform.system}.no-ai;
      });
    };
}
