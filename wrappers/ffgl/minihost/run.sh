#!/usr/bin/env bash
# Builds the plugin and the minimal FFGL host, then runs the host's checks.
# Usage: ./minihost/run.sh [case]     (default: all)
# Runs on macOS, and on Windows from Git Bash. CC picks the C compiler (default cc).
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
target="$here/../../../target"
# The plugin's name, which the host checks.
. "$here/../../../plugin.env"
case "$(uname -s)" in
  MINGW* | MSYS* | CYGWIN*)
    plugin="$target/release/vfx_ffgl.dll" host="$target/ffgl-minihost/minihost.exe"
    libs=(-lopengl32 -lgdi32 -luser32) ;;
  *)
    plugin="$target/release/libvfx_ffgl.dylib" host="$target/ffgl-minihost/minihost"
    libs=(-framework OpenGL) ;;
esac
cargo build --manifest-path "$here/../Cargo.toml" --release --quiet
mkdir -p "$target/ffgl-minihost"
"${CC:-cc}" -std=c11 -Wall -Wextra -Wno-cast-function-type -O1 "-DPLUGIN_NAME=\"$PLUGIN_NAME\"" "$here/minihost.c" -o "$host" "${libs[@]}"
"$host" "$plugin" "${1:-all}"
