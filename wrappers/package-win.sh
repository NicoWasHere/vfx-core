#!/usr/bin/env bash
# Builds the Windows plugins and packages them for a release:
#   target/vfx-plugin/VfxPreview-<version>-Windows-Setup.exe   installer (Inno Setup)
#   target/vfx-plugin/VfxPreview-<version>-Windows.zip         the same files, to copy by hand
# Usage: ./package-win.sh   (run from Git Bash)
#   VERSION   the release version, default "dev"
#   PLUGINS   which plugins to build and include, default "ofx ffgl adobe"
#   ISCC      Inno Setup's compiler, if it isn't on PATH or in its default folders
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
target="$here/../target"
out="$target/vfx-plugin"
version="${VERSION:-dev}"
plugins="${PLUGINS:-ofx ffgl adobe}"

iscc="${ISCC:-$(command -v iscc || true)}"
if [ -z "$iscc" ]; then
  for c in "/c/Program Files/Inno Setup "*/ISCC.exe "/c/Program Files (x86)/Inno Setup "*/ISCC.exe \
    "$(cygpath -u "${LOCALAPPDATA:-C:\\}")/Programs/Inno Setup "*/ISCC.exe; do
    [ -f "$c" ] && iscc="$c"
  done
fi
if [ -z "$iscc" ]; then
  echo "Inno Setup's ISCC.exe not found (install Inno Setup, or set ISCC)" >&2
  exit 2
fi

mkdir -p "$out"
rm -rf "$out/VfxPreview.ofx.bundle" "$out/VfxPreview.dll" "$out/VfxAdobe.aex"
files=()
for plugin in $plugins; do
  case "$plugin" in
    ofx) "$here/ofx/bundle-win.sh" release; files+=("VfxPreview.ofx.bundle") ;;
    ffgl) "$here/ffgl/bundle-win.sh" release; files+=("VfxPreview.dll") ;;
    adobe)
      cargo build --manifest-path "$here/adobe/Cargo.toml" --release
      cp "$target/release/vfx_adobe.dll" "$out/VfxAdobe.aex"
      files+=("VfxAdobe.aex") ;;
    *) echo "unknown plugin $plugin" >&2; exit 2 ;;
  esac
done

cat > "$out/INSTALL.txt" <<'TXT'
VFX Preview plugins for Windows (64-bit)

The Setup installer puts these where the apps look. To install by hand instead, copy:

- VfxPreview.ofx.bundle (DaVinci Resolve, Nuke, Natron) into
  C:\Program Files\Common Files\OFX\Plugins
- VfxPreview.dll (Resolume Avenue or Arena) into
  Documents\Resolume Avenue\Extra Effects  (or Resolume Arena\Extra Effects)
- VfxAdobe.aex (After Effects, Premiere Pro; not tested on Windows yet) into
  C:\Program Files\Adobe\Common\Plug-ins\7.0\MediaCore

Then restart the app.
TXT
files+=("INSTALL.txt")

name="VfxPreview-$version-Windows"
zip="$out/$name.zip"
rm -f "$zip"
win_out="$(cygpath -w "$out")"
paths=""
for f in "${files[@]}"; do paths+="'$win_out\\$f',"; done
powershell -NoProfile -Command "Compress-Archive -Path ${paths%,} -DestinationPath '$(cygpath -w "$zip")'"
echo "$zip"

"$iscc" -Q "-DVersion=$version" "-DSource=$(cygpath -w "$out")" "-O$(cygpath -w "$out")" \
  "-F$name-Setup" "$(cygpath -w "$here/installer-win.iss")"
echo "$out/$name-Setup.exe"
