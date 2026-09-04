# Build the death_http_bridge Deadlock addon.
#
# Windows: runs scripts/build.ps1 with PowerShell (native CSDK tools).
# Linux:   runs scripts/build.sh, which runs the CSDK tools under Wine.
#
# Usage:
#   just build               # build dist/deadlock_death_hook.vpk
#   just build out.vpk       # build to a different path
#   just clean               # remove dist/ and CSDK build dirs
#   just companion-build     # debug build of the Rust companion
#   just companion-run       # debug build + launch the companion
#   just companion-test      # run the Rust test suite
#
# Override the CSDK location with `csdk=...` or $DEADLOCK_CSDK.
# On Windows an empty value lets scripts/build.ps1 auto-detect the install.

csdk := if os_family() == "windows" { env_var_or_default('DEADLOCK_CSDK', '') } else { env_var_or_default('DEADLOCK_CSDK', '/home/cat/Documents/Reduced_CSDK_12') }
output := 'dist/deadlock_death_hook.vpk'

# compile Panorama sources and pack the VPK
build out=output:
    @just _build-{{os_family()}} "{{out}}"

# Windows path: native tools via scripts/build.ps1 (powershell ships with Windows;
# swap to pwsh if you prefer PowerShell 7 — the script is compatible).
_build-windows out:
    powershell -NoProfile -ExecutionPolicy Bypass -File "{{justfile_directory()}}/scripts/build.ps1" -CsdkRoot "{{csdk}}" -OutputPath "{{out}}"

# Linux path: CSDK tools under Wine.
_build-unix out:
    "{{justfile_directory()}}/scripts/build.sh" "{{csdk}}" "{{justfile_directory()}}" "{{out}}"

# remove build artifacts
clean:
    @just _clean-{{os_family()}}

_clean-windows:
    powershell -NoProfile -Command "$csdk='{{csdk}}'; Remove-Item '{{justfile_directory()}}/dist' -Recurse -Force -ErrorAction SilentlyContinue; if ($csdk) { Remove-Item (Join-Path $csdk 'content/citadel_addons/death_http_bridge'), (Join-Path $csdk 'game/citadel_addons/death_http_bridge') -Recurse -Force -ErrorAction SilentlyContinue }"

_clean-unix:
    #!/usr/bin/env bash
    set -euo pipefail
    rm -rf "{{justfile_directory()}}/dist"
    if [ -n "{{csdk}}" ]; then
        rm -rf "{{csdk}}/content/citadel_addons/death_http_bridge" "{{csdk}}/game/citadel_addons/death_http_bridge"
    fi

# ---- Rust companion (native cargo, same on Windows and Linux) ----

# debug build of the companion (pass extra cargo args, e.g. `just companion-build --release`)
companion-build *args:
    cargo build --manifest-path "{{justfile_directory()}}/companion/Cargo.toml" {{args}}

# debug build + launch, mirroring scripts/build_and_run.bat
companion-run *args:
    cargo run --manifest-path "{{justfile_directory()}}/companion/Cargo.toml" {{args}}

# release build of the companion
companion-release *args:
    cargo build --locked --release --manifest-path "{{justfile_directory()}}/companion/Cargo.toml" {{args}}

# Rust test suite (companion + lovense crates)
companion-test *args:
    cargo test --manifest-path "{{justfile_directory()}}/companion/Cargo.toml" {{args}}

# fast typecheck without producing binaries
companion-check *args:
    cargo check --manifest-path "{{justfile_directory()}}/companion/Cargo.toml" {{args}}

default:
    @just --list
