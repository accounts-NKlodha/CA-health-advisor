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

  -Phase UninstallLoop  (I9a, docs/V1_Acceptance.md) install A, run it until its WebView2
                    processes are up, uninstall with /S, and check that the WebView2 cache
                    (%LOCALAPPDATA%\app.healthieryou.companion) is gone, -Iterations times.
                    Odd rounds quit the app first (as the lifecycle does), even rounds
                    uninstall while it runs (the uninstaller closes it, as Windows Settings
                    would). Each round's timings and, when the cache is left, what holds it
                    are written to uninstall-loop.csv; it ends with nothing installed.
  -Phase SelfTest   checks this script's own helpers (which processes count as the app's
                    WebView2, the loop summary) on synthetic data; runs anywhere.

  A runner is not an office PC: SmartScreen, Smart App Control and a standard user's
  rights are not exercised here (see docs/V1-6_Install_Update.md, physical tests).
#>
[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)][ValidateSet('Install', 'Lifecycle', 'UninstallLoop', 'SelfTest')][string]$Phase,
  # Folder with A, B and C installers and their .sig files.
  [string]$Installers,
  # Folder with the wrongly-signed copy of B (same name, signed with another key).
  [string]$OtherKeyDir,
  [string]$Work,
  [string]$VersionA = '0.1.0',
  [string]$VersionB = '0.1.1',
  [string]$VersionC = '0.1.2',
  [int]$Iterations = 20
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 3

# The WebView2 user-data folder of this app (WebView2 names it after the bundle id). Its
# path is on the command line of every WebView2 process of the app, whichever runtime copy
# runs it, so that is how they are found (not only by the runtime's path).
$UserDataName = 'app.healthieryou.companion'

# A process is one of this app's WebView2 processes when it is msedgewebview2.exe and runs
# from the install folder or names the app's user-data folder on its command line.
function Test-AppWebView2([object]$Process, [string]$InstallRoot) {
  if ("$($Process.Name)" -ne 'msedgewebview2.exe') { return $false }
  $path = "$($Process.ExecutablePath)"
  if ($InstallRoot -and $path -and $path.StartsWith($InstallRoot.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase)) { return $true }
  $cmd = "$($Process.CommandLine)"
  $cmd.IndexOf("\$UserDataName", [StringComparison]::OrdinalIgnoreCase) -ge 0
}

