#!/usr/bin/env bash
# Builds a release OpenFX plugin and packages it as an installer:
#   target/vfx-plugin/<PLUGIN_FILE>-ofx.pkg   (names from plugin.env)
# The installer puts <PLUGIN_FILE>.ofx.bundle in /Library/OFX/Plugins, where DaVinci
# Resolve, Nuke, Natron and other OpenFX hosts on macOS look for plugins.
#
# Signing and notarization are opt-in, through environment variables:
#   SIGN_IDENTITY       "Developer ID Application: …"  signs the plugin (hardened runtime)
#   NOTARY_PROFILE      notarytool keychain profile    notarizes and staples the plugin
#                                                      (needs SIGN_IDENTITY)
#   INSTALLER_IDENTITY  "Developer ID Installer: …"    signs the .pkg
# Without them the result is unsigned, which is fine for testing on this Mac.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
target="$here/../../target"
# The plugin's names and IDs.
. "$here/../../plugin.env"
version="1.0.0"
install_location="/Library/OFX/Plugins"

"$here/bundle-mac.sh" release
bundle="$target/vfx-plugin/$PLUGIN_FILE.ofx.bundle"

if [ -n "${SIGN_IDENTITY:-}" ]; then
  codesign --force --options runtime --timestamp --sign "$SIGN_IDENTITY" "$bundle"
  codesign --verify --strict --verbose=2 "$bundle"
fi

if [ -n "${NOTARY_PROFILE:-}" ]; then
  if [ -z "${SIGN_IDENTITY:-}" ]; then
    echo "NOTARY_PROFILE needs SIGN_IDENTITY: Apple only notarizes signed code." >&2
    exit 1
  fi
  zip="$target/vfx-plugin/$PLUGIN_FILE-ofx-notarize.zip"
  ditto -c -k --keepParent "$bundle" "$zip"
  xcrun notarytool submit "$zip" --keychain-profile "$NOTARY_PROFILE" --wait
  xcrun stapler staple "$bundle"
  rm -f "$zip"
fi

stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
# mktemp makes the folder 700, and the payload root's mode becomes the install folder's.
chmod 755 "$stage"
# No extended attributes, or the payload gains AppleDouble ._ files.
ditto --norsrc --noextattr --noqtn "$bundle" "$stage/$PLUGIN_FILE.ofx.bundle"

# Bundles are "relocatable" by default: the installer would follow an existing copy
# with the same bundle ID anywhere on disk. Always install to the OFX folder.
components="$stage.plist"
pkgbuild --analyze --root "$stage" "$components" >/dev/null
plutil -replace 0.BundleIsRelocatable -bool NO "$components"
trap 'rm -rf "$stage" "$components"' EXIT

sign_args=()
if [ -n "${INSTALLER_IDENTITY:-}" ]; then
  sign_args=(--sign "$INSTALLER_IDENTITY")
fi
pkg="$target/vfx-plugin/$PLUGIN_FILE-ofx.pkg"
COPYFILE_DISABLE=1 pkgbuild --root "$stage" \
  --component-plist "$components" \
  --install-location "$install_location" \
  --identifier "$PLUGIN_ID.ofx" \
  --version "$version" \
  "${sign_args[@]+"${sign_args[@]}"}" \
  "$pkg"

echo "$pkg"
