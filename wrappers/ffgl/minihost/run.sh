#!/usr/bin/env bash
# Builds the plugin and the minimal FFGL host, then runs the host's checks.
# Usage: ./minihost/run.sh [case]     (default: all)
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
target="$here/../../../target"
cargo build --manifest-path "$here/../Cargo.toml" --release --quiet
mkdir -p "$target/ffgl-minihost"
cc -std=c11 -Wall -Wextra -O1 -framework OpenGL "$here/minihost.c" -o "$target/ffgl-minihost/minihost"
"$target/ffgl-minihost/minihost" "$target/release/libvfx_ffgl.dylib" "${1:-all}"
