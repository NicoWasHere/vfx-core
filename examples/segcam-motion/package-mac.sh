#!/usr/bin/env bash
# Builds a release effect and packages it as an installer:
#   target/vfx-plugin/SegcamMotionRust.pkg
# The installer puts SegcamMotionRust.plugin in Adobe's shared MediaCore folder, which every
# After Effects version scans.
#
# Signing is opt-in, as for wrappers/adobe/package-mac.sh:
#   SIGN_IDENTITY       "Developer ID Application: …"  signs the plugin (hardened runtime)
#   INSTALLER_IDENTITY  "Developer ID Installer: …"    signs the .pkg
# Without them the result is unsigned, which is fine for testing on this Mac.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
target="$here/../../target"
version="1.0.0"
install_location="/Library/Application Support/Adobe/Common/Plug-ins/7.0/MediaCore"

"$here/bundle-mac.sh" release
bundle="$target/vfx-plugin/SegcamMotionRust.plugin"

if [ -n "${SIGN_IDENTITY:-}" ]; then
  codesign --force --options runtime --timestamp --sign "$SIGN_IDENTITY" "$bundle"
  codesign --verify --strict --verbose=2 "$bundle"
fi

stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
# mktemp makes the folder 700, and the payload root's mode becomes the install folder's.
chmod 755 "$stage"
# No extended attributes, or the payload gains AppleDouble ._ files.
ditto --norsrc --noextattr --noqtn "$bundle" "$stage/SegcamMotionRust.plugin"

# Bundles are "relocatable" by default: the installer would follow an existing copy
# with the same bundle ID anywhere on disk. Always install to the MediaCore folder.
components="$stage.plist"
pkgbuild --analyze --root "$stage" "$components" >/dev/null
plutil -replace 0.BundleIsRelocatable -bool NO "$components"
trap 'rm -rf "$stage" "$components"' EXIT

sign_args=()
if [ -n "${INSTALLER_IDENTITY:-}" ]; then
  sign_args=(--sign "$INSTALLER_IDENTITY")
fi
pkg="$target/vfx-plugin/SegcamMotionRust.pkg"
COPYFILE_DISABLE=1 pkgbuild --root "$stage" \
  --component-plist "$components" \
  --install-location "$install_location" \
  --identifier com.vfx-core.segcam-motion \
  --version "$version" \
  "${sign_args[@]+"${sign_args[@]}"}" \
  "$pkg"

echo "$pkg"
