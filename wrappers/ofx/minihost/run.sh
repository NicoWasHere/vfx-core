#!/usr/bin/env bash
# Builds the plugin and the minimal OpenFX host, then runs the host's checks.
# Usage: ./minihost/run.sh [case]     (default: all)
# Needs the OpenFX 1.4 headers; DaVinci Resolve installs them. Set OFX_INCLUDE to use others.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
target="$here/../../../target"
include="${OFX_INCLUDE:-/Library/Application Support/Blackmagic Design/DaVinci Resolve/Developer/OpenFX/OpenFX-1.4/include}"
if [ ! -f "$include/ofxCore.h" ]; then
  echo "OpenFX headers not found in $include (set OFX_INCLUDE)" >&2
  exit 2
fi

cargo build --manifest-path "$here/../Cargo.toml" --quiet
mkdir -p "$target/ofx-minihost"
cc -std=c11 -Wall -Wextra -Wno-unused-parameter -Wno-cast-function-type -O1 \
  -I"$include" "$here/minihost.c" -o "$target/ofx-minihost/minihost"
"$target/ofx-minihost/minihost" "$target/debug/libvfx_ofx.dylib" "${1:-all}"
