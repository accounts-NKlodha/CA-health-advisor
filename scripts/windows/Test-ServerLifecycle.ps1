<#
.SYNOPSIS
  Install, upgrade, roll back and uninstall the HealthierYou Server on a Windows CI runner
  (slice V1-6b). Needs an administrator (the runner's account is one).

.DESCRIPTION
  Uses real installers built in the same job:
    S1 (VersionA)  the real server
    S2 (VersionB)  the real server, a newer release number
    S3 (VersionC)  a release whose hy-server.exe cannot start (the rollback case)
  and a copy of S2 signed with a key the server does not trust.

  Checks: the service under NT SERVICE\HealthierYouServer, delayed automatic start and
  restart on failure; the data folder open only to SYSTEM, Administrators and the
  service; keys made by the service at its first start; TCP 8443 listening with a
  firewall rule for private and domain networks only; the admin page on loopback only;
  an upgrade checked by the installed server (other key and missing .sig refused,
  nothing changed); a release that does not start is rolled back; uninstall keeps the
  data and removes the service, the rule, the program and its registry keys; a
  reinstall reuses the kept keys; uninstall with /DELETEDATA removes the data too.
#>
[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)][string]$Installers,
  [Parameter(Mandatory = $true)][string]$OtherKeyDir,
  # On another volume than C: (the control-log mirror must be).
  [Parameter(Mandatory = $true)][string]$Mirror,
  [Parameter(Mandatory = $true)][string]$Work,
  [string]$VersionA = '0.1.0',
  [string]$VersionB = '0.1.1',
  [string]$VersionC = '0.1.2'
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
function Run-Setup([string]$Path, [string[]]$Arguments) {
  $p = Start-Process -FilePath $Path -ArgumentList $Arguments -PassThru -Wait
  $p.ExitCode
}
function Invoke-Uninstall([string[]]$Arguments) {
  $u = Join-Path $Program 'uninstall.exe'
  if (-not (Test-Path $u)) { return $false }
  Start-Process -FilePath $u -ArgumentList $Arguments -Wait | Out-Null
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

try {
  Check 'clean runner: no service before' ((Service-State) -eq 'Missing')

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
  Check 'install: keys made by the service at its first start' (Test-Path (Join-Path $Data 'keys\tls.cert.der'))
  Check 'install: private keys sealed (DPAPI files)' ((Test-Path (Join-Path $Data 'keys\tls.key.dpapi')) -and (Test-Path (Join-Path $Data 'keys\head.key.dpapi')))
  Check 'install: database created' (Test-Path (Join-Path $Data 'server.db'))
  Check 'install: mirror written on the other volume' (Test-Path (Join-Path $Mirror 'control-log.mirror'))

  $acl = Get-Acl $Data
  $who = @($acl.Access | ForEach-Object { $_.IdentityReference.Translate([Security.Principal.SecurityIdentifier]).Value } | Sort-Object -Unique)
  $serviceSid = (New-Object Security.Principal.NTAccount($Account)).Translate([Security.Principal.SecurityIdentifier]).Value
  $expected = @('S-1-5-18', 'S-1-5-32-544', $serviceSid) | Sort-Object -Unique
  Check 'data folder: inheritance removed' ($acl.AreAccessRulesProtected)
  Check 'data folder: only SYSTEM, Administrators and the service' ((($who -join ',') -eq ($expected -join ','))) ("{0} entries" -f $who.Count)
  $keyAcl = Get-Acl (Join-Path $Data 'keys\tls.key.dpapi')
  $keyWho = @($keyAcl.Access | ForEach-Object { $_.IdentityReference.Translate([Security.Principal.SecurityIdentifier]).Value } | Sort-Object -Unique)
  Check 'key file: no other account' (@($keyWho | Where-Object { $expected -notcontains $_ }).Count -eq 0)

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
  $cert1 = Hash (Join-Path $Data 'keys\tls.cert.der')

  # ------------------------------------------------------ refused upgrades
  $exe1 = Hash $Exe
  $code = Run-Setup (Installer $VersionB $OtherKeyDir) @('/S')
  Check 'upgrade refused: signed with another key (exit 3)' ($code -eq 3) "exit $code"
  $nosig = Join-Path $Work 'nosig'; New-Item -ItemType Directory -Force $nosig | Out-Null
  Copy-Item (Installer $VersionB) $nosig
  $code = Run-Setup (Installer $VersionB $nosig) @('/S')
  Check 'upgrade refused: no signature file (exit 3)' ($code -eq 3) "exit $code"
  Check 'upgrade refused: nothing changed' ((Hash $Exe) -eq $exe1 -and (RegValue $UninstallKey 'DisplayVersion') -eq $VersionA -and (Service-State) -eq 'Running')

  # ------------------------------------------------------------------ upgrade S2
  $code = Run-Setup (Installer $VersionB) @('/S')
  Check 'upgrade: exit code 0' ($code -eq 0) "exit $code"
  if ($code -ne 0) { Show-ServiceError }
  Check 'upgrade: entry shows the new version' ((RegValue $UninstallKey 'DisplayVersion') -eq $VersionB)
  Check 'upgrade: service running' ((Service-State) -eq 'Running')
  Check 'upgrade: mirror folder kept' ((RegValue $SettingsKey 'MirrorFolder') -eq $Mirror.TrimEnd('\'))
  Check 'upgrade: same keys (no re-pairing)' ((Hash (Join-Path $Data 'keys\tls.cert.der')) -eq $cert1)
  Check 'upgrade: previous copy cleared after a good start' (-not (Test-Path (Join-Path $Program 'hy-server.previous.exe')))
  Check 'upgrade: still listening' ((Wait-Listening 8443).Count -gt 0)

  # -------------------------------------------- S3 does not start: rolled back
  $exe2 = Hash $Exe
  $code = Run-Setup (Installer $VersionC) @('/S')
  Check 'bad release: rolled back (exit 4)' ($code -eq 4) "exit $code"
  Check 'bad release: previous hy-server.exe back' ((Hash $Exe) -eq $exe2)
  Check 'bad release: entry back to the previous version' ((RegValue $UninstallKey 'DisplayVersion') -eq $VersionB) "$(RegValue $UninstallKey 'DisplayVersion')"
  Check 'bad release: service running again' ((Service-State) -eq 'Running')
  Check 'bad release: same keys' ((Hash (Join-Path $Data 'keys\tls.cert.der')) -eq $cert1)
  Check 'bad release: listening again' ((Wait-Listening 8443).Count -gt 0)

  # ------------------------------------------------ uninstall keeps the data
  $ok = Invoke-Uninstall @('/S')
  Check 'uninstall: finished' $ok
  Check 'uninstall: service removed' ((Service-State) -eq 'Missing')
  Check 'uninstall: firewall rule removed' (@(Get-NetFirewallRule -DisplayName $Rule -ErrorAction SilentlyContinue).Count -eq 0)
  Check 'uninstall: program removed' (-not (Test-Path $Program))
  Check 'uninstall: no uninstall entry' (-not (Test-Path $UninstallKey))
  Check 'uninstall: no HKLM\Software\HealthierYou key' (-not (Test-Path 'HKLM:\Software\HealthierYou'))
  Check 'uninstall: data kept (keys, database)' ((Test-Path (Join-Path $Data 'keys\tls.cert.der')) -and (Test-Path (Join-Path $Data 'server.db')))
  Check 'uninstall: mirror kept' (Test-Path (Join-Path $Mirror 'control-log.mirror'))

  # -------------------------------------------- reinstall reuses the kept keys
  $code = Run-Setup (Installer $VersionB) @('/S', "/MIRROR=$Mirror")
  Check 'reinstall: exit code 0' ($code -eq 0) "exit $code"
  if ($code -ne 0) { Show-ServiceError }
  Check 'reinstall: same keys (PCs stay paired)' ((Hash (Join-Path $Data 'keys\tls.cert.der')) -eq $cert1)
  Check 'reinstall: running' ((Service-State) -eq 'Running')

  # -------------------------------------------------- uninstall with /DELETEDATA
  $ok = Invoke-Uninstall @('/S', '/DELETEDATA')
  Check 'uninstall /DELETEDATA: finished' $ok
  Check 'uninstall /DELETEDATA: service removed' ((Service-State) -eq 'Missing')
  Check 'uninstall /DELETEDATA: data folder removed' (-not (Test-Path (Join-Path $env:ProgramData 'HealthierYou')))
  Check 'uninstall /DELETEDATA: mirror removed' (-not (Test-Path (Join-Path $Mirror 'control-log.mirror')))
  Check 'uninstall /DELETEDATA: no registry keys left' (-not (Test-Path $UninstallKey) -and -not (Test-Path 'HKLM:\Software\HealthierYou'))
}
catch {
  Check 'script error' $false $_.Exception.Message
  Show-ServiceError
}

$failed = @($script:Results | Where-Object { -not $_.Ok })
"{0} checks, {1} failed" -f $script:Results.Count, $failed.Count | Write-Host
$script:Results | Export-Csv -NoTypeInformation -Path (Join-Path $Work 'server-lifecycle.csv')
if ($failed.Count -gt 0) { exit 1 }
