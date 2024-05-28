# docker run .#dockerImage && ./result | docker load
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
          inherit (pkgs) lib;
          rustToolchain = pkgs.pkgsBuildHost.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
          craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;

          # When filtering sources, we want to allow assets other than .rs files
          src = lib.cleanSourceWith {
            src = ./.; # The original, unfiltered source
            filter = path: type:
              (lib.hasSuffix "\.css" path) ||
              (lib.hasSuffix "\.ico" path) ||
              (lib.hasSuffix "\.txt" path) ||
              # Example of a folder for images, icons, etc
              # (lib.hasInfix "/assets/" path) ||
              # Default filter from crane (allow .rs files)
              (craneLib.filterCargoSources path type)
            ;
          };

          nativeBuildInputs = with pkgs; [ rustToolchain clang ];
          buildInputs = with pkgs; [ ];
          commonArgs = {
            inherit src buildInputs nativeBuildInputs;
            LIBCLANG_PATH = "${pkgs.libclang.lib}/lib"; # for rocksdb

            # link rocksdb dynamically
            ROCKSDB_INCLUDE_DIR = "${pkgs.rocksdb}/include";
            ROCKSDB_LIB_DIR = "${pkgs.rocksdb}/lib";
          };
          cargoArtifacts = craneLib.buildDepsOnly commonArgs;
          BITCOIND_EXE = pkgs.bitcoind + "/bin/bitcoind";
          bin = craneLib.buildPackage (commonArgs // {
            inherit cargoArtifacts BITCOIND_EXE;
            preCheck = ''
              export FBBE_EXE=./target/release/fbbe
            '';
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
              fbbe = bin;
            };
          devShells.default = mkShell {
            inputsFrom = [ bin ];

            LIBCLANG_PATH = "${pkgs.libclang.lib}/lib"; # for rocksdb

            # link rocksdb dynamically
            ROCKSDB_INCLUDE_DIR = "${pkgs.rocksdb}/include";
            ROCKSDB_LIB_DIR = "${pkgs.rocksdb}/lib";

            buildInputs = with pkgs; [ dive ];
          };
        }
      );
}
