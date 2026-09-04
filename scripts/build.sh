#!/usr/bin/env bash
# Linux build helper for the death_http_bridge addon: mirrors scripts/build.ps1,
# running the Windows CSDK tools under Wine.
# Usage: build.sh <csdk_root> <project_root> <output_vpk>
set -euo pipefail

if [ "$#" -ne 3 ]; then
    echo "Usage: $0 <csdk_root> <project_root> <output_vpk>" >&2
    exit 2
fi

CSDK="$1"
PROJECT="$2"
OUTPUT="$3"
NAME="death_http_bridge"

# Match scripts/build.ps1: a relative output path is resolved against the project root.
case "$OUTPUT" in
    /*) ;;
    *) OUTPUT="$PROJECT/$OUTPUT" ;;
esac

COMPILER="$CSDK/game/bin_cs2/win64/resourcecompiler.exe"
PACKER="$CSDK/game/bin/win64/CSDKCfgVPK.exe"
SRC="$PROJECT/mod"
CONTENT="$CSDK/content/citadel_addons/$NAME"
GAME="$CSDK/game/citadel_addons/$NAME"

for f in "$COMPILER" "$PACKER"; do
    [ -f "$f" ] || { echo "CSDK tool not found: $f" >&2; exit 1; }
done
[ -d "$SRC" ] || { echo "Mod source folder not found: $SRC" >&2; exit 1; }
command -v wine >/dev/null || { echo "wine is not installed" >&2; exit 1; }

# Quiet Wine's fixme spam by default; override by exporting WINEDEBUG yourself.
export WINEDEBUG="${WINEDEBUG:--all}"
export DXVK_LOG_LEVEL="${DXVK_LOG_LEVEL:-none}"

# Windows path for a POSIX path (prefer winepath, fall back to Z: mapping).
winpath() {
    if command -v winepath >/dev/null; then
        winepath -w "$1"
    else
        printf '%s' "$1" | sed 's|^/|Z:\\|; s|/|\|g'
    fi
}

rm -rf "$CONTENT" "$GAME"
mkdir -p "$CONTENT" "$GAME" "$(dirname "$OUTPUT")"
cp -r "$SRC/." "$CONTENT/"

mapfile -t sources < <(find "$CONTENT" -type f \( -name '*.js' -o -name '*.xml' \) | sort)
[ "${#sources[@]}" -gt 0 ] || { echo "No Panorama sources found under $SRC" >&2; exit 1; }

for src in "${sources[@]}"; do
    echo "Compiling ${src#"$CONTENT"/}"
    wine "$(winpath "$COMPILER")" -i "$(winpath "$src")" -nop4
done

rm -f "$OUTPUT"
echo "Packing VPK to $OUTPUT"
wine "$(winpath "$PACKER")" "$(winpath "$GAME")" "$(winpath "$OUTPUT")"

[ -f "$OUTPUT" ] || { echo "VPK not created at $OUTPUT" >&2; exit 1; }
echo "Built $OUTPUT"
