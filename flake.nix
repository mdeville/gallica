{
  description = "gallica — a bulk downloader for Gallica (BnF): SDK, CLI and GUI";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
    crate2nix = {
      url = "github:nix-community/crate2nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
      flake-utils,
      crate2nix,
    }:
    # nixpkgs-unstable dropped x86_64-darwin in 26.11.
    flake-utils.lib.eachSystem
      [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ]
      (
        system:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ (import rust-overlay) ];
          };
          # The toolchain is pinned by rust-toolchain.toml so cargo/rustup and
          # nix agree on the version.
          toolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
          inherit (pkgs) lib;
          libs = import ./nix/libs.nix { inherit pkgs; };
          nativeBuildInputs = [ pkgs.pkg-config ];

          # Native builds go through crate2nix.
          native = import ./nix/crate2nix.nix {
            inherit pkgs toolchain crate2nix;
            src = ./.;
          };
          # Windows x86_64 (MinGW), cross compiled from Linux with a plain cargo
          # build.
          windows = import ./nix/rust-package.nix {
            pkgs = pkgs.pkgsCross.mingwW64;
            toolchain = toolchain.override { targets = [ "x86_64-pc-windows-gnu" ]; };
            src = ./.;
          };
        in
        {
          packages = {
            inherit (native) gallica-tui gallica-gui;
            default = native.gallica-tui;
          }
          // lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux {
            gallica-tui-windows = windows.gallica-tui;
            gallica-gui-windows = windows.gallica-gui;
          };

          checks = lib.mapAttrs' (name: lib.nameValuePair "test-${name}") native.tests;

          devShells.default = pkgs.mkShell {
            packages = [
              toolchain
              pkgs.cargo-watch
              pkgs.cargo-nextest
            ]
            ++ nativeBuildInputs
            ++ libs.gui
            ++ libs.darwin;
            RUST_SRC_PATH = "${toolchain}/lib/rustlib/src/rust/library";
            # winit and glow load these at run time.
            LD_LIBRARY_PATH = lib.makeLibraryPath libs.gui;
            # Work environments sometimes export a shared target dir and linker
            # flags; this project builds on its own.
            shellHook = ''
              unset CARGO_TARGET_DIR CARGO_BUILD_TARGET_DIR RUSTFLAGS
              export NIX_CFLAGS_LINK="''${NIX_CFLAGS_LINK//-fuse-ld=mold/}"
            '';
          };

          formatter = pkgs.nixfmt-tree;
        }
      );
}
