<#
.SYNOPSIS
  Install, update, roll back and uninstall the HealthierYou companion on a Windows CI
  runner, checking every step (slice V1-6).

.DESCRIPTION
  Runs as the runner's user against real installers built in the same job. It drives the
  product's own paths: the NSIS installer and uninstaller run silently, and updates go
  through the companion's command-line update steps (the same check, staging, helper,
  start confirmation and rollback as Settings > App & updates).

  -Phase Install    install release A and check the files, the uninstall entry (per user,
                    no machine-wide entry) and start at sign-in.
  -Phase Lifecycle  update A -> B; a release that does not start (C, a broken runtime) is
                    rolled back to B; tampered, wrongly signed, renamed, unsigned and
                    same-version files are refused; a downloaded-file mark is detected; B
                    is rolled back to A from its file; uninstall keeps the data and leaves
                    no registry entry or program file; reinstall keeps a start-at-sign-in
                    "off" choice; uninstall with /DELETEDATA removes everything.

  A runner is not an office PC: SmartScreen, Smart App Control and a standard user's
  rights are not exercised here (see docs/V1-6_Install_Update.md, physical tests).
#>
[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)][ValidateSet('Install', 'Lifecycle')][string]$Phase,
  # Folder with A, B and C installers and their .sig files.
  [Parameter(Mandatory = $true)][string]$Installers,
  # Folder with the wrongly-signed copy of B (same name, signed with another key).
  [string]$OtherKeyDir,
  [Parameter(Mandatory = $true)][string]$Work,
  [string]$VersionA = '0.1.0',
  [string]$VersionB = '0.1.1',
  [string]$VersionC = '0.1.2'
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 3

$Product = 'HealthierYou'
$Root = Join-Path $env:LOCALAPPDATA $Product
$Exe = Join-Path $Root 'hy-companion.exe'
$Data = Join-Path $Root 'data'
$Updates = Join-Path $Root 'updates'
$UninstallKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$Product"
$MachineUninstallKey = "HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$Product"
$ManuKey = "HKCU:\Software\$Product"
$RunKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$ApprovedKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run'
$WebViewCache = Join-Path $env:LOCALAPPDATA 'app.healthieryou.companion'
New-Item -ItemType Directory -Force $Work | Out-Null

