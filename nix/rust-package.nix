# The TUI and GUI packages built with nixpkgs' buildRustPackage (one cargo
# invocation per package), for the platform `pkgs` targets: native, or a
# pkgsCross set. Used where crate2nix is not.
{
  pkgs,
  toolchain,
  src,
}:
let
  inherit (pkgs) lib;
  libs = import ./libs.nix { inherit pkgs; };
  metadata = import ./metadata.nix { inherit lib src; };
  rustPlatform = pkgs.makeRustPlatform {
    cargo = toolchain;
    rustc = toolchain;
  };

  # Both binaries share one source tree and lock file; each package builds
  # and tests only its own crate (and what it depends on).
  buildCrate =
    dir: args:
    let
      crate = metadata dir;
      cargoFlags = [
        "-p"
        crate.name
      ];
    in
    rustPlatform.buildRustPackage (
      {
        pname = crate.name;
        inherit (crate) version meta;
        src = lib.cleanSource src;
        cargoLock.lockFile = src + "/Cargo.lock";
        nativeBuildInputs = [ pkgs.buildPackages.pkg-config ];
        cargoBuildFlags = cargoFlags;
        cargoTestFlags = cargoFlags;
        # Integration tests spin up a local mock server; fine in the sandbox.
        # Skipped automatically when cross compiling.
        doCheck = true;
      }
      // args
    );
in
{
  # Command line and terminal UI.
  gallica-tui = buildCrate (src + "/crates/gallica-tui") {
    buildInputs = libs.darwin;
  };

  # Desktop GUI.
  gallica-gui = buildCrate (src + "/crates/gallica-gui") {
    buildInputs = libs.gui ++ libs.darwin;
    postFixup = libs.patchGuiRunpath "$out/bin/gallica-gui";
  };
}
