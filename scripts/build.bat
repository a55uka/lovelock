@echo off
setlocal
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0build.ps1" %*
set BUILD_RESULT=%ERRORLEVEL%
if not %BUILD_RESULT% == 0 (
    echo.
    echo Build failed with exit code %BUILD_RESULT%.
    pause
)
exit /b %BUILD_RESULT%
