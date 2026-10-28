#!/usr/bin/env bash
# Builds the FFGL plugin as a universal (arm64 + x86_64) macOS bundle:
#   target/vfx-plugin/VfxPreview.bundle
# Usage: ./bundle-mac.sh [debug|release] [--install]   (default: release)
# --install copies it into Resolume Avenue's and Arena's Extra Effects folders.
# Needs the rustup targets aarch64-apple-darwin and x86_64-apple-darwin.
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

binaries=()
for arch in aarch64 x86_64; do
  cargo build --manifest-path "$here/Cargo.toml" $flag --target "${arch}-apple-darwin"
  binaries+=("$target/${arch}-apple-darwin/$profile/libvfx_ffgl.dylib")
done

bundle="$target/vfx-plugin/VfxPreview.bundle"
rm -rf "$bundle"
mkdir -p "$bundle/Contents/MacOS"
lipo -create "${binaries[@]}" -output "$bundle/Contents/MacOS/VfxPreview"

cat > "$bundle/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key><string>VfxPreview</string>
  <key>CFBundleIdentifier</key><string>com.vfx-core.vfx-ffgl</string>
  <key>CFBundleName</key><string>VFX Preview</string>
  <key>CFBundlePackageType</key><string>BNDL</string>
  <key>CFBundleVersion</key><string>1.0.0</string>
  <key>CFBundleShortVersionString</key><string>1.0.0</string>
</dict>
</plist>
PLIST
echo "$bundle"

if [ "$install" = 1 ]; then
  for app in "Resolume Avenue" "Resolume Arena"; do
    dir="$HOME/Documents/$app/Extra Effects"
    if [ -d "$HOME/Documents/$app" ]; then
      mkdir -p "$dir"
      rm -rf "$dir/VfxPreview.bundle"
      ditto "$bundle" "$dir/VfxPreview.bundle"
      echo "installed in $dir"
    fi
  done
fi
