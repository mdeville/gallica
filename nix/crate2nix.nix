# The TUI and GUI packages built with crate2nix: one derivation per crate,
# so dependencies are cached across builds. Cargo.nix is generated at
# evaluation time from Cargo.lock (import from derivation).
{
  pkgs,
  toolchain,
  crate2nix,
  src,
}:
let
  inherit (pkgs) lib;
  libs = import ./libs.nix { inherit pkgs; };
  metadata = import ./metadata.nix { inherit lib src; };

  # The workspace crates, by name, read from Cargo.toml so that listing them
  # needs no import from derivation.
  members = lib.listToAttrs (
    map (
      dir:
      let
        crate = metadata (src + "/${dir}");
      in
      lib.nameValuePair crate.name crate
    ) (lib.importTOML (src + "/Cargo.toml")).workspace.members
  );

  tools = pkgs.callPackage "${crate2nix}/tools.nix" { };
  cargoNix =
    pkgs.callPackage
      (tools.generatedCargoNix {
        name = "gallica";
        src = lib.cleanSource src;
        cargo = toolchain;
      })
      {
        buildRustCrateForPkgs =
          pkgs:
          pkgs.buildRustCrate.override {
            rustc = toolchain;
            cargo = toolchain;
          };
        # crate2nix leaves CARGO_PKG_DESCRIPTION empty, which clap uses for
        # `--help`.
        defaultCrateOverrides =
          pkgs.defaultCrateOverrides
          // lib.mapAttrs (_: crate: _: { inherit (crate.meta) description; }) members;
      };

  # Tests run from the crate's own sources only; the core's and the TUI's
  # integration tests also read the SDK's recorded Gallica responses.
  # testPreRun runs before each test binary.
  withSdkFixtures.testPreRun = ''
    if [ ! -e ../gallica/tests/fixtures ]; then
      mkdir -p ../gallica/tests
      cp -r ${src + "/crates/gallica/tests/fixtures"} ../gallica/tests/fixtures
    fi
  '';
  testOverrides = {
    gallica-core = withSdkFixtures;
    gallica-tui = withSdkFixtures;
  };

  buildCrate =
    name: overrides:
    cargoNix.workspaceMembers.${name}.build.overrideAttrs (
      old: { inherit (members.${name}) meta; } // overrides old
    );
in
{
  # Command line and terminal UI.
  gallica-tui = buildCrate "gallica-tui" (old: { });

  # Desktop GUI.
  gallica-gui = buildCrate "gallica-gui" (old: {
    postFixup = (old.postFixup or "") + libs.patchGuiRunpath "$out/bin/gallica-gui";
  });

  # Every workspace crate with its unit and integration tests run.
  tests = lib.mapAttrs (
    name: _:
    cargoNix.workspaceMembers.${name}.build.override (
      {
        runTests = true;
      }
      // testOverrides.${name} or { }
    )
  ) members;
}
