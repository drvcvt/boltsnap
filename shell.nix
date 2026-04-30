# Fallback for users without flakes. Use the flake instead if you can:
#   nix develop
{ pkgs ? import <nixpkgs> { } }:

let
  runtimeLibs = with pkgs; [
    wayland
    libxkbcommon
    vulkan-loader
    libGL
    libdrm
    xorg.libX11
    xorg.libXcursor
    xorg.libXi
    xorg.libXrandr
    xorg.libxcb
    fontconfig
    freetype
  ];
in
pkgs.mkShell {
  packages = (with pkgs; [
    cargo
    rustc
    pkg-config
  ]) ++ runtimeLibs;

  LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath runtimeLibs;
}
