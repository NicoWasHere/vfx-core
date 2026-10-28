#!/usr/bin/env bash
# Build the x64 DLL; --install copies it into existing Resolume Documents folders.
# Usage: ./bundle-win.sh [debug|release] [--install]
set -euo pipefail

profile=release
install=0
for arg in "$@"; do
  case "$arg" in
    debug|release) profile="$arg" ;;
    --install) install=1 ;;
    *) echo "unknown argument $arg" >&2; exit 2 ;;
  esac
done
here="$(cd "$(dirname "$0")" && pwd)"
target="$here/../../target"
flag=()
[ "$profile" = release ] && flag=(--release)
cargo build --manifest-path "$here/Cargo.toml" "${flag[@]}"
mkdir -p "$target/vfx-plugin"
bundle="$target/vfx-plugin/SegcamMotionRust.dll"
cp "$target/$profile/segcam_motion_ffgl.dll" "$bundle"
echo "$bundle"

if [ "$install" = 1 ]; then
  for app in "Resolume Avenue" "Resolume Arena"; do
    dir="$HOME/Documents/$app/Extra Effects"
    if [ -d "$HOME/Documents/$app" ]; then
      mkdir -p "$dir"
      cp "$bundle" "$dir/SegcamMotionRust.dll"
      echo "installed in $dir"
    fi
  done
fi
