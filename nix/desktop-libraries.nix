{ pkgs }:
with pkgs;
[
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
