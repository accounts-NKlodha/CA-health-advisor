@echo off
rem HealthierYou office test kit - PC2 (camera), PAIRING RETEST. Double-click this file to
rem redo only the pairing with this kit's own builds, after a first run whose pairing
rem could not work. It keeps its own notes and results: the first run's are not changed.
rem PowerShell runs with "-ExecutionPolicy Bypass" for this one process only: nothing
rem on the PC is changed by that. The kit asks Windows once for administrator rights
rem (one UAC prompt, explained on screen) only if a step it still has to do needs them.
setlocal
set "HYKIT_LAUNCHER=%~f0"
title HealthierYou test kit - PC2 (camera) - pairing retest
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0Invoke-OfficeTestKit.ps1" -Role Camera -Retest Pairing %*
echo.
echo You can close this window now.
pause
