#!/usr/bin/env bash
# Builds the FFGL plugin as a Windows (x64) DLL:
#   target/vfx-plugin/<PLUGIN_FILE>.dll   (names from plugin.env)
# Usage: ./bundle-win.sh [debug|release] [--install]   (default: release)
# Run from Git Bash. --install copies into existing Resolume Extra Effects folders.
set -euo pipefail

profile="release"
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
# The plugin's names and IDs.
. "$here/../../plugin.env"
flag=""
[ "$profile" = "release" ] && flag="--release"

cargo build --manifest-path "$here/Cargo.toml" $flag
mkdir -p "$target/vfx-plugin"
bundle="$target/vfx-plugin/$PLUGIN_FILE.dll"
cp "$target/$profile/vfx_ffgl.dll" "$bundle"
echo "$bundle"

if [ "$install" = 1 ]; then
  for app in "Resolume Avenue" "Resolume Arena"; do
    dir="$HOME/Documents/$app/Extra Effects"
    if [ -d "$HOME/Documents/$app" ]; then
      mkdir -p "$dir"
      cp "$bundle" "$dir/$PLUGIN_FILE.dll"
      echo "installed in $dir"
    fi
  done
fi