$script:Results = [System.Collections.Generic.List[object]]::new()
function Check([string]$Name, [bool]$Ok, [string]$Detail = '') {
  $script:Results.Add([pscustomobject]@{ Check = $Name; Ok = $Ok; Detail = $Detail })
  $mark = if ($Ok) { 'PASS' } else { 'FAIL' }
  Write-Host ("[{0}] {1} {2}" -f $mark, $Name, $Detail)
}
function Installer([string]$Version) { Join-Path $Installers ("{0}_{1}_x64-setup.exe" -f $Product, $Version) }
function RegValue([string]$Key, [string]$Name) {
  $p = Get-ItemProperty -Path $Key -Name $Name -ErrorAction SilentlyContinue
  if ($p) { $p.$Name } else { $null }
}
function DisplayVersion { RegValue $UninstallKey 'DisplayVersion' }
function Stop-Companion {
  Get-Process -Name 'hy-companion', 'hy-update-helper' -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
  # The WebView2 processes of the bundled runtime end shortly after the app.
  $deadline = (Get-Date).AddSeconds(30)
  while ((Get-Date) -lt $deadline) {
    $left = @(Get-CimInstance Win32_Process -Filter "Name = 'msedgewebview2.exe'" | Where-Object { $_.ExecutablePath -and $_.ExecutablePath.StartsWith($Root, [StringComparison]::OrdinalIgnoreCase) })
    if ($left.Count -eq 0) { return }
    Start-Sleep -Milliseconds 500
  }
}
# Start a program and wait for it alone. Start-Process -Wait would also wait for every
# process it starts (the update helper, the restarted app), which never all end.
function Start-AndWait([string]$Path, [object]$Arguments, [int]$Minutes = 10) {
  $p = Start-Process -FilePath $Path -ArgumentList $Arguments -PassThru
  $null = $p.Handle  # keep the handle so the exit code is readable
  if (-not $p.WaitForExit($Minutes * 60000)) {
    $p | Stop-Process -Force -ErrorAction SilentlyContinue
    return -1
  }
  $p.ExitCode
}
function Run-Installer([string]$Path, [string]$Arguments = '/S') {
  Start-AndWait $Path $Arguments
}
function Invoke-Uninstall([string]$Arguments = '/S') {
  $u = Join-Path $Root 'uninstall.exe'
  if (-not (Test-Path $u)) { return $false }
  # The NSIS uninstaller copies itself to the temp folder and exits at once; wait for
  # the copy (Au_.exe) to finish and the entry to go, as Windows Settings would.
  $null = Start-AndWait $u $Arguments
  $deadline = (Get-Date).AddMinutes(3)
  while ((Get-Date) -lt $deadline) {
    $running = @(Get-Process -Name 'Au_' -ErrorAction SilentlyContinue).Count -gt 0
    if (-not $running -and -not (Test-Path $UninstallKey) -and -not (Test-Path $Exe)) { return $true }
    Start-Sleep -Milliseconds 500
  }
  $false
}
function Read-State {
  $f = Join-Path $Updates 'state.json'
  if (Test-Path $f) { Get-Content -Raw $f | ConvertFrom-Json } else { $null }
}
# Run a command-line update step; returns @{ Code; Report }.
function Update-Step([string]$Mode, [string]$File) {
  $report = Join-Path $Work ("report-{0}.json" -f [guid]::NewGuid().ToString('N'))
  $code = Start-AndWait $Exe @($Mode, "`"$File`"", '--report', "`"$report`"") 2
  $r = if (Test-Path $report) { Get-Content -Raw $report | ConvertFrom-Json } else { $null }
  @{ Code = $code; Report = $r }
}
# Wait for the helper to record the attempt's outcome.
function Wait-Outcome([string]$To, [int]$Seconds = 300) {
  $deadline = (Get-Date).AddSeconds($Seconds)
  while ((Get-Date) -lt $deadline) {
    $s = Read-State
    if ($s -and $null -eq $s.attempt -and $s.last -and $s.last.to -eq $To) {
      # Let the helper finish its last step (restart and exit).
      $h = Get-Process -Name 'hy-update-helper' -ErrorAction SilentlyContinue
      if ($h) { $h | Wait-Process -Timeout 60 -ErrorAction SilentlyContinue }
      return $s
    }
    Start-Sleep -Seconds 1
  }
  Read-State
}
function Hash([string]$Path) { if (Test-Path $Path) { (Get-FileHash $Path -Algorithm SHA256).Hash } else { '' } }
function Assert-Clean-Registry([string]$When) {
  Check "${When}: no uninstall entry" (-not (Test-Path $UninstallKey))
  Check "${When}: no machine-wide uninstall entry" (-not (Test-Path $MachineUninstallKey))
  Check "${When}: no HKCU\Software\$Product key" (-not (Test-Path $ManuKey))
  Check "${When}: no Run value" ($null -eq (RegValue $RunKey $Product))
  Check "${When}: no StartupApproved value" ($null -eq (RegValue $ApprovedKey $Product))
}
function Assert-No-Program-Files([string]$When) {
  foreach ($e in 'hy-companion.exe', 'uninstall.exe', 'webview2-fixed', 'updates') {
    Check "${When}: $e removed" (-not (Test-Path (Join-Path $Root $e)))
  }
  Check "${When}: WebView2 cache removed" (-not (Test-Path $WebViewCache))
}

try {
  if ($Phase -eq 'Install') {
    Check 'clean runner: not installed before' (-not (Test-Path $UninstallKey) -and -not (Test-Path $Exe))
    $code = Run-Installer (Installer $VersionA)
    Check 'install A: silent installer exit code 0' ($code -eq 0) "exit $code"
    Check 'install A: program in %LOCALAPPDATA%\HealthierYou' (Test-Path $Exe)
    Check 'install A: uninstaller present' (Test-Path (Join-Path $Root 'uninstall.exe'))
    Check 'install A: bundled WebView2 runtime present' (Test-Path (Join-Path $Root 'webview2-fixed\msedgewebview2.exe'))
    Check 'install A: per-user uninstall entry' ((DisplayVersion) -eq $VersionA) "DisplayVersion $(DisplayVersion)"
    Check 'install A: no machine-wide entry (no admin rights used)' (-not (Test-Path $MachineUninstallKey))
    Check 'install A: publisher' ((RegValue $UninstallKey 'Publisher') -eq $Product)
    $run = RegValue $RunKey $Product
    Check 'install A: start at sign-in (per-user Run value)' ($run -eq "`"$Exe`"")
    Check 'install A: exe version' ((Get-Item $Exe).VersionInfo.ProductVersion -like "$VersionA*") (Get-Item $Exe).VersionInfo.ProductVersion
  }
  else {
    # ------------------------------------------------------------ update A -> B
    Stop-Companion
    $exeA = Hash $Exe
    $r = Update-Step '--hy-update-install' (Installer $VersionB)
    Check 'update A->B: accepted and started' ($r.Code -eq 0) "exit $($r.Code) $($r.Report.error)"
    $s = Wait-Outcome $VersionB
    Check 'update A->B: outcome updated' ($s -and $s.last.kind -eq 'updated') "$($s.last.kind): $($s.last.detail)"
    Check 'update A->B: uninstall entry shows B' ((DisplayVersion) -eq $VersionB) "DisplayVersion $(DisplayVersion)"
    Check 'update A->B: exe is B' ((Get-Item $Exe).VersionInfo.ProductVersion -like "$VersionB*" -and (Hash $Exe) -ne $exeA)
    Check 'update A->B: previous version recorded as A' ($s -and $s.previous_version -eq $VersionA) "$($s.previous_version)"
    Check 'update A->B: B is running (confirmed its start)' (@(Get-Process -Name 'hy-companion' -ErrorAction SilentlyContinue).Count -gt 0)
    Check 'update A->B: previous-version copy cleared after success' (-not (Test-Path (Join-Path $Updates 'previous')))
    Check 'update A->B: start at sign-in kept' ((RegValue $RunKey $Product) -eq "`"$Exe`"")
    Check 'update A->B: data folder created by B' (Test-Path (Join-Path $Data 'hy.db'))

    # ------------------------------------------------------ C does not start: roll back
    Stop-Companion
    $exeB = Hash $Exe
    $uninstB = Hash (Join-Path $Root 'uninstall.exe')
    $dbBefore = Hash (Join-Path $Data 'hy.db')
    $r = Update-Step '--hy-update-install' (Installer $VersionC)
    Check 'update B->C: accepted and started' ($r.Code -eq 0) "exit $($r.Code) $($r.Report.error)"
    $s = Wait-Outcome $VersionC
    Check 'update B->C: rolled back' ($s -and $s.last.kind -eq 'rolled_back') "$($s.last.kind): $($s.last.detail)"
    Check 'update B->C: uninstall entry back to B' ((DisplayVersion) -eq $VersionB) "DisplayVersion $(DisplayVersion)"
    Check 'update B->C: exe back to B' ((Hash $Exe) -eq $exeB)
    Check 'update B->C: uninstaller back to B' ((Hash (Join-Path $Root 'uninstall.exe')) -eq $uninstB)
    Check 'update B->C: runtime back' (Test-Path (Join-Path $Root 'webview2-fixed\msedgewebview2.exe'))
    Check 'update B->C: previous version still A' ($s -and $s.previous_version -eq $VersionA) "$($s.previous_version)"
    Check 'update B->C: data untouched by the program copy' ((Hash (Join-Path $Data 'hy.db')) -ne '' -and $dbBefore -ne '')
    Check 'update B->C: B started again' (@(Get-Process -Name 'hy-companion' -ErrorAction SilentlyContinue).Count -gt 0)

    # ------------------------------------------------------------- refused files
    Stop-Companion
    $exeBefore = Hash $Exe
    $tamper = Join-Path $Work 'tampered'; New-Item -ItemType Directory -Force $tamper | Out-Null
    $nameA = Split-Path -Leaf (Installer $VersionA)
    Copy-Item (Installer $VersionA) $tamper; Copy-Item ((Installer $VersionA) + '.sig') $tamper
    $bytes = [IO.File]::ReadAllBytes((Join-Path $tamper $nameA))
    $i = [int]($bytes.Length / 2); $bytes[$i] = $bytes[$i] -bxor 0x5A
    [IO.File]::WriteAllBytes((Join-Path $tamper $nameA), $bytes)
    $r = Update-Step '--hy-update-check' (Join-Path $tamper $nameA)
    Check 'refuse: one changed byte' ($r.Code -eq 2 -and "$($r.Report.error)" -match 'does not match') "$($r.Report.error)"
    if ($OtherKeyDir) {
      $r = Update-Step '--hy-update-check' (Join-Path $OtherKeyDir $nameA)
      Check 'refuse: signed with another key' ($r.Code -eq 2 -and "$($r.Report.error)" -match 'does not trust') "$($r.Report.error)"
    }
    $renamed = Join-Path $Work 'renamed'; New-Item -ItemType Directory -Force $renamed | Out-Null
    $fake = Join-Path $renamed ("{0}_0.9.0_x64-setup.exe" -f $Product)
    Copy-Item (Installer $VersionA) $fake; Copy-Item ((Installer $VersionA) + '.sig') ($fake + '.sig')
    $r = Update-Step '--hy-update-check' $fake
    Check 'refuse: renamed to claim a newer version' ($r.Code -eq 2 -and "$($r.Report.error)" -match 'renamed') "$($r.Report.error)"
    $nosig = Join-Path $Work 'nosig'; New-Item -ItemType Directory -Force $nosig | Out-Null
    Copy-Item (Installer $VersionA) $nosig
    $r = Update-Step '--hy-update-check' (Join-Path $nosig $nameA)
    Check 'refuse: no signature file' ($r.Code -eq 2 -and "$($r.Report.error)" -match 'signature file') "$($r.Report.error)"
    $r = Update-Step '--hy-update-check' (Installer $VersionB)
    Check 'refuse: the installed version' ($r.Code -eq 2 -and "$($r.Report.error)" -match 'already installed') "$($r.Report.error)"
    Check 'refuse: nothing changed' ((Hash $Exe) -eq $exeBefore -and (DisplayVersion) -eq $VersionB)
    # A running app: the command-line install is refused (Settings is the path then).
    $app = Start-Process -FilePath $Exe -PassThru
    Start-Sleep -Seconds 8
    $r = Update-Step '--hy-update-install' (Installer $VersionA)
    Check 'refuse: install while the app runs' ($r.Code -eq 3) "exit $($r.Code)"
    Stop-Companion

    # ------------------------------------------------- the downloaded-file mark is seen
    $marked = Join-Path $Work 'marked'; New-Item -ItemType Directory -Force $marked | Out-Null
    Copy-Item (Installer $VersionA) $marked; Copy-Item ((Installer $VersionA) + '.sig') $marked
    Set-Content -Path (Join-Path $marked $nameA) -Stream 'Zone.Identifier' -Value "[ZoneTransfer]`r`nZoneId=3"
    $r = Update-Step '--hy-update-check' (Join-Path $marked $nameA)
    Check 'mark of the web: detected and reported' ($r.Code -eq 0 -and $r.Report.check.marked_from_internet -eq $true) "exit $($r.Code)"
    Check 'mark of the web: rollback to A offered' ($r.Report.check.plan -eq 'rollback')
    Check 'mark of the web: left on the file (not removed)' ($null -ne (Get-Item (Join-Path $marked $nameA) -Stream 'Zone.Identifier' -ErrorAction SilentlyContinue))

    # ------------------------------------------------------- roll back B -> A by file
    $r = Update-Step '--hy-update-install' (Installer $VersionA)
    Check 'rollback B->A: accepted and started' ($r.Code -eq 0) "exit $($r.Code) $($r.Report.error)"
    $s = Wait-Outcome $VersionA
    Check 'rollback B->A: outcome updated' ($s -and $s.last.kind -eq 'updated') "$($s.last.kind): $($s.last.detail)"
    Check 'rollback B->A: uninstall entry shows A' ((DisplayVersion) -eq $VersionA)
    Check 'rollback B->A: exe is A' ((Get-Item $Exe).VersionInfo.ProductVersion -like "$VersionA*")
    Check 'rollback B->A: nothing older may follow' ($s -and $null -eq $s.previous_version)
    Check 'rollback B->A: data kept' (Test-Path (Join-Path $Data 'hy.db'))

    # --------------------------------------------- uninstall keeps data, no residue
    Stop-Companion
    # As if the user had switched start at sign-in off in Task Manager.
    New-Item -Path $ApprovedKey -Force | Out-Null
    New-ItemProperty -Path $ApprovedKey -Name $Product -PropertyType Binary -Value ([byte[]](3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0)) -Force | Out-Null
    Set-Content -Path (Join-Path $Data 'keep-me.txt') -Value 'personal'
    $ok = Invoke-Uninstall '/S'
    Check 'uninstall: finished' $ok
    Assert-Clean-Registry 'uninstall'
    Assert-No-Program-Files 'uninstall'
    Check 'uninstall: data kept (box unticked by default)' ((Test-Path (Join-Path $Data 'hy.db')) -and (Test-Path (Join-Path $Data 'keep-me.txt')))
    $left = @(Get-ChildItem -Force $Root | ForEach-Object Name)
    Check 'uninstall: only the data folder is left' (($left -join ',') -eq 'data') ($left -join ',')

    # -------------------------------- reinstall keeps the data and an "off" choice
    Set-Content -Path (Join-Path $Data 'autostart-off') -Value 'off'
    $code = Run-Installer (Installer $VersionB)
    Check 'reinstall: exit code 0' ($code -eq 0) "exit $code"
    Check 'reinstall: data still there' (Test-Path (Join-Path $Data 'keep-me.txt'))
    Check 'reinstall: start at sign-in stays off' ($null -eq (RegValue $RunKey $Product))
    Remove-Item (Join-Path $Data 'autostart-off')

    # ------------------------------------------- uninstall with /DELETEDATA: nothing left
    $ok = Invoke-Uninstall '/S /DELETEDATA'
    Check 'uninstall /DELETEDATA: finished' $ok
    Assert-Clean-Registry 'uninstall /DELETEDATA'
    Check 'uninstall /DELETEDATA: %LOCALAPPDATA%\HealthierYou removed' (-not (Test-Path $Root))
    Check 'uninstall /DELETEDATA: WebView2 cache removed' (-not (Test-Path $WebViewCache))
    $rest = @(Get-ChildItem 'HKCU:\Software' -ErrorAction SilentlyContinue | Where-Object { $_.PSChildName -match 'HealthierYou|healthieryou' })
    Check 'uninstall /DELETEDATA: no HKCU\Software key mentions the product' ($rest.Count -eq 0) (($rest | ForEach-Object PSChildName) -join ',')
  }
}
catch {
  Check 'script error' $false $_.Exception.Message
}
finally {
  Stop-Companion
  $log = Join-Path $Updates 'update.log'
  if (Test-Path $log) { Write-Host '--- update.log ---'; Get-Content $log | Write-Host }
}

$failed = @($script:Results | Where-Object { -not $_.Ok })
"{0} checks, {1} failed" -f $script:Results.Count, $failed.Count | Write-Host
$script:Results | Export-Csv -NoTypeInformation -Path (Join-Path $Work "lifecycle-$Phase.csv")
if ($failed.Count -gt 0) { exit 1 }
