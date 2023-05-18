{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs = {
        nixpkgs.follows = "nixpkgs";
        flake-utils.follows = "flake-utils";
      };
    };
    crane = {
      url = "github:ipetkov/crane";
      inputs = {
        nixpkgs.follows = "nixpkgs";
        rust-overlay.follows = "rust-overlay";
        flake-utils.follows = "flake-utils";
      };
    };
  };
  outputs = { self, nixpkgs, flake-utils, rust-overlay, crane }:
    flake-utils.lib.eachDefaultSystem
      (system:
        let
          overlays = [ (import rust-overlay) ];
          pkgs = import nixpkgs {
            inherit system overlays;
          };
          rustToolchain = pkgs.pkgsBuildHost.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
          craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;
          
          src = craneLib.cleanCargoSource ./.;
          
          nativeBuildInputs = with pkgs; [ rustToolchain ];
          buildInputs = with pkgs; [ ];
          commonArgs = {
            inherit src buildInputs nativeBuildInputs;
          };
          cargoArtifacts = craneLib.buildDepsOnly commonArgs;
          bin = craneLib.buildPackage (commonArgs // {
            inherit cargoArtifacts;
          });
          dockerImage = pkgs.dockerTools.streamLayeredImage {
            name = "xenoky/fbbe";
            tag = "latest";
            contents = [ bin ];
            config = {
              Cmd = [ "${bin}/bin/fbbe" ];
            };
          };
        in
        with pkgs;
        {
          packages =
            {
              inherit bin dockerImage;
              default = bin;
            };
          devShells.default = mkShell {
            inputsFrom = [ bin ];
            buildInputs = with pkgs; [ dive ];
          };
        }
      );
}
