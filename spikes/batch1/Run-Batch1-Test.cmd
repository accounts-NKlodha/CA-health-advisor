@echo off
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0Run-Batch1-Test.ps1" %*
echo.
echo Finished. Press any key to close this window.
pause >nul
