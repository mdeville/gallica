{
  description = "gallica — a bulk downloader for Gallica (BnF): SDK, CLI and GUI";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
        };
        # The toolchain is pinned by rust-toolchain.toml so cargo/rustup and
        # nix agree on the version.
        toolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
        rustPlatform = pkgs.makeRustPlatform {
          cargo = toolchain;
          rustc = toolchain;
        };
        nativeBuildInputs = [ pkgs.pkg-config ];
        buildInputs = pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [ ]
          ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isDarwin [ pkgs.libiconv ];
      in {
        packages.default = rustPlatform.buildRustPackage {
          pname = "gallica";
          version = "0.1.0";
          src = pkgs.lib.cleanSource ./.;
          cargoLock.lockFile = ./Cargo.lock;
          inherit nativeBuildInputs buildInputs;
          # Integration tests spin up a local mock server; fine in the sandbox.
          doCheck = true;
        };

        devShells.default = pkgs.mkShell {
          packages = [ toolchain pkgs.cargo-watch pkgs.cargo-nextest ] ++ nativeBuildInputs ++ buildInputs;
          RUST_SRC_PATH = "${toolchain}/lib/rustlib/src/rust/library";
          # Work environments sometimes export a shared target dir and linker
          # flags; this project builds on its own.
          shellHook = ''
            unset CARGO_TARGET_DIR CARGO_BUILD_TARGET_DIR RUSTFLAGS
            export NIX_CFLAGS_LINK="''${NIX_CFLAGS_LINK//-fuse-ld=mold/}"
          '';
        };

        formatter = pkgs.nixfmt-rfc-style;
      });
}
