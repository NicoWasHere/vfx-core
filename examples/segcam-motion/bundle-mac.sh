#!/usr/bin/env bash
# Builds the After Effects effect as a universal (arm64 + x86_64) macOS bundle:
#   target/vfx-plugin/SegcamMotionRust.plugin
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
  cargo build --manifest-path "$here/Cargo.toml" --lib $flag --target "${arch}-apple-darwin"
  dylibs+=("$target/${arch}-apple-darwin/$profile/libsegcam_motion.dylib")
  # The build script writes the PiPL resource into OUT_DIR. Take the newest copy.
  rsrc="$(ls -t "$target/${arch}-apple-darwin/$profile"/build/segcam-motion-*/out/SegcamMotion.rsrc | head -1)"
done

# Not SegcamMotion.plugin: that is segcam's own C++ effect, which can stay installed.
bundle="$target/vfx-plugin/SegcamMotionRust.plugin"
rm -rf "$bundle"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
lipo -create "${dylibs[@]}" -output "$bundle/Contents/MacOS/SegcamMotionRust"
cp "$rsrc" "$bundle/Contents/Resources/SegcamMotionRust.rsrc"
printf 'eFKTFXTC' > "$bundle/Contents/PkgInfo"

cat > "$bundle/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key><string>SegcamMotionRust</string>
  <key>CFBundleIdentifier</key><string>com.vfx-core.segcam-motion</string>
  <key>CFBundleName</key><string>segcam Motion (Rust)</string>
  <key>CFBundlePackageType</key><string>eFKT</string>
  <key>CFBundleSignature</key><string>FXTC</string>
  <key>CFBundleVersion</key><string>1.0.0</string>
  <key>CFBundleShortVersionString</key><string>1.0.0</string>
</dict>
</plist>
PLIST

echo "$bundle"
