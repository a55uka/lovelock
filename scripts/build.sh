#!/usr/bin/env bash
# Linux build helper for the death_http_bridge addon: mirrors scripts/build.ps1,
# running the Windows CSDK tools under Proton (via protontricks-launch).
# Plain Wine cannot create the D3D11 device resourcecompiler.exe needs, so
# Proton is the default; set USE_WINE=1 to force plain Wine (packer-only use).
# Usage: build.sh <csdk_root> <project_root> <output_vpk>
# Env: PROTON_APPID (Steam app whose prefix Proton runs in; default 4217940),
#      USE_WINE=1 to use wine instead of Proton.
set -euo pipefail

if [ "$#" -ne 3 ]; then
    echo "Usage: $0 <csdk_root> <project_root> <output_vpk>" >&2
    exit 2
fi

CSDK="$1"
PROJECT="$2"
OUTPUT="$3"
NAME="death_http_bridge"
APPID="${PROTON_APPID:-4217940}"

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

if [ -n "${USE_WINE:-}" ]; then
    RUNNER="wine"
elif command -v protontricks-launch >/dev/null; then
    RUNNER="proton"
elif command -v wine >/dev/null; then
    echo "protontricks-launch not found, falling back to wine (compile will likely fail)" >&2
    RUNNER="wine"
else
    echo "neither protontricks-launch nor wine is installed" >&2
    exit 1
fi

# Quiet Wine fixme spam and DXVK info logs by default; override by exporting
# WINEDEBUG / DXVK_LOG_LEVEL yourself.
export WINEDEBUG="${WINEDEBUG:--all}"
export DXVK_LOG_LEVEL="${DXVK_LOG_LEVEL:-none}"

# Windows path for a POSIX path. The Proton prefix maps z: -> /, so the
# Z: fallback is correct there; under Wine prefer winepath when present.
winpath() {
    if [ "$RUNNER" = "wine" ] && command -v winepath >/dev/null; then
        winepath -w "$1"
    else
        printf '%s' "$1" | sed 's|^/|Z:\\|; s|/|\\|g'
    fi
}

# Run a CSDK tool: linux exe path first, then already-Windows-formatted args.
run_tool() {
    local exe="$1"
    shift
    if [ "$RUNNER" = "proton" ]; then
        protontricks-launch --appid "$APPID" "$exe" "$@"
    else
        wine "$(winpath "$exe")" "$@"
    fi
}

rm -rf "$CONTENT" "$GAME"
mkdir -p "$CONTENT" "$GAME" "$(dirname "$OUTPUT")"
cp -r "$SRC/." "$CONTENT/"

mapfile -t sources < <(find "$CONTENT" -type f \( -name '*.js' -o -name '*.xml' \) | sort)
[ "${#sources[@]}" -gt 0 ] || { echo "No Panorama sources found under $SRC" >&2; exit 1; }

for src in "${sources[@]}"; do
    echo "Compiling ${src#"$CONTENT"/}"
    run_tool "$COMPILER" -i "$(winpath "$src")" -nop4
done

rm -f "$OUTPUT"
echo "Packing VPK to $OUTPUT"
run_tool "$PACKER" "$(winpath "$GAME")" "$(winpath "$OUTPUT")"

[ -f "$OUTPUT" ] || { echo "VPK not created at $OUTPUT" >&2; exit 1; }
echo "Built $OUTPUT"
