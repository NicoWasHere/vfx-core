#!/usr/bin/env bash
# Build a universal FFGL bundle; --install copies it into existing Resolume folders.
# Usage: ./bundle-mac.sh [debug|release] [--install]
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
binaries=()
for arch in aarch64 x86_64; do
  cargo build --manifest-path "$here/Cargo.toml" "${flag[@]}" --target "${arch}-apple-darwin"
  binaries+=("$target/${arch}-apple-darwin/$profile/libsegcam_motion_ffgl.dylib")
done

bundle="$target/vfx-plugin/SegcamMotionRust.bundle"
mkdir -p "$bundle/Contents/MacOS"
lipo -create "${binaries[@]}" -output "$bundle/Contents/MacOS/SegcamMotionRust"
cat > "$bundle/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key><string>SegcamMotionRust</string>
  <key>CFBundleIdentifier</key><string>com.vfx-core.segcam-motion.ffgl</string>
  <key>CFBundleName</key><string>segcam Motion</string>
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
      ditto "$bundle" "$dir/SegcamMotionRust.bundle"
      echo "installed in $dir"
    fi
  done
fi
