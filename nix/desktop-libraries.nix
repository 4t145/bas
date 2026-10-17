{ pkgs }:
with pkgs;
lib.optionals stdenv.isLinux [
  alsa-lib
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
