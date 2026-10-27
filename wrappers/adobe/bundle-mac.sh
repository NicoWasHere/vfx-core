#!/usr/bin/env bash
# Builds the After Effects plugin as a universal (arm64 + x86_64) macOS bundle:
#   target/vfx-plugin/VfxAdobe.plugin
# Usage: ./bundle-mac.sh [debug|release]   (default: release)
# Needs the rustup targets aarch64-apple-darwin and x86_64-apple-darwin.
set -euo pipefail

profile="${1:-release}"
here="$(cd "$(dirname "$0")" && pwd)"
target="$here/../../target"
flag=""
[ "$profile" = "release" ] && flag="--release"

dylibs=()
rsrc=""
for arch in aarch64 x86_64; do
  cargo build --manifest-path "$here/Cargo.toml" $flag --target "${arch}-apple-darwin"
  dylibs+=("$target/${arch}-apple-darwin/$profile/libvfx_adobe.dylib")
  # The build script writes the PiPL resource into OUT_DIR. Take the newest copy.
  rsrc="$(ls -t "$target/${arch}-apple-darwin/$profile"/build/vfx-adobe-*/out/VfxAdobe.rsrc | head -1)"
done

bundle="$target/vfx-plugin/VfxAdobe.plugin"
rm -rf "$bundle"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
lipo -create "${dylibs[@]}" -output "$bundle/Contents/MacOS/VfxAdobe"
cp "$rsrc" "$bundle/Contents/Resources/VfxAdobe.rsrc"
printf 'eFKTFXTC' > "$bundle/Contents/PkgInfo"

cat > "$bundle/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key><string>VfxAdobe</string>
  <key>CFBundleIdentifier</key><string>com.vfx-core.vfx-adobe</string>
  <key>CFBundleName</key><string>VFX Preview</string>
  <key>CFBundlePackageType</key><string>eFKT</string>
  <key>CFBundleSignature</key><string>FXTC</string>
  <key>CFBundleVersion</key><string>1.1.0</string>
  <key>CFBundleShortVersionString</key><string>1.1.0</string>
</dict>
</plist>
PLIST

echo "$bundle"
