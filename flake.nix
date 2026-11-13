{
  description = "ggreddit: anonymous reddit reader";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";

    crane.url = "github:ipetkov/crane";

    flake-utils.url = "github:numtide/flake-utils";

    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, crane, flake-utils, rust-overlay, ... }:
    flake-utils.lib.eachSystem [ "x86_64-linux" ] (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
        };

        inherit (pkgs) lib;

        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          targets = [ "x86_64-unknown-linux-musl" ];
        };

        craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;


        src = lib.cleanSourceWith {
          src = craneLib.path ./.;
          filter = path: type:
            craneLib.filterCargoSources path type
            || lib.any (extension: lib.hasSuffix extension path) [ ".html" ".css" ".js" ];
        };

        package = { cargoExtraArgs }: craneLib.buildPackage {
          inherit src;
          pname = "ggreddit";
          strictDeps = true;
          doCheck = true;
          checkPhaseCargoCommand = "cargo test --locked --workspace";

          inherit cargoExtraArgs;
          CARGO_BUILD_TARGET = "x86_64-unknown-linux-musl";
          CARGO_BUILD_RUSTFLAGS = "-C target-feature=+crt-static";
        };

        ggreddit = package {
          cargoExtraArgs = "-p ggreddit";
        };

        release = pkgs.runCommandNoCC "ggreddit-release-0.1.0" {
          src = pkgs.fetchurl {
            url = "https://github.com/rishiad/ggreddit/releases/download/v0.1.0/ggreddit-x86_64-linux-musl.tar.gz";
            hash = "sha256-hepsG0QKz/c+5T8MKJXGyYprT+ICkUBbthQkX1jw7VE=";
          };
          nativeBuildInputs = [ pkgs.gnutar pkgs.gzip ];
        } ''
          mkdir -p "$out/bin"
          tar -xzf "$src" -C "$out/bin" ggreddit
        '';
      in
      {
        checks = {
          inherit ggreddit;
        };

        packages = {
          default = ggreddit;
          inherit ggreddit release;
        };
      }) // {
        nixosModules.default = import ./nix/module.nix { inherit self; };
        nixosModules.ggreddit = self.nixosModules.default;
      };
}
