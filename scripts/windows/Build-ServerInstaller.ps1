<#
.SYNOPSIS
  Build the HealthierYou Server installer (slice V1-6b):
  HealthierYou-Server_<version>_x64-setup.exe from a built hy-server.exe.

.DESCRIPTION
  Uses NSIS: the copy the Tauri CLI downloads for the companion's installer
  (%LOCALAPPDATA%\tauri\NSIS), an installed NSIS, or -MakeNsis. Signing is a separate
  step, the same as for the companion:
    npx tauri signer sign -f <key> --app-version <version> HealthierYou-Server_<version>_x64-setup.exe
  The installed server checks that .sig before an upgrade replaces anything.

.EXAMPLE
  cargo build --release --locked -p hy-server
  .\scripts\windows\Build-ServerInstaller.ps1 -Version 0.1.0 -ServerExe target\release\hy-server.exe -OutDir dist
#>
[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)][ValidatePattern('^\d+\.\d+\.\d+$')][string]$Version,
  [Parameter(Mandatory = $true)][string]$ServerExe,
  [Parameter(Mandatory = $true)][string]$OutDir,
  [string]$MakeNsis
)
$ErrorActionPreference = 'Stop'

$candidates = @(
  $MakeNsis,
  (Join-Path $env:LOCALAPPDATA 'tauri\NSIS\makensis.exe'),
  (Join-Path ${env:ProgramFiles(x86)} 'NSIS\makensis.exe'),
  (Join-Path $env:ProgramFiles 'NSIS\makensis.exe')
) | Where-Object { $_ -and (Test-Path $_) }
if (-not $candidates) { throw 'NSIS (makensis.exe) not found: build the companion installer first, install NSIS, or pass -MakeNsis' }
$tool = @($candidates)[0]

$script = Join-Path $PSScriptRoot '..\..\crates\hy-server\windows\server-installer.nsi'
New-Item -ItemType Directory -Force $OutDir | Out-Null
$out = Join-Path (Resolve-Path $OutDir) ("HealthierYou-Server_{0}_x64-setup.exe" -f $Version)
$src = (Resolve-Path $ServerExe).Path
& $tool /V2 /INPUTCHARSET UTF8 "/DVERSION=$Version" "/DSRCEXE=$src" "/DOUTFILE=$out" (Resolve-Path $script).Path
if ($LASTEXITCODE -ne 0) { throw "makensis failed (exit $LASTEXITCODE)" }
$f = Get-Item $out
"{0} size_mb={1} sha256={2}" -f $f.Name, [math]::Round($f.Length / 1MB, 1), (Get-FileHash $f.FullName -Algorithm SHA256).Hash
