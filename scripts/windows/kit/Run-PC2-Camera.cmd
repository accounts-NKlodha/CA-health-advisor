@echo off
rem HealthierYou office test kit - PC2 (the camera PC). Double-click this file to start
rem the tests, or to carry on after a restart, a sign-out or a closed window.
rem PowerShell runs with "-ExecutionPolicy Bypass" for this one process only: nothing
rem on the PC is changed by that. The kit asks Windows once for administrator rights
rem (one UAC prompt, explained on screen) only if a step it still has to do needs them.
setlocal
set "HYKIT_LAUNCHER=%~f0"
title HealthierYou test kit - PC2 (camera)
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0Invoke-OfficeTestKit.ps1" -Role Camera %*
echo.
echo You can close this window now.
pause
