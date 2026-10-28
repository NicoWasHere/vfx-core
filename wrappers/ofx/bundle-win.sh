#!/usr/bin/env bash
# Builds the OpenFX plugin as a Windows (x64) bundle:
#   target/vfx-plugin/VfxPreview.ofx.bundle
# Usage: ./bundle-win.sh [debug|release] [--install]   (default: release)
# --install copies it into C:\Program Files\Common Files\OFX\Plugins, where DaVinci
# Resolve, Nuke, Natron and other OpenFX hosts on Windows look for plugins. Writing there
# needs a Git Bash started as administrator.
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
flag=""
[ "$profile" = "release" ] && flag="--release"

cargo build --manifest-path "$here/Cargo.toml" $flag

# Same layout as on macOS, with the DLL renamed .ofx under Contents/Win64.
bundle="$target/vfx-plugin/VfxPreview.ofx.bundle"
rm -rf "$bundle"
mkdir -p "$bundle/Contents/Win64"
cp "$target/$profile/vfx_ofx.dll" "$bundle/Contents/Win64/VfxPreview.ofx"
echo "$bundle"

if [ "$install" = 1 ]; then
  dir="$(cygpath -u "${COMMONPROGRAMFILES:-C:\\Program Files\\Common Files}")/OFX/Plugins"
  mkdir -p "$dir"
  rm -rf "$dir/VfxPreview.ofx.bundle"
  cp -r "$bundle" "$dir/"
  echo "installed in $dir"
fi
