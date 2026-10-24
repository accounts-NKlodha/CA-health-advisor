<#
.SYNOPSIS
  Install, upgrade, roll back and uninstall the HealthierYou Server on a Windows CI runner
  (slice V1-6b). Needs an administrator (the runner's account is one).

.DESCRIPTION
  Uses real installers built in the same job:
    S1 (VersionA)  the real server
    S2 (VersionB)  the real server, a newer release number
    S3 (VersionC)  a release whose hy-server.exe cannot start (the rollback case)
    S4 (VersionD)  a test release that changes the database so older servers refuse it,
                   leaves that in the WAL, then fails to start
  a copy of S2 signed with a key the server does not trust, and hy-sim, which seeds a
  populated data folder and mirror (users, devices, summaries, control log) and counts
  what a data folder holds.

  Checks, with the seeded data counted before and after every step that keeps it:
  - the data folder made before the install, with extra Users entries and an untrusted
    owner on the folder and its files, is closed (owner and every entry checked);
  - the service under NT SERVICE\HealthierYouServer, delayed automatic start, restart on
    failure, TCP 8443 with a private/domain firewall rule, the admin page on loopback;
  - upgrades checked by the installed server: another key, no .sig and the same version
    again are refused (exit 3), nothing changed;
  - another mirror on an upgrade and on a reinstall that keeps the data is refused
    (exit 6), nothing changed; a reinstall without /MIRROR uses the remembered one; a
    copied mirror (a verified move) is accepted;
  - the program copy cannot be made: refused before anything is replaced (exit 7);
  - S3 does not start: the previous program is back and running (exit 4);
  - S4 changes the database and fails: the previous program and the database copy are
    back and running with the same data, the failed database is kept with its WAL
    (exit 4);
  - putting the previous version back cannot complete: exit 9, nothing deleted, the
    copies kept, a new setup refuses to overwrite them (exit 7); the documented recovery
    brings the data back;
  - going back to exactly the previous version is allowed;
  - uninstall keeps the data, removes the service, the rule, the program and the
    registry keys; /DELETEDATA removes the data too; a fresh install makes new keys.
#>
[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)][string]$Installers,
  [Parameter(Mandatory = $true)][string]$OtherKeyDir,
  # On another volume than C: (the control-log mirror must be).
  [Parameter(Mandatory = $true)][string]$Mirror,
  [Parameter(Mandatory = $true)][string]$Work,
  [Parameter(Mandatory = $true)][string]$Sim,
  [string]$VersionA = '0.1.0',
  [string]$VersionB = '0.1.1',
  [string]$VersionC = '0.1.2',
  [string]$VersionD = '0.1.3'
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 3

$Service = 'HealthierYouServer'
$Account = 'NT SERVICE\HealthierYouServer'
$Program = Join-Path $env:ProgramFiles 'HealthierYou Server'
$Exe = Join-Path $Program 'hy-server.exe'
$Data = Join-Path $env:ProgramData 'HealthierYou\Server'
$UninstallKey = 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\HealthierYouServer'
$SettingsKey = 'HKLM:\Software\HealthierYou\Server'
$Rule = 'HealthierYou Server'
New-Item -ItemType Directory -Force $Work | Out-Null

