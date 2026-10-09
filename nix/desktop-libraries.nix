{ pkgs }:
with pkgs;
lib.optionals stdenv.isLinux [
  fontconfig
  freetype
  libxkbcommon
  wayland
  vulkan-loader
  libGL
  libx11
  libxcb
  stdenv.cc.cc.lib
]
