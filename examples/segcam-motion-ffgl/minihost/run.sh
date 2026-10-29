#!/usr/bin/env bash
# Build segcam and the shared FFGL mini host, then run the CPU effect checks.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$here/../../.."
target="$root/target"
case "$(uname -s)" in
  MINGW* | MSYS* | CYGWIN*)
    plugin="$target/release/segcam_motion_ffgl.dll"
    host="$target/ffgl-minihost/segcam.exe"
    libs=(-lopengl32 -lgdi32 -luser32) ;;
  *)
    plugin="$target/release/libsegcam_motion_ffgl.dylib"
    host="$target/ffgl-minihost/segcam"
    libs=(-framework OpenGL) ;;
esac
cargo build --manifest-path "$here/../Cargo.toml" --release --quiet
mkdir -p "$target/ffgl-minihost"
"${CC:-cc}" -std=c11 -Wall -Wextra -Wno-cast-function-type -O1 '-DPLUGIN_NAME="segcam Motion"' "$root/wrappers/ffgl/minihost/minihost.c" -o "$host" "${libs[@]}"
case_name="${1:-all}"
"$host" "$plugin" "$case_name"
if [ "$case_name" = all ] || [ "$case_name" = segcam ]; then
  cargo build --manifest-path "$root/examples/segcam-motion/Cargo.toml" --no-default-features --bin segcam-motion --release --quiet
  cli="$target/release/segcam-motion"
  [ -f "$cli.exe" ] && cli="$cli.exe"
  python3 "$here/compare.py" "$host" "$plugin" "$cli" "${@:2}"
fi
