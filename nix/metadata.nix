# Package metadata of a workspace crate, read from its Cargo.toml.
{ lib, src }:
dir:
let
  workspace = (lib.importTOML (src + "/Cargo.toml")).workspace.package;
  manifest = lib.importTOML (dir + "/Cargo.toml");
  pkg = manifest.package;
  field = name: if pkg.${name}.workspace or false then workspace.${name} else pkg.${name};
in
{
  name = pkg.name;
  version = field "version";
  meta = {
    description = pkg.description;
    license = map lib.getLicenseFromSpdxId (lib.splitString " OR " (field "license"));
  }
  // lib.optionalAttrs (manifest ? bin) {
    mainProgram = (builtins.head manifest.bin).name;
  };
}
