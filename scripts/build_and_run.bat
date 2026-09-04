@echo off
cd /d "%~dp0"
cargo build --manifest-path companion/Cargo.toml 2>&1
if %ERRORLEVEL% == 0 (
    start "" "companion\target\debug\companion.exe"
) else (
    pause
)