$script:Results = [System.Collections.Generic.List[object]]::new()
function Check([string]$Name, [bool]$Ok, [string]$Detail = '') {
  $script:Results.Add([pscustomobject]@{ Check = $Name; Ok = $Ok; Detail = $Detail })
  $mark = if ($Ok) { 'PASS' } else { 'FAIL' }
  Write-Host ("[{0}] {1} {2}" -f $mark, $Name, $Detail)
}
function Installer([string]$Version, [string]$Dir = $Installers) { Join-Path $Dir ("HealthierYou-Server_{0}_x64-setup.exe" -f $Version) }
# Wait for the started program alone (Start-Process -Wait also waits for what it starts).
function Start-AndWait([string]$Path, [string[]]$Arguments, [int]$Minutes = 10) {
  $p = Start-Process -FilePath $Path -ArgumentList $Arguments -PassThru
  $null = $p.Handle  # keep the handle so the exit code is readable
  if (-not $p.WaitForExit($Minutes * 60000)) {
    $p | Stop-Process -Force -ErrorAction SilentlyContinue
    return -1
  }
  $p.ExitCode
}
function Run-Setup([string]$Path, [string[]]$Arguments) { Start-AndWait $Path $Arguments }
function Invoke-Uninstall([string[]]$Arguments) {
  $u = Join-Path $Program 'uninstall.exe'
  if (-not (Test-Path $u)) { return $false }
  $null = Start-AndWait $u $Arguments
  # The uninstaller runs on from a temp copy (Au_.exe); wait for it.
  $deadline = (Get-Date).AddMinutes(3)
  while ((Get-Date) -lt $deadline) {
    if (@(Get-Process -Name 'Au_' -ErrorAction SilentlyContinue).Count -eq 0 -and -not (Test-Path $UninstallKey)) { return $true }
    Start-Sleep -Milliseconds 500
  }
  $false
}
function RegValue([string]$Key, [string]$Name) {
  $p = Get-ItemProperty -Path $Key -Name $Name -ErrorAction SilentlyContinue
  if ($p) { $p.$Name } else { $null }
}
function Service-State { $s = Get-Service -Name $Service -ErrorAction SilentlyContinue; if ($s) { "$($s.Status)" } else { 'Missing' } }
function Wait-Listening([int]$Port, [int]$Seconds = 60) {
  $deadline = (Get-Date).AddSeconds($Seconds)
  while ((Get-Date) -lt $deadline) {
    $l = @(Get-NetTCPConnection -State Listen -LocalPort $Port -ErrorAction SilentlyContinue)
    if ($l.Count -gt 0) { return $l }
    Start-Sleep -Seconds 1
  }
  @()
}
function Hash([string]$Path) { if (Test-Path $Path) { (Get-FileHash $Path -Algorithm SHA256).Hash } else { '' } }
function Service-Config { Get-CimInstance Win32_Service -Filter "Name = '$Service'" }
function Show-ServiceError {
  $f = Join-Path $Data 'service-error.txt'
  if (Test-Path $f) { Write-Host '--- service-error.txt ---'; Get-Content $f | Write-Host }
}

