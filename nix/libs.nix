# System libraries the binaries need, for the platform `pkgs` targets.
{ pkgs }:
let
  inherit (pkgs) lib;
in
rec {
  # The GUI (eframe/winit) links against, and dlopens, the windowing and GL
  # libraries on Linux.
  gui = lib.optionals pkgs.stdenv.hostPlatform.isLinux [
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
  darwin = lib.optionals pkgs.stdenv.hostPlatform.isDarwin [ pkgs.libiconv ];

  # Puts the GUI libraries on a Linux binary's RUNPATH, since winit and glow
  # dlopen them at run time.
  patchGuiRunpath =
    bin:
    lib.optionalString pkgs.stdenv.hostPlatform.isLinux ''
      patchelf --add-rpath ${lib.makeLibraryPath gui} ${bin}
    '';
}