# True for the NSIS uninstaller's temp copy working on this install: Un.exe (NSIS 3) or
# Au_.exe (NSIS 2) started with "_?=<install folder>", or, when the command line cannot be
# read, run from a ~nsu*.tmp folder.
function Test-UninstallerCopy([object]$Process, [string]$InstallRoot) {
  if (@('Un.exe', 'Au_.exe') -notcontains "$($Process.Name)") { return $false }
  $cmd = "$($Process.CommandLine)"
  if ($cmd) {
    $root = $InstallRoot.TrimEnd('\')
    $i = $cmd.IndexOf('_?=', [StringComparison]::OrdinalIgnoreCase)
    if ($i -lt 0) { return $false }
    $dir = $cmd.Substring($i + 3).Trim().Trim('"').TrimEnd('\')
    return [string]::Equals($dir, $root, [StringComparison]::OrdinalIgnoreCase)
  }
  "$($Process.ExecutablePath)" -match '\\~nsu[0-9A-Fa-f]*\.tmp\\[^\\]+$'
}

# One line per uninstall-loop round, then the verdict: every round must leave no cache.
function Get-LoopSummary([object[]]$Rounds) {
  $left = @($Rounds | Where-Object { $_.cache_left })
  $forced = @($Rounds | Where-Object { $_.webview2_forced })
  $waits = @($Rounds | ForEach-Object { [double]$_.webview2_exit_s })
  $maxWait = if ($waits.Count) { ($waits | Measure-Object -Maximum).Maximum } else { 0 }
  [pscustomobject]@{
    rounds = $Rounds.Count; cache_left = $left.Count; webview2_forced = $forced.Count
    max_webview2_exit_s = $maxWait
    ok = ($Rounds.Count -gt 0 -and $left.Count -eq 0)
  }
}

if ($Phase -eq 'SelfTest') {
  $fails = 0
  function T([string]$Name, [bool]$Ok) {
    if ($Ok) { Write-Host "[PASS] $Name" } else { Write-Host "[FAIL] $Name"; $script:fails++ }
  }
  $root = 'C:\Users\u\AppData\Local\HealthierYou'
  $p = { param($n, $path, $cmd) [pscustomobject]@{ Name = $n; ExecutablePath = $path; CommandLine = $cmd } }
  T 'bundled runtime process counts' (Test-AppWebView2 (& $p 'msedgewebview2.exe' "$root\webview2-fixed\msedgewebview2.exe" '') $root)
  T 'path match is case-insensitive' (Test-AppWebView2 (& $p 'msedgewebview2.exe' "C:\USERS\U\APPDATA\LOCAL\healthieryou\webview2-fixed\msedgewebview2.exe" $null) $root)
  T 'another runtime with this app''s user-data folder counts (crashpad handler)' (Test-AppWebView2 (& $p 'msedgewebview2.exe' 'C:\Program Files (x86)\Microsoft\EdgeWebView\Application\1.0\msedgewebview2.exe' '"msedgewebview2.exe" --type=crashpad-handler --database=C:\Users\u\AppData\Local\app.healthieryou.companion\EBWebView\Crashpad') $root)
  T 'path not readable, command line names the folder: counts' (Test-AppWebView2 (& $p 'msedgewebview2.exe' $null '--user-data-dir="C:\Users\u\AppData\Local\app.healthieryou.companion\EBWebView"') $root)
  T 'another app''s WebView2 does not count' (-not (Test-AppWebView2 (& $p 'msedgewebview2.exe' 'C:\Program Files (x86)\Microsoft\EdgeWebView\Application\1.0\msedgewebview2.exe' '--user-data-dir="C:\Users\u\AppData\Local\Packages\Teams\EBWebView"') $root))
  T 'a folder that only starts like the install folder does not count' (-not (Test-AppWebView2 (& $p 'msedgewebview2.exe' 'C:\Users\u\AppData\Local\HealthierYouOld\msedgewebview2.exe' '') $root))
  T 'a folder that only ends like the user-data name does not count' (-not (Test-AppWebView2 (& $p 'msedgewebview2.exe' 'C:\x\msedgewebview2.exe' '--user-data-dir=C:\x\notapp.healthieryou.companion') $root))
  T 'other process names never count' (-not (Test-AppWebView2 (& $p 'hy-companion.exe' "$root\hy-companion.exe" "--user-data-dir=$UserDataName") $root))
  $r = { param($left, $forced, $wait) [pscustomobject]@{ cache_left = $left; webview2_forced = $forced; webview2_exit_s = $wait } }
  $s = Get-LoopSummary @((& $r $false $false 1.5), (& $r $false $true 31), (& $r $false $false 0))
  T 'loop summary: all rounds clean passes, forced and slowest exit reported' ($s.ok -and $s.rounds -eq 3 -and $s.webview2_forced -eq 1 -and $s.max_webview2_exit_s -eq 31)
  $s = Get-LoopSummary @((& $r $false $false 1), (& $r $true $false 2))
  T 'loop summary: one round with the cache left fails' (-not $s.ok -and $s.cache_left -eq 1)
  T 'loop summary: no rounds is not a pass' (-not (Get-LoopSummary @()).ok)
  $u = { param($n, $path, $cmd) [pscustomobject]@{ Name = $n; ExecutablePath = $path; CommandLine = $cmd } }
  $tmp = 'C:\Users\u\AppData\Local\Temp'
  T 'uninstaller copy (NSIS 3 Un.exe) for this install counts' (Test-UninstallerCopy (& $u 'Un.exe' "$tmp\~nsu1.tmp\Un.exe" "`"$tmp\~nsu1.tmp\Un.exe`" /S _?=$root\") $root)
  T 'uninstaller copy: case-insensitive, NSIS 2 Au_.exe, /DELETEDATA' (Test-UninstallerCopy (& $u 'Au_.exe' "$tmp\~nsu.tmp\Au_.exe" "`"$tmp\~nsu.tmp\Au_.exe`" /S /DELETEDATA _?=C:\USERS\U\APPDATA\LOCAL\HEALTHIERYOU\") $root)
  T 'uninstaller copy: command line not readable, ~nsu folder counts' (Test-UninstallerCopy (& $u 'Un.exe' "$tmp\~nsuA.tmp\Un.exe" $null) $root)
  T 'another program''s uninstaller copy does not count' (-not (Test-UninstallerCopy (& $u 'Un.exe' "$tmp\~nsu2.tmp\Un.exe" "`"$tmp\~nsu2.tmp\Un.exe`" /S _?=C:\Program Files\Other\") $root))
  T 'a folder that only starts like the install folder does not count' (-not (Test-UninstallerCopy (& $u 'Un.exe' "$tmp\~nsu1.tmp\Un.exe" "`"x`" _?=${root}Old\") $root))
  T 'other processes named Un.exe elsewhere do not count' (-not (Test-UninstallerCopy (& $u 'Un.exe' 'C:\tools\Un.exe' $null) $root))
  T 'the original uninstall.exe does not count' (-not (Test-UninstallerCopy (& $u 'uninstall.exe' "$root\uninstall.exe" "`"$root\uninstall.exe`" /S") $root))
  Write-Host "self-test: $fails failed"
  exit $(if ($fails -eq 0) { 0 } else { 1 })
}
if (-not $Installers -or -not $Work) { throw "-Installers and -Work are required for -Phase $Phase" }

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
$WebViewCache = Join-Path $env:LOCALAPPDATA $UserDataName
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
function Get-AppWebView2 {
  @(Get-CimInstance Win32_Process -Filter "Name = 'msedgewebview2.exe'" -ErrorAction SilentlyContinue | Where-Object { Test-AppWebView2 $_ $Root })
}
function Format-Processes([object[]]$Processes) {
  ($Processes | ForEach-Object {
      $cmd = "$($_.CommandLine)"; if ($cmd.Length -gt 220) { $cmd = $cmd.Substring(0, 220) + '...' }
      "pid $($_.ProcessId) parent $($_.ParentProcessId) $($_.ExecutablePath) :: $cmd"
    }) -join ' | '
}
# Quit the app and wait for every one of its WebView2 processes to end (they end on their
# own shortly after the app). Waits at most -Seconds; then names the ones left, closes
# them, and waits again. Returns what happened, for the loop's log.
function Stop-Companion([int]$Seconds = 60) {
  Get-Process -Name 'hy-companion', 'hy-update-helper' -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
  $start = Get-Date
  $deadline = $start.AddSeconds($Seconds)
  $left = @(Get-AppWebView2)
  while ($left.Count -gt 0 -and (Get-Date) -lt $deadline) {
    Start-Sleep -Milliseconds 500
    $left = @(Get-AppWebView2)
  }
  $result = [pscustomobject]@{ exit_s = [math]::Round(((Get-Date) - $start).TotalSeconds, 1); forced = $false; holders = '' }
  if ($left.Count -gt 0) {
    $result.forced = $true
    $result.holders = Format-Processes $left
    Write-Host "::warning title=Lifecycle::$($left.Count) WebView2 process(es) of the app still running $Seconds s after it quit; closing them: $($result.holders)"
    $left | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
    $deadline = (Get-Date).AddSeconds(15)
    while (@(Get-AppWebView2).Count -gt 0 -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 500 }
    $still = @(Get-AppWebView2)
    if ($still.Count -gt 0) { Write-Host "::error title=Lifecycle::WebView2 processes of the app could not be closed: $(Format-Processes $still)" }
  }
  $result
}
# When the WebView2 cache is still there after an uninstall: what holds it (processes that
# name it, files that cannot be opened for exclusive access, and Sysinternals handle.exe's
# view if it is on the PATH), then how long it takes to go once nothing is touched
# (polled every 500 ms for at most 60 s; the folder is removed by hand if it frees up, to
# see whether a retry would have succeeded).
function Trace-CacheHolders {
  if (-not (Test-Path $WebViewCache)) { return $null }
  $procs = @(Get-AppWebView2)
  $files = @(Get-ChildItem -Recurse -Force -File $WebViewCache -ErrorAction SilentlyContinue)
  $locked = @()
  foreach ($f in $files) {
    try { $s = [IO.File]::Open($f.FullName, 'Open', 'ReadWrite', 'None'); $s.Close() }
    catch { $locked += $f.FullName.Substring($WebViewCache.Length + 1) }
  }
  $handle = ''
  $h = Get-Command handle64.exe, handle.exe -ErrorAction SilentlyContinue | Select-Object -First 1
  if ($h) { $handle = ((& $h.Source -accepteula -nobanner $WebViewCache 2>&1) | Out-String).Trim() }
  $start = Get-Date
  $goneAfter = $null
  while (((Get-Date) - $start).TotalSeconds -lt 60) {
    Remove-Item -Recurse -Force $WebViewCache -ErrorAction SilentlyContinue
    if (-not (Test-Path $WebViewCache)) { $goneAfter = [math]::Round(((Get-Date) - $start).TotalSeconds, 1); break }
    Start-Sleep -Milliseconds 500
  }
  $t = [pscustomobject]@{
    processes = Format-Processes $procs
    files_left = $files.Count
    locked = ($locked | Select-Object -First 10) -join ', '
    handle = $handle
    gone_after_s = $goneAfter
  }
  Write-Host "::warning title=Lifecycle::WebView2 cache left after uninstall: $($t.files_left) file(s), locked: [$($t.locked)], processes: [$($t.processes)], removable after: $(if ($null -ne $goneAfter) { "$goneAfter s" } else { 'not within 60 s' })$(if ($handle) { "; handle.exe: $($handle -replace '[\r\n]+', ' ')" })"
  $t
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
  # The NSIS uninstaller copies itself to %TEMP%\~nsu<N>.tmp\Un.exe (Au_.exe before NSIS
  # 3), starts the copy with "_?=<install folder>" and exits at once; the copy does the
  # work. Wait for the copy to end, as Windows Settings would: the uninstall entry and the
  # program go early in the copy's run, before the installer hooks (the WebView2 wait
  # and the cache removal), so they alone do not say it has finished (I9a). The install
  # key HKCU\Software\HealthierYou, removed by the hook after the cache, must be gone too.
  $null = Start-AndWait $u $Arguments
  $seen = $false
  $deadline = (Get-Date).AddMinutes(3)
  while ((Get-Date) -lt $deadline) {
    $copies = @(Get-CimInstance Win32_Process -Filter "Name = 'Un.exe' OR Name = 'Au_.exe'" -ErrorAction SilentlyContinue | Where-Object { Test-UninstallerCopy $_ $Root })
    if ($copies.Count -gt 0) { $seen = $true }
    if ($copies.Count -eq 0 -and -not (Test-Path $UninstallKey) -and -not (Test-Path $Exe) -and -not (Test-Path $ManuKey)) {
      if (-not $seen) { Write-Host "::warning title=Lifecycle::the uninstaller's temp copy was never seen running; finished judged by the registry and files only" }
      return $true
    }
    Start-Sleep -Milliseconds 250
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
  foreach ($e in 'hy-companion.exe', 'uninstall.exe', 'webview2-fixed', 'updates', 'logs') {
    Check "${When}: $e removed" (-not (Test-Path (Join-Path $Root $e)))
  }
  $null = Assert-Cache-Removed $When
}
# The I9a check, with what held the cache when it is left.
function Assert-Cache-Removed([string]$When) {
  $gone = -not (Test-Path $WebViewCache)
  $detail = ''
  if (-not $gone) {
    $t = Trace-CacheHolders
    $detail = "files $($t.files_left); locked [$($t.locked)]; processes [$($t.processes)]; removable after $(if ($null -ne $t.gone_after_s) { "$($t.gone_after_s) s" } else { 'more than 60 s' })"
  }
  Check "${When}: WebView2 cache removed" $gone $detail
  $gone
}

try {
  if ($Phase -eq 'UninstallLoop') {
    $rounds = [System.Collections.Generic.List[object]]::new()
    for ($i = 1; $i -le $Iterations; $i++) {
      $quitFirst = ($i % 2) -eq 1
      $t0 = Get-Date
      $code = Run-Installer (Installer $VersionA)
      $installS = [math]::Round(((Get-Date) - $t0).TotalSeconds, 1)
      Check "round ${i}: install A exit code 0" ($code -eq 0 -and (Test-Path $Exe)) "exit $code"
      # Run the app until its WebView2 processes are up and have had time to write the cache.
      $null = Start-Process -FilePath $Exe -PassThru
      $deadline = (Get-Date).AddSeconds(60)
      while (@(Get-AppWebView2).Count -eq 0 -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 500 }
      $procs = @(Get-AppWebView2).Count
      Start-Sleep -Seconds 5
      Check "round ${i}: the app's WebView2 started and made its cache" ($procs -gt 0 -and (Test-Path $WebViewCache)) "$procs process(es)"
      $stop = [pscustomobject]@{ exit_s = 0; forced = $false; holders = '' }
      if ($quitFirst) { $stop = Stop-Companion }
      $t0 = Get-Date
      $ok = Invoke-Uninstall '/S'
      $uninstallS = [math]::Round(((Get-Date) - $t0).TotalSeconds, 1)
      Check "round ${i}: uninstall finished ($(if ($quitFirst) { 'app quit first' } else { 'app running' }))" $ok "$uninstallS s"
      $leftNow = @(Get-AppWebView2)
      $trace = $null
      $gone = -not (Test-Path $WebViewCache)
      if (-not $gone) { $trace = Trace-CacheHolders }
      Check "round ${i}: WebView2 cache removed" $gone $(if ($trace) { "locked [$($trace.locked)] processes [$($trace.processes)]" } else { '' })
      Check "round ${i}: no WebView2 process of the app left" ($leftNow.Count -eq 0) (Format-Processes $leftNow)
      $rounds.Add([pscustomobject]@{
          round = $i; mode = $(if ($quitFirst) { 'quit_first' } else { 'running' })
          install_s = $installS; webview2_processes = $procs
          webview2_exit_s = $stop.exit_s; webview2_forced = $stop.forced; holders_at_stop = $stop.holders
          uninstall_s = $uninstallS; cache_left = (-not $gone)
          cache_files_left = $(if ($trace) { $trace.files_left } else { 0 })
          cache_locked = $(if ($trace) { $trace.locked } else { '' })
          cache_processes = $(if ($trace) { $trace.processes } else { '' })
          cache_gone_after_s = $(if ($trace) { $trace.gone_after_s } else { $null })
          handle = $(if ($trace) { $trace.handle } else { '' })
        })
      # Start the next round from nothing: the uninstall keeps the data folder.
      $null = Stop-Companion
      Remove-Item -Recurse -Force $Root -ErrorAction SilentlyContinue
      Remove-Item -Recurse -Force $WebViewCache -ErrorAction SilentlyContinue
    }
    $rounds | Export-Csv -NoTypeInformation -Path (Join-Path $Work 'uninstall-loop.csv')
    $rounds | Format-Table round, mode, install_s, webview2_processes, webview2_exit_s, webview2_forced, uninstall_s, cache_left, cache_gone_after_s -AutoSize | Out-String -Width 200 | Write-Host
    $sum = Get-LoopSummary $rounds.ToArray()
    Check "uninstall loop: the cache was removed in every one of $($sum.rounds) rounds" $sum.ok "left in $($sum.cache_left); WebView2 closed by force after the wait in $($sum.webview2_forced); slowest WebView2 exit $($sum.max_webview2_exit_s) s"
  }
  elseif ($Phase -eq 'Install') {
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
    $null = Stop-Companion
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
    $null = Stop-Companion
    $exeB = Hash $Exe
    $uninstB = Hash (Join-Path $Root 'uninstall.exe')
    $r = Update-Step '--hy-update-install' (Installer $VersionC)
    Check 'update B->C: accepted and started' ($r.Code -eq 0) "exit $($r.Code) $($r.Report.error)"
    $s = Wait-Outcome $VersionC
    Check 'update B->C: rolled back' ($s -and $s.last.kind -eq 'rolled_back') "$($s.last.kind): $($s.last.detail)"
    Check 'update B->C: uninstall entry back to B' ((DisplayVersion) -eq $VersionB) "DisplayVersion $(DisplayVersion)"
    Check 'update B->C: exe back to B' ((Hash $Exe) -eq $exeB)
    Check 'update B->C: uninstaller back to B' ((Hash (Join-Path $Root 'uninstall.exe')) -eq $uninstB)
    Check 'update B->C: runtime back' (Test-Path (Join-Path $Root 'webview2-fixed\msedgewebview2.exe'))
    Check 'update B->C: previous version still A' ($s -and $s.previous_version -eq $VersionA) "$($s.previous_version)"
    # (B is running again and holds the database open, so it is not hashed here.)
    Check 'update B->C: data kept' ((Test-Path (Join-Path $Data 'hy.db')) -and (Get-Item (Join-Path $Data 'hy.db')).Length -gt 0)
    Check 'update B->C: B started again' (@(Get-Process -Name 'hy-companion' -ErrorAction SilentlyContinue).Count -gt 0)

    # ------------------------------------------------------------- refused files
    $null = Stop-Companion
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
    $null = Stop-Companion

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
    $null = Stop-Companion
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
    $null = Assert-Cache-Removed 'uninstall /DELETEDATA'
    $rest = @(Get-ChildItem 'HKCU:\Software' -ErrorAction SilentlyContinue | Where-Object { $_.PSChildName -match 'HealthierYou|healthieryou' })
    Check 'uninstall /DELETEDATA: no HKCU\Software key mentions the product' ($rest.Count -eq 0) (($rest | ForEach-Object PSChildName) -join ',')
  }
}
catch {
  Check 'script error' $false $_.Exception.Message
}
finally {
  $null = Stop-Companion
  $log = Join-Path $Updates 'update.log'
  if (Test-Path $log) { Write-Host '--- update.log ---'; Get-Content $log | Write-Host }
}

$failed = @($script:Results | Where-Object { -not $_.Ok })
"{0} checks, {1} failed" -f $script:Results.Count, $failed.Count | Write-Host
$script:Results | Export-Csv -NoTypeInformation -Path (Join-Path $Work "lifecycle-$Phase.csv")
if ($failed.Count -gt 0) { exit 1 }