$Mirror = $Mirror.TrimEnd('\')
$Mirror2 = "$Mirror-moved"
$MirrorFile = 'control-log.mirror'
$Users = 'S-1-5-32-545'

# What the data folder holds: users and summaries must stay the same; the control log
# may only grow.
function Counts {
  $o = (& $Sim count $Data 2>&1) -join ' '
  if ($LASTEXITCODE -ne 0) { return @{ ok = $false; text = $o } }
  $h = @{ ok = $true; text = $o }
  foreach ($m in [regex]::Matches($o, '(\w+)=(\d+)')) { $h[$m.Groups[1].Value] = [int]$m.Groups[2].Value }
  $h
}
function Check-Data([string]$When) {
  $c = Counts
  Check "${When}: data kept (users, summaries, control log)" ($c.ok -and $c.users -eq $script:Seed.users -and $c.summaries -eq $script:Seed.summaries -and $c.control_entries -ge $script:Seed.control_entries) $c.text
}
# Open the data folder up again the ways a folder made before the install can be: an
# explicit Users entry on the folder and on a key file, files owned by Users, and a file
# with its own Everyone entry and no inheritance.
function Open-Up {
  & icacls.exe $Data /grant "*${Users}:(OI)(CI)F" /Q | Out-Null
  & icacls.exe (Join-Path $Data 'keys\tls.key.dpapi') /grant "*${Users}:F" /Q | Out-Null
  & icacls.exe (Join-Path $Data 'server.db') /setowner "*$Users" /Q | Out-Null
  & icacls.exe (Join-Path $Data 'keys') /setowner "*$Users" /Q | Out-Null
  $stray = Join-Path $Data 'notes.txt'
  Set-Content -Path $stray -Value 'left by hand'
  & icacls.exe $stray /inheritance:r /grant '*S-1-1-0:F' /Q | Out-Null
  $before = Find-Open
  Check 'test setup: the data folder was opened up' ($before.Count -gt 0) ("{0} findings" -f $before.Count)
}
# Every owner and access entry in the data folder that is not SYSTEM, Administrators or
# the service (read independently of hy-server check-acl).
function Find-Open {
  $sid = (New-Object Security.Principal.NTAccount($Account)).Translate([Security.Principal.SecurityIdentifier]).Value
  $ok = @('S-1-5-18', 'S-1-5-32-544', $sid)
  $found = [System.Collections.Generic.List[string]]::new()
  foreach ($item in @(Get-Item -Force $Data) + @(Get-ChildItem -Force -Recurse $Data)) {
    $acl = Get-Acl -LiteralPath $item.FullName
    $owner = $acl.GetOwner([Security.Principal.SecurityIdentifier]).Value
    if ($ok -notcontains $owner) { $found.Add("$($item.Name): owner $owner") }
    foreach ($r in $acl.GetAccessRules($true, $true, [Security.Principal.SecurityIdentifier])) {
      $s = $r.IdentityReference.Value
      if ("$($r.AccessControlType)" -eq 'Allow' -and $ok -notcontains $s) { $found.Add("$($item.Name): $s") }
    }
  }
  , $found
}
function Check-Closed([string]$When) {
  $out = (& $Exe check-acl 2>&1) -join ' '
  Check "${When}: hy-server check-acl passes" ($LASTEXITCODE -eq 0) $out
  $open = Find-Open
  Check "${When}: no other owner or entry anywhere in the data folder" ($open.Count -eq 0) (($open | Select-Object -First 5) -join '; ')
  Check "${When}: the data folder does not inherit" ((Get-Acl $Data).AreAccessRulesProtected)
}
# Hold a file open so nobody else can use it, until the returned stream is closed.
function Hold([string]$Path, [System.IO.FileShare]$Share = [System.IO.FileShare]::None) {
  [System.IO.File]::Open($Path, [System.IO.FileMode]::OpenOrCreate, [System.IO.FileAccess]::Read, $Share)
}
# The schema version in a SQLite file's header (bytes 60-63, big-endian).
function User-Version([string]$Path) {
  $fs = [System.IO.File]::Open($Path, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read, [System.IO.FileShare]::ReadWrite)
  try { $b = New-Object byte[] 64; [void]$fs.Read($b, 0, 64); ([int]$b[60] -shl 24) -bor ([int]$b[61] -shl 16) -bor ([int]$b[62] -shl 8) -bor [int]$b[63] }
  finally { $fs.Dispose() }
}
function Wait-Running([int]$Seconds = 90) {
  $deadline = (Get-Date).AddSeconds($Seconds)
  while ((Get-Date) -lt $deadline) { if ((Service-State) -eq 'Running') { return $true }; Start-Sleep -Seconds 1 }
  $false
}

try {
  Check 'clean runner: no service before' ((Service-State) -eq 'Missing')

  # --------------------------------- a populated data folder from before the install
  $seedOut = (& $Sim seed $Data (Join-Path $Mirror $MirrorFile) 2>&1) -join ' '
  Check 'seed: hy-sim seeded a data folder and mirror' ($LASTEXITCODE -eq 0) $seedOut
  $script:Seed = Counts
  Check 'seed: users, devices and summaries present' ($script:Seed.ok -and $script:Seed.users -gt 0 -and $script:Seed.summaries -gt 0) $script:Seed.text
  $cert1 = Hash (Join-Path $Data 'keys\tls.cert.der')
  Open-Up

  # ------------------------------------------------------------------ install S1
  $code = Run-Setup (Installer $VersionA) @('/S', "/MIRROR=$Mirror")
  Check 'install: exit code 0' ($code -eq 0) "exit $code"
  if ($code -ne 0) { Show-ServiceError }
  Check 'install: service running' ((Service-State) -eq 'Running') (Service-State)
  $svc = Service-Config
  Check 'install: runs as NT SERVICE\HealthierYouServer (virtual account)' ($svc -and $svc.StartName -eq $Account) "$($svc.StartName)"
  Check 'install: automatic (delayed) start' ($svc -and $svc.StartMode -eq 'Auto' -and $svc.DelayedAutoStart) "$($svc.StartMode) delayed=$($svc.DelayedAutoStart)"
  $failure = (& sc.exe qfailure $Service) -join ' '
  Check 'install: restarts after a failure' ($failure -match 'RESTART')
  Check 'install: program in Program Files' (Test-Path $Exe)
  Check 'install: machine-wide uninstall entry' ((RegValue $UninstallKey 'DisplayVersion') -eq $VersionA)
  Check 'install: the kept keys are used (no new keys)' ((Hash (Join-Path $Data 'keys\tls.cert.der')) -eq $cert1)
  Check 'install: mirror location remembered in the data folder' ((Get-Content (Join-Path $Data 'mirror-location.txt') -ErrorAction SilentlyContinue) -eq $Mirror)
  Check-Data 'install'
  Check-Closed 'install over a data folder opened to Users'

  $api = Wait-Listening 8443
  Check 'API: TCP 8443 listening' ($api.Count -gt 0)
  Check 'API: owned by hy-server.exe' (@($api | Where-Object { (Get-Process -Id $_.OwningProcess -ErrorAction SilentlyContinue).Name -eq 'hy-server' }).Count -gt 0)
  $admin = Wait-Listening 8444
  Check 'admin page: listening on loopback only' ($admin.Count -gt 0 -and @($admin | Where-Object { $_.LocalAddress -ne '127.0.0.1' }).Count -eq 0) (($admin | ForEach-Object LocalAddress) -join ',')
  $fw = @(Get-NetFirewallRule -DisplayName $Rule -ErrorAction SilentlyContinue)
  Check 'firewall: one inbound allow rule' ($fw.Count -eq 1 -and "$($fw[0].Direction)" -eq 'Inbound' -and "$($fw[0].Action)" -eq 'Allow')
  if ($fw.Count -eq 1) {
    $profiles = "$($fw[0].Profile)"
    Check 'firewall: private and domain only (not public)' ($profiles -match 'Domain' -and $profiles -match 'Private' -and $profiles -notmatch 'Public') $profiles
    $port = $fw[0] | Get-NetFirewallPortFilter
    Check 'firewall: TCP 8443 only' ("$($port.Protocol)" -eq 'TCP' -and "$($port.LocalPort)" -eq '8443')
    $app = $fw[0] | Get-NetFirewallApplicationFilter
    Check 'firewall: for hy-server.exe only' ($app.Program -eq $Exe)
  }
  $fp1 = (& $Exe fingerprint) -join "`n"
  Check 'administrator can read the fingerprint (machine-scope DPAPI)' ($LASTEXITCODE -eq 0 -and $fp1 -match 'fingerprint [0-9a-f]{64}')

  # ------------------------------------------------------ refused upgrades
  $exe1 = Hash $Exe
  $code = Run-Setup (Installer $VersionB $OtherKeyDir) @('/S')
  Check 'upgrade refused: signed with another key (exit 3)' ($code -eq 3) "exit $code"
  $nosig = Join-Path $Work 'nosig'; New-Item -ItemType Directory -Force $nosig | Out-Null
  Copy-Item (Installer $VersionB) $nosig
  $code = Run-Setup (Installer $VersionB $nosig) @('/S')
  Check 'upgrade refused: no signature file (exit 3)' ($code -eq 3) "exit $code"
  $code = Run-Setup (Installer $VersionA) @('/S')
  Check 'upgrade refused: the same version again (exit 3)' ($code -eq 3) "exit $code"
  Check 'upgrade refused: nothing changed' ((Hash $Exe) -eq $exe1 -and (RegValue $UninstallKey 'DisplayVersion') -eq $VersionA -and (Service-State) -eq 'Running')
  $code = Run-Setup (Installer $VersionB) @('/S', "/MIRROR=$Mirror2")
  Check 'upgrade refused: another mirror (exit 6)' ($code -eq 6) "exit $code"
  Check 'upgrade refused, another mirror: nothing changed, running' ((Hash $Exe) -eq $exe1 -and (RegValue $SettingsKey 'MirrorFolder') -eq $Mirror -and (Wait-Running)) "$(RegValue $SettingsKey 'MirrorFolder')"
  Check 'upgrade refused, another mirror: the service still uses the data''s mirror' ((Service-Config).PathName -like "*$Mirror\$MirrorFile*")
  Check-Data 'upgrade refused, another mirror'

  # ------------------------------------------------------------------ upgrade S2
  $code = Run-Setup (Installer $VersionB) @('/S')
  Check 'upgrade: exit code 0' ($code -eq 0) "exit $code"
  if ($code -ne 0) { Show-ServiceError }
  Check 'upgrade: entry shows the new version' ((RegValue $UninstallKey 'DisplayVersion') -eq $VersionB)
  Check 'upgrade: version and the one it came from recorded' ((RegValue $SettingsKey 'Version') -eq $VersionB -and (RegValue $SettingsKey 'PreviousVersion') -eq $VersionA)
  Check 'upgrade: service running' ((Service-State) -eq 'Running')
  Check 'upgrade: mirror kept' ((RegValue $SettingsKey 'MirrorFolder') -eq $Mirror)
  Check 'upgrade: same keys (no re-pairing)' ((Hash (Join-Path $Data 'keys\tls.cert.der')) -eq $cert1)
  Check 'upgrade: copies cleared after a good start' (-not (Test-Path (Join-Path $Program 'hy-server.previous.exe')) -and -not (Test-Path (Join-Path $Data 'pre-update')))
  Check 'upgrade: still listening' ((Wait-Listening 8443).Count -gt 0)
  Check-Data 'upgrade'
  Check-Closed 'upgrade'

  # ------------------------------- the program copy cannot be made: nothing replaced
  $exe2 = Hash $Exe
  $prevCopy = Join-Path $Program 'hy-server.previous.exe'
  Copy-Item $Exe $prevCopy
  $h = Hold $prevCopy
  try { $code = Run-Setup (Installer $VersionC) @('/S') } finally { $h.Dispose() }
  Remove-Item -Force $prevCopy -ErrorAction SilentlyContinue
  Check 'copy fails: refused before anything is replaced (exit 7)' ($code -eq 7) "exit $code"
  Check 'copy fails: program and entry unchanged' ((Hash $Exe) -eq $exe2 -and (RegValue $UninstallKey 'DisplayVersion') -eq $VersionB)
  Check 'copy fails: the service runs again' (Wait-Running)
  Check-Data 'copy fails'

  # -------------------------------------------- S3 does not start: rolled back
  $code = Run-Setup (Installer $VersionC) @('/S')
  Check 'bad release: rolled back (exit 4)' ($code -eq 4) "exit $code"
  Check 'bad release: previous hy-server.exe back' ((Hash $Exe) -eq $exe2)
  Check 'bad release: entry back to the previous version' ((RegValue $UninstallKey 'DisplayVersion') -eq $VersionB) "$(RegValue $UninstallKey 'DisplayVersion')"
  Check 'bad release: recorded versions unchanged' ((RegValue $SettingsKey 'Version') -eq $VersionB -and (RegValue $SettingsKey 'PreviousVersion') -eq $VersionA)
  Check 'bad release: service running again' (Wait-Running)
  Check 'bad release: same keys' ((Hash (Join-Path $Data 'keys\tls.cert.der')) -eq $cert1)
  Check 'bad release: listening again' ((Wait-Listening 8443).Count -gt 0)
  Check-Data 'bad release'

  # ------------- putting the previous version back cannot complete: exit 9, kept
  # S4 changes the database so the previous version refuses it; a file where the
  # failed version's database folder must go stops the restore half way.
  $block = Join-Path $Data "failed-update-$VersionD"
  Set-Content -Path $block -Value 'in the way'
  $code = Run-Setup (Installer $VersionD) @('/S')
  Check 'restore cannot complete: exit 9' ($code -eq 9) "exit $code"
  Check 'restore cannot complete: previous program back' ((Hash $Exe) -eq $exe2)
  Check 'restore cannot complete: nothing deleted (database and both copies kept)' ((Test-Path (Join-Path $Data 'server.db')) -and (Test-Path (Join-Path $Data 'pre-update\server.db')) -and (Test-Path $prevCopy))
  Remove-Item -Force $block
  $code = Run-Setup (Installer $VersionD) @('/S')
  Check 'restore cannot complete: a new setup refuses to overwrite the copies (exit 7)' ($code -eq 7) "exit $code"
  Check 'restore cannot complete: the copies are still there' (Test-Path (Join-Path $Data 'pre-update\server.db'))
  # The documented recovery: set the changed database aside, put the copy back, start.
  & sc.exe config $Service start= disabled | Out-Null
  Stop-Service $Service -Force -ErrorAction SilentlyContinue
  $aside = Join-Path $Work 'changed-database'; New-Item -ItemType Directory -Force $aside | Out-Null
  Get-ChildItem -Force $Data -Filter 'server.db*' | Move-Item -Destination $aside
  Copy-Item (Join-Path $Data 'pre-update\*') $Data
  Move-Item (Join-Path $Data 'pre-update') (Join-Path $Work 'pre-update-used')
  Remove-Item -Force $prevCopy
  & sc.exe config $Service start= delayed-auto | Out-Null
  Start-Service $Service -ErrorAction SilentlyContinue
  Check 'restore cannot complete: the documented recovery brings the server back' (Wait-Running)
  Check-Data 'restore cannot complete, after the recovery'

  # ------------------- S4 changes the database and fails: program and data back
  $code = Run-Setup (Installer $VersionD) @('/S')
  Check 'schema-changing release: rolled back (exit 4)' ($code -eq 4) "exit $code"
  if ($code -ne 4) { Show-ServiceError }
  Check 'schema-changing release: previous program back' ((Hash $Exe) -eq $exe2)
  Check 'schema-changing release: service running on the database copy' (Wait-Running)
  Check-Data 'schema-changing release'
  $failedDir = Join-Path $Data "failed-update-$VersionD"
  Check 'schema-changing release: failed database kept' (Test-Path (Join-Path $failedDir 'server.db'))
  # The change is in the WAL S4 left, or in the database itself if the previous
  # version's attempt to open it folded the WAL in; either way it is kept together.
  $failedDb = Join-Path $failedDir 'server.db'
  $kept = (Test-Path (Join-Path $failedDir 'server.db-wal')) -or ((Test-Path $failedDb) -and (User-Version $failedDb) -ge 3)
  Check 'schema-changing release: the failed version''s change is kept with its database (WAL kept with it)' $kept
  Check 'schema-changing release: copies cleared after the rollback' (-not (Test-Path (Join-Path $Data 'pre-update')) -and -not (Test-Path $prevCopy))
  Check-Closed 'schema-changing release'

  # ------------------------------- back to exactly the previous version: allowed
  $code = Run-Setup (Installer $VersionA) @('/S')
  Check 'back to the previous version: allowed (exit 0)' ($code -eq 0) "exit $code"
  Check 'back to the previous version: recorded' ((RegValue $SettingsKey 'Version') -eq $VersionA -and (RegValue $SettingsKey 'PreviousVersion') -eq $VersionB)
  Check 'back to the previous version: running' (Wait-Running)
  Check-Data 'back to the previous version'

  # ------------------------------------------------ uninstall keeps the data
  $ok = Invoke-Uninstall @('/S')
  Check 'uninstall: finished' $ok
  Check 'uninstall: service removed' ((Service-State) -eq 'Missing')
  Check 'uninstall: firewall rule removed' (@(Get-NetFirewallRule -DisplayName $Rule -ErrorAction SilentlyContinue).Count -eq 0)
  Check 'uninstall: program removed' (-not (Test-Path $Program))
  Check 'uninstall: no uninstall entry' (-not (Test-Path $UninstallKey))
  Check 'uninstall: no HKLM\Software\HealthierYou key' (-not (Test-Path 'HKLM:\Software\HealthierYou'))
  Check 'uninstall: data kept (keys, database, mirror location)' ((Test-Path (Join-Path $Data 'keys\tls.cert.der')) -and (Test-Path (Join-Path $Data 'server.db')) -and (Test-Path (Join-Path $Data 'mirror-location.txt')))
  Check 'uninstall: mirror kept' (Test-Path (Join-Path $Mirror $MirrorFile))
  Check-Data 'uninstall'

  # ------------------------- reinstall keeping the data: another mirror is refused
  $code = Run-Setup (Installer $VersionB) @('/S', "/MIRROR=$Mirror2")
  Check 'reinstall refused: another mirror (exit 6)' ($code -eq 6) "exit $code"
  Check 'reinstall refused: nothing installed' ((Service-State) -eq 'Missing' -and -not (Test-Path $UninstallKey) -and -not (Test-Path $Exe))
  Check-Data 'reinstall refused'

  # ------------------------- reinstall without /MIRROR uses the remembered mirror
  Open-Up
  $code = Run-Setup (Installer $VersionB) @('/S')
  Check 'reinstall: exit code 0' ($code -eq 0) "exit $code"
  if ($code -ne 0) { Show-ServiceError }
  Check 'reinstall: the remembered mirror is used' ((RegValue $SettingsKey 'MirrorFolder') -eq $Mirror -and (Service-Config).PathName -like "*$Mirror\$MirrorFile*")
  Check 'reinstall: same keys (PCs stay paired)' ((Hash (Join-Path $Data 'keys\tls.cert.der')) -eq $cert1)
  Check 'reinstall: running' (Wait-Running)
  Check-Data 'reinstall'
  Check-Closed 'reinstall over a data folder opened to Users'

  # ---------------------- moving the mirror: copy it, then give the new folder
  $ok = Invoke-Uninstall @('/S')
  Check 'move: uninstall keeping the data' $ok
  New-Item -ItemType Directory -Force $Mirror2 | Out-Null
  Copy-Item (Join-Path $Mirror $MirrorFile) $Mirror2
  $code = Run-Setup (Installer $VersionB) @('/S', "/MIRROR=$Mirror2")
  Check 'move: reinstall with the copied mirror (exit 0)' ($code -eq 0) "exit $code"
  if ($code -ne 0) { Show-ServiceError }
  Check 'move: the new mirror is used and remembered' ((RegValue $SettingsKey 'MirrorFolder') -eq $Mirror2 -and (Get-Content (Join-Path $Data 'mirror-location.txt')) -eq $Mirror2)
  Check 'move: running' (Wait-Running)
  Check-Data 'move'

  # -------------------------------------------------- uninstall with /DELETEDATA
  $ok = Invoke-Uninstall @('/S', '/DELETEDATA')
  Check 'uninstall /DELETEDATA: finished' $ok
  Check 'uninstall /DELETEDATA: service removed' ((Service-State) -eq 'Missing')
  Check 'uninstall /DELETEDATA: data folder removed' (-not (Test-Path (Join-Path $env:ProgramData 'HealthierYou')))
  Check 'uninstall /DELETEDATA: mirror removed' (-not (Test-Path (Join-Path $Mirror2 $MirrorFile)))
  Check 'uninstall /DELETEDATA: no registry keys left' (-not (Test-Path $UninstallKey) -and -not (Test-Path 'HKLM:\Software\HealthierYou'))
  Remove-Item -Recurse -Force $Mirror -ErrorAction SilentlyContinue

  # ----------------------------------------------- fresh install with no data
  $code = Run-Setup (Installer $VersionB) @('/S', "/MIRROR=$Mirror")
  Check 'fresh install: exit code 0' ($code -eq 0) "exit $code"
  if ($code -ne 0) { Show-ServiceError }
  Check 'fresh install: keys made by the service at its first start' (Test-Path (Join-Path $Data 'keys\tls.cert.der'))
  Check 'fresh install: private keys sealed (DPAPI files)' ((Test-Path (Join-Path $Data 'keys\tls.key.dpapi')) -and (Test-Path (Join-Path $Data 'keys\head.key.dpapi')))
  Check 'fresh install: new keys' ((Hash (Join-Path $Data 'keys\tls.cert.der')) -ne $cert1)
  Check 'fresh install: mirror written on the other volume' (Test-Path (Join-Path $Mirror $MirrorFile))
  Check-Closed 'fresh install'
  $ok = Invoke-Uninstall @('/S', '/DELETEDATA')
  Check 'fresh install: removed again with /DELETEDATA' ($ok -and -not (Test-Path (Join-Path $env:ProgramData 'HealthierYou')))
}
catch {
  Check 'script error' $false $_.Exception.Message
  Show-ServiceError
}

$failed = @($script:Results | Where-Object { -not $_.Ok })
"{0} checks, {1} failed" -f $script:Results.Count, $failed.Count | Write-Host
$script:Results | Export-Csv -NoTypeInformation -Path (Join-Path $Work 'server-lifecycle.csv')
if ($failed.Count -gt 0) { exit 1 }
