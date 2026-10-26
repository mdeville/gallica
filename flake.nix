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

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
      flake-utils,
    }:
    flake-utils.lib.eachDefaultSystem (
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
        workspace = (lib.importTOML ./Cargo.toml).workspace.package;

        # Libraries for a given package set: the GUI (eframe/winit) links
        # against the windowing and GL libraries on Linux.
        guiLibsFor =
          pkgs:
          lib.optionals pkgs.stdenv.hostPlatform.isLinux [
            pkgs.libxkbcommon
            pkgs.libGL
            pkgs.wayland
            pkgs.libx11
            pkgs.libxcursor
            pkgs.libxi
            pkgs.libxrandr
            pkgs.fontconfig
            pkgs.freetype
          ];
        darwinLibsFor = pkgs: lib.optionals pkgs.stdenv.hostPlatform.isDarwin [ pkgs.libiconv ];

        guiLibs = guiLibsFor pkgs;
        nativeBuildInputs = [ pkgs.pkg-config ];
        buildInputs = guiLibs ++ darwinLibsFor pkgs;

        # The TUI and GUI packages for the platform `pkgs` targets (native, or
        # a pkgsCross set), built with `toolchain`.
        cratePackages =
          pkgs: toolchain:
          let
            rustPlatform = pkgs.makeRustPlatform {
              cargo = toolchain;
              rustc = toolchain;
            };
            guiLibs = guiLibsFor pkgs;
            darwinLibs = darwinLibsFor pkgs;

            # Builds one binary crate of the workspace, taking its name,
            # version, description, license and binary name from its
            # Cargo.toml. Both binaries share one source tree and lock file;
            # each package builds and tests only its own crate (and what it
            # depends on).
            buildCrate =
              dir: args:
              let
                manifest = lib.importTOML (dir + "/Cargo.toml");
                pkg = manifest.package;
                field = name: if pkg.${name}.workspace or false then workspace.${name} else pkg.${name};
                cargoFlags = [
                  "-p"
                  pkg.name
                ];
              in
              rustPlatform.buildRustPackage (
                {
                  pname = pkg.name;
                  version = field "version";
                  src = lib.cleanSource ./.;
                  cargoLock.lockFile = ./Cargo.lock;
                  nativeBuildInputs = [ pkgs.buildPackages.pkg-config ];
                  cargoBuildFlags = cargoFlags;
                  cargoTestFlags = cargoFlags;
                  # Integration tests spin up a local mock server; fine in the
                  # sandbox. Skipped automatically when cross compiling.
                  doCheck = true;
                  meta = {
                    description = pkg.description;
                    license = map lib.getLicenseFromSpdxId (lib.splitString " OR " (field "license"));
                    mainProgram = (builtins.head manifest.bin).name;
                  };
                }
                // args
              );
          in
          {
            # Command line and terminal UI.
            gallica-tui = buildCrate ./crates/gallica-tui {
              buildInputs = darwinLibs;
            };

            # Desktop GUI.
            gallica-gui = buildCrate ./crates/gallica-gui {
              buildInputs = guiLibs ++ darwinLibs;
              # winit and glow dlopen the windowing and GL libraries at run
              # time, so they must be on the binary's RUNPATH.
              postFixup = lib.optionalString pkgs.stdenv.hostPlatform.isLinux ''
                patchelf --add-rpath ${lib.makeLibraryPath guiLibs} $out/bin/gallica-gui
              '';
            };
          };

        native = cratePackages pkgs toolchain;
        # Windows x86_64 (MinGW), cross compiled from Linux.
        windows = cratePackages pkgs.pkgsCross.mingwW64 (
          toolchain.override { targets = [ "x86_64-pc-windows-gnu" ]; }
        );
      in
      {
        packages =
          native
          // {
            default = native.gallica-tui;
          }
          // lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux {
            gallica-tui-windows = windows.gallica-tui;
            gallica-gui-windows = windows.gallica-gui;
          };

        devShells.default = pkgs.mkShell {
          packages = [
            toolchain
            pkgs.cargo-watch
            pkgs.cargo-nextest
          ]
          ++ nativeBuildInputs
          ++ buildInputs;
          RUST_SRC_PATH = "${toolchain}/lib/rustlib/src/rust/library";
          # winit and glow load these at run time.
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath guiLibs;
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
