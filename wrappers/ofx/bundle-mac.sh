#!/usr/bin/env bash
# Builds the OpenFX plugin as a universal (arm64 + x86_64) macOS bundle:
#   target/vfx-plugin/<PLUGIN_FILE>.ofx.bundle   (names from plugin.env)
# Usage: ./bundle-mac.sh [debug|release]   (default: release)
# Needs the rustup targets aarch64-apple-darwin and x86_64-apple-darwin.
set -euo pipefail

profile="${1:-release}"
here="$(cd "$(dirname "$0")" && pwd)"
target="$here/../../target"
# The plugin's names and IDs.
. "$here/../../plugin.env"
flag=""
[ "$profile" = "release" ] && flag="--release"

binaries=()
for arch in aarch64 x86_64; do
  cargo build --manifest-path "$here/Cargo.toml" $flag --target "${arch}-apple-darwin"
  binaries+=("$target/${arch}-apple-darwin/$profile/libvfx_ofx.dylib")
done

bundle="$target/vfx-plugin/$PLUGIN_FILE.ofx.bundle"
rm -rf "$bundle"
mkdir -p "$bundle/Contents/MacOS"
lipo -create "${binaries[@]}" -output "$bundle/Contents/MacOS/$PLUGIN_FILE.ofx"

cat > "$bundle/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key><string>$PLUGIN_FILE.ofx</string>
  <key>CFBundleIdentifier</key><string>$PLUGIN_ID</string>
  <key>CFBundleName</key><string>$PLUGIN_NAME</string>
  <key>CFBundlePackageType</key><string>BNDL</string>
  <key>CFBundleVersion</key><string>1.0.0</string>
  <key>CFBundleShortVersionString</key><string>1.0.0</string>
</dict>
</plist>
PLIST

echo "$bundle"
