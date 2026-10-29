#!/usr/bin/env bash
# DLL against the CLI, using the Preview mini host's suites.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$here/../../.."
: "${SEGCAM_TEST_DIR:?set SEGCAM_TEST_DIR to a scratch directory}"
mkdir -p "$SEGCAM_TEST_DIR"
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) plugin="$root/target/release/segcam_motion_ofx.dll"; host="$SEGCAM_TEST_DIR/host.exe"; cli="$root/target/release/segcam-motion.exe" ;;
  *) plugin="$root/target/release/libsegcam_motion_ofx.dylib"; host="$SEGCAM_TEST_DIR/host"; cli="$root/target/release/segcam-motion" ;;
esac
cargo build --release -p segcam-motion-ofx -p segcam-motion --no-default-features
"${CC:-cc}" -std=c11 -Wno-incompatible-pointer-types -I"${OFX_INCLUDE:?set OFX_INCLUDE}" "$here/host.c" -o "$host"
python "$here/compare.py" "$host" "$plugin" "$cli" "$SEGCAM_TEST_DIR"
