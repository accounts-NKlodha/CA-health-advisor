<#
.SYNOPSIS
  The HealthierYou camera soak (slice V2-5; architecture 4.5, 11 "Soak" and assumption A2;
  PRD V2 performance and camera-off requirements): the posture check left capturing for
  hours, sampled from outside and from the app's own camera log, with a PASS, FAIL or
  INCONCLUSIVE verdict and a report safe to share.

.DESCRIPTION
  Office PC run (the real soak; the owner runs it, one command):

    .\Test-CameraSoak.ps1 -Report .\hy-camera-soak.txt

  Run it in Windows PowerShell 5.1 or PowerShell 7 as the office user (no administrator),
  with HealthierYou installed. It asks you to quit HealthierYou from its tray icon, then
  starts it again with its camera log switched on (HY_CAMERA_SOAK_LOG: one line of numbers
  every 5 s, written by the app itself), measures two minutes with the camera off, and
  asks you to turn the posture check on and work as usual. Lock the PC at least once (at
  lunch, or Windows+L for a minute) and, if you can, have one call or slide show: those
  are the holds the soak must see release the camera. It ends after -Hours (2) of camera
  time (holds, sleep and conflicts do not count; at most twice that plus 30 minutes in
  all), asks you to turn the camera off and whether the webcam light went out, and writes
  the report. Run it once with the PC's own webcam and once with a USB (second) webcam,
  on Windows 10 and on Windows 11 (-Webcam names which one; it is asked if missing).

  Every -SampleSeconds (30) it records, for the companion's whole process tree (the app
  and its WebView2 processes): working set, private bytes, handles, threads, CPU as a
  share of the whole machine, whether the session was locked, and how many TCP
  connections the tree holds to loopback, to the LAN (the office server) and to any
  internet address (counts only, never an address). The camera log gives the status
  (reading, no reading, held and why, off, unavailable and why), the frame rate in force,
  frames read, watchdog stalls and worker restarts, and every release of the camera with
  how long it took.

  At the end it checks:

    - the same companion process throughout (no crash or restart);
    - at least -Hours of camera time: the camera reading (reading or no reading), not
      starting or stalled (else INCONCLUSIVE: the run could not prove it);
    - process samples covering the camera time (at least 80% of those expected, and one
      in every 15 minutes of use); without them the budget and trend checks are
      INCONCLUSIVE, never PASS;
    - the posture check was never stopped as "not responding" (two stalls);
    - the frame-rate budget held: each 5 s interval delivered the rate the governor set
      (median at least 80%, the 5th percentile at least 50%) and never above 3 a second;
    - watchdog stalls at most 1 per hour of camera time;
    - every release of the camera (holds, the final Off) confirmed by the page within
      2 s, none forced; at least one release seen (else INCONCLUSIVE);
    - every hold (lock, sleep, meeting, presentation, off hours) closed the camera page;
      at least one hold seen (else INCONCLUSIVE);
    - PRD budgets while capturing: process-tree working set at most 350 MB at every
      sample (the highest counts; the median is reported) and CPU
      at most 10% of the machine on average;
    - no growth trend in working set, private bytes, handles or threads while capturing
      (the last third against the first, and the fitted slope, as Test-Soak.ps1);
    - no TCP connection to an internet address by the tree at any sample (the office
      server on the LAN is allowed; what crosses to it is Test-NetworkCapture.ps1's job);
    - the final Off released the camera within 2 s, and you saw the light go out.

  A check whose evidence is missing is INCONCLUSIVE, never PASS. Exit code 0 PASS, 1 FAIL,
  2 INCONCLUSIVE.

  CI run (the Windows job; shortened, on Chromium's synthetic camera):

    -CiSelfTest -Minutes 10   starts the installed companion with HY_SELFTEST,
                              HY_SELFTEST_CAMERA_SOAK_S and the camera log: after its
                              self-test the app keeps the camera capturing for the given
                              minutes, releasing it every 30 s or so in turn by lock,
                              presentation, sleep, meeting and Off, and checks each release
                              and resume itself; this script samples the tree and reads
                              the log with the same analysis as the office run (budgets
                              informational on a runner; no stall allowed).
    -SelfTest                 checks this script's analysis on synthetic series.

  The report (text, plus JSON with every sample and log line) holds no user name, PC
  name, path, address, device id, frame or posture number. It records the Windows build,
  processor and memory and the webcam kind you name.

.EXAMPLE
  .\Test-CameraSoak.ps1 -Report .\hy-camera-soak.txt
.EXAMPLE
  .\Test-CameraSoak.ps1 -Hours 2 -Webcam usb -Report .\hy-camera-soak-usb.txt
#>
[CmdletBinding()]
param(
  [double]$Hours = 2,
  [int]$SampleSeconds = 30,
  [ValidateSet('', 'built-in', 'usb', 'other')][string]$Webcam = '',
  [switch]$CiSelfTest,
  [double]$Minutes = 10,
  [switch]$SelfTest,
  [string]$Report
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2
# On a CI runner the job log may be unreadable from outside: name the error in an annotation.
trap {
  if ($env:GITHUB_ACTIONS) { Write-Host ("::error title=Camera soak script error::{0} (line {1}: {2})" -f ($_.Exception.Message -replace '[\r\n]+', ' '), $_.InvocationInfo.ScriptLineNumber, ($_.InvocationInfo.Line.Trim() -replace '[\r\n]+', ' ')) }
  break
}

# Budgets and bounds (PRD "Performance": V2 combined working set at most 350 MB during
# check-ins, at most 10% CPU averaged during capture; architecture 4.5: off within 2 s,
# 2 to 3 fps). Trend tolerances as Test-Soak.ps1.
$Budget = @{
  MemoryMB       = 350
  CpuPct         = 10.0
  StopMs         = 2000
  StallsPerHour  = 1.0
  RateMedian     = 0.8
  RateLow        = 0.5
  FpsCap         = 3
  MinIntervals   = 5
  MinLogLines    = 10
  SampleDrift    = 1.25  # a sample's own time plus scheduler drift, on the configured interval
}
$Trend = @{
  MemoryMB = @{ Abs = 20; Rel = 0.10 }
  Handles  = @{ Abs = 200; Rel = 0.15 }
  Threads  = @{ Abs = 15; Rel = 0.20 }
}
# The camera page is in use (memory and CPU are sampled then) ...
$Capturing = @('starting', 'reading', 'no_reading')
# ... and the camera is actually reading: only this counts as camera time (a start or a
# stalled worker does not).
function Test-Reading($Line) {
  return (@('reading', 'no_reading') -contains $Line.kind) -and $Line.detail -ne 'starting' -and $Line.detail -ne 'stalled'
}

# ------------------------------------------------------------------- analysis

function Get-Median([double[]]$v) {
  if (-not $v -or $v.Count -eq 0) { return [double]::NaN }
  $s = $v | Sort-Object
  $n = $s.Count
  if ($n % 2) { return [double]$s[($n - 1) / 2] }
  return ([double]$s[$n / 2 - 1] + [double]$s[$n / 2]) / 2
}

function Get-Percentile([double[]]$v, [double]$p) {
  if (-not $v -or $v.Count -eq 0) { return [double]::NaN }
  $s = @($v | Sort-Object)
  $i = [int][Math]::Min($s.Count - 1, [Math]::Max(0, [Math]::Floor($p * ($s.Count - 1))))
  return [double]$s[$i]
}

# Growth of one series (as Test-Soak.ps1): fails only if the last third's median is past
# the tolerance above the first third's AND the fitted slope adds up to it.
function Test-Trend([string]$Name, [double[]]$Hours, [double[]]$Values, [double]$Abs, [double]$Rel) {
  $n = $Values.Count
  if ($n -lt 9) { return [pscustomobject]@{ Name = $Name; Pass = $null; Detail = "too few samples ($n) for a trend" } }
  $third = [int][Math]::Floor($n / 3)
  $first = Get-Median ($Values[0..($third - 1)])
  $last = Get-Median ($Values[($n - $third)..($n - 1)])
  $mx = ($Hours | Measure-Object -Average).Average
  $my = ($Values | Measure-Object -Average).Average
  $sxy = 0.0; $sxx = 0.0
  for ($i = 0; $i -lt $n; $i++) { $sxy += ($Hours[$i] - $mx) * ($Values[$i] - $my); $sxx += ($Hours[$i] - $mx) * ($Hours[$i] - $mx) }
  $slope = if ($sxx -gt 0) { $sxy / $sxx } else { 0.0 }
  $span = $Hours[$n - 1] - $Hours[0]
  $tol = [Math]::Max($Abs, $Rel * [Math]::Abs($first))
  $growth = $last - $first
  $pass = -not ($growth -gt $tol -and $slope * $span -gt $tol)
  return [pscustomobject]@{
    Name = $Name; Pass = $pass
    Detail = ('first third {0:N1}, last third {1:N1}, growth {2:N1} (tolerance {3:N1}); slope {4:N2}/h over {5:N2} h' -f $first, $last, $growth, $tol, $slope, $span)
  }
}

# A remote address's class, by its text: loopback, unspecified, lan (private, link-local or
# unique-local) or internet. Only the class is ever reported.
function Get-AddressClass([string]$Address) {
  $a = $Address.Trim().ToLowerInvariant()
  if ($a.StartsWith('::ffff:') -and $a.Contains('.')) { $a = $a.Substring(7) }
  if ($a -eq '' -or $a -eq '0.0.0.0' -or $a -eq '::') { return 'unspecified' }
  if ($a -eq '::1' -or $a.StartsWith('127.')) { return 'loopback' }
  if ($a.Contains(':')) {
    if ($a -match '^fe[89ab][0-9a-f]:' -or $a -match '^f[cd][0-9a-f]{2}:') { return 'lan' }
    return 'internet'
  }
  $o = @($a.Split('.') | ForEach-Object { [int]$_ })
  if ($o.Count -ne 4) { return 'internet' }
  if ($o[0] -eq 10) { return 'lan' }
  if ($o[0] -eq 172 -and $o[1] -ge 16 -and $o[1] -le 31) { return 'lan' }
  if ($o[0] -eq 192 -and $o[1] -eq 168) { return 'lan' }
  if ($o[0] -eq 169 -and $o[1] -eq 254) { return 'lan' }
  return 'internet'
}

# A time from the app's JSON: RFC 3339 text (Windows PowerShell 5.1) or a DateTime
# (PowerShell 7 converts ISO text itself).
function ConvertTo-UtcTime($v) {
  if ($v -is [datetime]) { return $v.ToUniversalTime() }
  # .NET reads at most 7 decimal places of a second; the app may write 9.
  $text = [regex]::Replace([string]$v, '(\.\d{7})\d+', '$1')
  return [DateTimeOffset]::Parse($text, [Globalization.CultureInfo]::InvariantCulture).UtcDateTime
}

function Get-Field($o, [string]$Name) {
  if ($null -eq $o) { return $null }
  if ($o -is [System.Collections.IDictionary]) { if ($o.Contains($Name)) { return $o[$Name] } else { return $null } }
  $p = $o.PSObject.Properties[$Name]
  if ($p) { return $p.Value }
  return $null
}

# The camera log's lines as objects with a time in seconds since $Origin (UTC), in order;
# lines that do not parse are counted, not guessed.
function ConvertFrom-CameraLog([object[]]$Raw, [datetime]$Origin) {
  $out = New-Object System.Collections.Generic.List[object]
  $bad = 0
  foreach ($r in $Raw) {
    $o = $r
    if ($r -is [string]) {
      if (-not $r.Trim()) { continue }
      try { $o = $r | ConvertFrom-Json } catch { $bad++; continue }
    }
    $utc = Get-Field $o 'utc'
    $kind = Get-Field $o 'kind'
    if (-not $utc -or -not $kind) { $bad++; continue }
    try { $t = ((ConvertTo-UtcTime $utc) - $Origin).TotalSeconds } catch { $bad++; continue }
    $out.Add([pscustomobject]@{
        t = $t; kind = [string]$kind; detail = [string](Get-Field $o 'detail')
        on = [bool](Get-Field $o 'on'); capture = [bool](Get-Field $o 'capture'); page_open = [bool](Get-Field $o 'page_open')
        fps = [double](Get-Field $o 'fps'); frames = [double](Get-Field $o 'frames'); valid_frames = [double](Get-Field $o 'valid_frames')
        stalls = Get-Field $o 'stalls'; worker_restarts = Get-Field $o 'worker_restarts'
        stops = Get-Field $o 'stops'; forced_stops = Get-Field $o 'forced_stops'
        max_stop_ms = Get-Field $o 'max_stop_ms'; last_stop_ms = Get-Field $o 'last_stop_ms'
        inference_ms = Get-Field $o 'inference_ms'
      })
  }
  return [pscustomobject]@{ Lines = @($out | Sort-Object t); Bad = $bad }
}

function New-Check([string]$Name, [string]$Verdict, [string]$Detail, [bool]$Required = $true) {
  return [pscustomobject]@{ Name = $Name; Verdict = $Verdict; Required = $Required; Detail = $Detail }
}
function Get-Verdict($Pass) { if ($null -eq $Pass) { 'INCONCLUSIVE' } elseif ($Pass) { 'PASS' } else { 'FAIL' } }

function Get-OverallVerdict($Checks) {
  $req = @($Checks | Where-Object { $_.Required })
  if ($req.Count -eq 0) { return 'INCONCLUSIVE' }
  if (@($req | Where-Object { $_.Verdict -eq 'FAIL' }).Count) { return 'FAIL' }
  if (@($req | Where-Object { $_.Verdict -ne 'PASS' }).Count) { return 'INCONCLUSIVE' }
  return 'PASS'
}

# The analysis shared by the office run, the CI run and the self-test.
#   $Lines    camera log lines (ConvertFrom-CameraLog), times in seconds.
#   $Samples  process samples: t (seconds, same origin), alive, working_set_mb,
#             private_bytes_mb, handles, threads, cpu_pct, tcp_ok, tcp_internet, tcp_lan.
#   $Opt      NeedCaptureS; StallsPerHour (or StallsAllowed); BudgetsRequired; NeedHold;
#             WarmupFraction; SampleSeconds (the sampler's interval); FailedSamples.
function Invoke-CameraAnalysis($Lines, $Samples, [hashtable]$Opt) {
  $checks = New-Object System.Collections.Generic.List[object]
  $L = @(foreach ($x in $Lines) { $x })
  $S = @(foreach ($x in $Samples) { $x })
  $facts = [ordered]@{}

  # Evidence: the log.
  if ($L.Count -lt $Budget.MinLogLines) {
    $checks.Add((New-Check 'camera log read' 'INCONCLUSIVE' "$($L.Count) lines (at least $($Budget.MinLogLines) needed): the app wrote no camera log, or the run was too short"))
  }
  else { $checks.Add((New-Check 'camera log read' 'PASS' "$($L.Count) lines over $([Math]::Round(($L[$L.Count - 1].t - $L[0].t) / 60, 1)) min")) }

  # No crash or restart.
  if ($S.Count -eq 0) { $checks.Add((New-Check 'no crash or restart' 'INCONCLUSIVE' 'no process samples')) }
  else {
    $dead = @($S | Where-Object { -not $_.alive })
    $checks.Add((New-Check 'no crash or restart' $(if ($dead.Count) { 'FAIL' } else { 'PASS' }) $(if ($dead.Count) { "the main process was gone from $([Math]::Round($dead[0].t)) s" } else { "$($S.Count) samples, same main process throughout" })))
  }

  # Intervals between log lines: a gap of more than 3 intervals is sleep or a suspended
  # app, not camera time.
  $dts = @(for ($i = 1; $i -lt $L.Count; $i++) { $L[$i].t - $L[$i - 1].t })
  $every = if ($dts.Count) { Get-Median ([double[]]$dts) } else { 5.0 }
  $gapLimit = [Math]::Max(3 * $every, $every + 10)
  $captureS = 0.0; $gaps = 0; $inUseS = 0.0; $binUse = @{}
  $rates = New-Object System.Collections.Generic.List[double]
  $ratios = New-Object System.Collections.Generic.List[double]
  $lowRateS = 0.0
  for ($i = 1; $i -lt $L.Count; $i++) {
    $a = $L[$i - 1]; $b = $L[$i]; $dt = $b.t - $a.t
    if ($dt -gt $gapLimit) { $gaps++; continue }
    if ((Test-Reading $a) -and (Test-Reading $b)) { $captureS += $dt }
    if ($Capturing -contains $a.kind -and $Capturing -contains $b.kind) {
      $inUseS += $dt
      $bin = [int][Math]::Floor($a.t / 900)
      $binUse[$bin] = $dt + [double]$binUse[$bin]
    }
    $steady = @('reading', 'no_reading')
    if ($steady -contains $a.kind -and $steady -contains $b.kind -and $a.detail -ne 'stalled' -and $b.detail -ne 'stalled' -and $a.detail -ne 'starting' -and $a.fps -eq $b.fps -and $a.fps -gt 0 -and $dt -gt 0) {
      $r = ($b.frames - $a.frames) / $dt
      $rates.Add($r); $ratios.Add($r / $a.fps)
      if ($a.fps -lt $Budget.FpsCap) { $lowRateS += $dt }
    }
  }
  $facts['capture_h'] = [Math]::Round($captureS / 3600, 3)
  $facts['log_gaps'] = $gaps
  $need = [double]$Opt.NeedCaptureS
  $checks.Add((New-Check 'camera time covered' $(if ($captureS -ge $need -and $need -gt 0) { 'PASS' } else { 'INCONCLUSIVE' }) ('{0:N2} h reading of {1:N2} h asked (starts and stalls not counted; {2:N2} h with the camera page in use); {3} sleep or suspend gaps in the log' -f ($captureS / 3600), ($need / 3600), ($inUseS / 3600), $gaps)))

  # Never stopped for "not responding"; other conflicts are reported.
  $stopped = @($L | Where-Object { $_.kind -eq 'unavailable' -and $_.detail -eq 'stalled' })
  $other = @($L | Where-Object { $_.kind -eq 'unavailable' -and $_.detail -ne 'stalled' } | ForEach-Object { $_.detail } | Group-Object | ForEach-Object { "$($_.Name) $($_.Count) lines" })
  $checks.Add((New-Check 'never stopped as not responding' $(if ($L.Count -eq 0) { 'INCONCLUSIVE' } elseif ($stopped.Count) { 'FAIL' } else { 'PASS' }) ("$($stopped.Count) log lines stopped for stalls; other unavailable states: $(if ($other) { $other -join ', ' } else { 'none' })")))

  # Frame rate.
  if ($rates.Count -lt $Budget.MinIntervals) {
    $checks.Add((New-Check 'frame-rate budget held' 'INCONCLUSIVE' "$($rates.Count) steady capture intervals (at least $($Budget.MinIntervals) needed)"))
  }
  else {
    $med = Get-Median ([double[]]$ratios); $p5 = Get-Percentile ([double[]]$ratios) 0.05
    $p95 = Get-Percentile ([double[]]$rates) 0.95
    $minR = ($ratios | Measure-Object -Minimum).Minimum
    $ok = $med -ge $Budget.RateMedian -and $p5 -ge $Budget.RateLow -and $p95 -le $Budget.FpsCap * 1.2
    $checks.Add((New-Check 'frame-rate budget held' $(if ($ok) { 'PASS' } else { 'FAIL' }) ('{0} intervals; delivered/set median {1:N2} (at least {2}), 5th percentile {3:N2} (at least {4}), lowest {5:N2}; 95th percentile rate {6:N2} a second (cap {7}); {8:N1}% of steady time at the reduced rate' -f $rates.Count, $med, $Budget.RateMedian, $p5, $Budget.RateLow, $minR, $p95, $Budget.FpsCap, $(if ($captureS -gt 0) { 100 * $lowRateS / $captureS } else { 0 }))))
    $facts['fps_median'] = [Math]::Round((Get-Median ([double[]]$rates)), 2)
  }

  # Counters: first and last line that carry them (older builds without them: missing).
  $withStops = @($L | Where-Object { $null -ne $_.stops -and $null -ne $_.stalls })
  if ($withStops.Count -lt 2) {
    $checks.Add((New-Check 'watchdog stalls bounded' 'INCONCLUSIVE' 'the log has no stall counters (a build from before V2-5?)'))
    $checks.Add((New-Check 'every release within 2 s' 'INCONCLUSIVE' 'the log has no release counters (a build from before V2-5?)'))
  }
  else {
    $f = $withStops[0]; $z = $withStops[$withStops.Count - 1]
    $stalls = [int]$z.stalls - [int]$f.stalls
    $restarts = [int]$z.worker_restarts - [int]$f.worker_restarts
    $allowed = if ($Opt.Contains('StallsAllowed')) { [int]$Opt.StallsAllowed } else { [int][Math]::Floor($Opt.StallsPerHour * $captureS / 3600) }
    $checks.Add((New-Check 'watchdog stalls bounded' $(if ($stalls -le $allowed -and $restarts -le $stalls) { 'PASS' } else { 'FAIL' }) "$stalls stalls and $restarts worker restarts (at most $allowed allowed; a restart only after a stall)"))
    $facts['stalls'] = $stalls

    # Releases: each increase of the stop counter between two lines shows that interval's
    # last stop; a run of more than one between lines shows only its last, but the
    # longest-ever counter catches any slow one.
    $stops = [int]$z.stops - [int]$f.stops
    $forced = [int]$z.forced_stops - [int]$f.forced_stops
    $seen = New-Object System.Collections.Generic.List[double]
    for ($i = 1; $i -lt $withStops.Count; $i++) {
      if ([int]$withStops[$i].stops -gt [int]$withStops[$i - 1].stops -and $null -ne $withStops[$i].last_stop_ms) { $seen.Add([double]$withStops[$i].last_stop_ms) }
    }
    $maxSeen = if ($seen.Count) { ($seen | Measure-Object -Maximum).Maximum } else { 0 }
    $maxF = if ($null -ne $f.max_stop_ms) { [double]$f.max_stop_ms } else { 0 }
    $maxZ = if ($null -ne $z.max_stop_ms) { [double]$z.max_stop_ms } else { 0 }
    $longest = if ($maxZ -gt $maxF) { [Math]::Max($maxZ, $maxSeen) } else { $maxSeen }
    $v = if ($stops -lt 1) { 'INCONCLUSIVE' } elseif ($longest -gt $Budget.StopMs -or $forced -gt 0) { 'FAIL' } else { 'PASS' }
    $checks.Add((New-Check 'every release within 2 s' $v ("$stops releases, $forced not confirmed by the page (forced); longest $longest ms (deadline $($Budget.StopMs) ms)$(if ($stops -lt 1) { '; no release happened, so nothing shows the camera is let go' })")))
    $facts['releases'] = $stops
    $facts['longest_release_ms'] = $longest
  }

  # Holds close the camera page: from the second line of a held run on, no page and no
  # capture.
  $holdRuns = @{}; $badHold = 0
  for ($i = 0; $i -lt $L.Count; $i++) {
    if ($L[$i].kind -ne 'held') { continue }
    $startRun = ($i -eq 0 -or $L[$i - 1].kind -ne 'held' -or $L[$i - 1].detail -ne $L[$i].detail -or ($L[$i].t - $L[$i - 1].t) -gt $gapLimit)
    if ($startRun) { $holdRuns[$L[$i].detail] = 1 + [int]$holdRuns[$L[$i].detail] }
    elseif ($L[$i].page_open -or $L[$i].capture) { $badHold++ }
  }
  $runs = @($holdRuns.Keys | Sort-Object | ForEach-Object { "$_ $($holdRuns[$_])" })
  $hv = if ($badHold) { 'FAIL' } elseif ($holdRuns.Count -eq 0 -and $Opt.NeedHold) { 'INCONCLUSIVE' } else { 'PASS' }
  $checks.Add((New-Check 'holds release the camera' $hv ("holds seen: $(if ($runs) { $runs -join ', ' } else { 'none' }); log lines with the page or capture still on inside a hold: $badHold$(if ($holdRuns.Count -eq 0) { '; lock the PC once for a minute during the run' })")))
  $facts['holds'] = $holdRuns

  # Samples while capturing: the log line at or before the sample and the one after both
  # capturing.
  $cap = New-Object System.Collections.Generic.List[object]
  $j = 0
  foreach ($x in $S) {
    if (-not $x.alive) { continue }
    while ($j + 1 -lt $L.Count -and $L[$j + 1].t -le $x.t) { $j++ }
    if ($L.Count -lt 2 -or $L[$j].t -gt $x.t -or $j + 1 -ge $L.Count) { continue }
    if ($Capturing -contains $L[$j].kind -and $Capturing -contains $L[$j + 1].kind -and ($L[$j + 1].t - $L[$j].t) -le $gapLimit) { $cap.Add($x) }
  }
  $facts['capture_samples'] = $cap.Count
  $req = [bool]$Opt.BudgetsRequired

  # Process evidence must cover the camera time: enough samples for the time the camera page
  # was in use at the CONFIGURED interval (allowing 25% for the time each sample takes and
  # scheduler drift; never the surviving samples' own, possibly sparse, cadence), at least
  # 80% of those, one in every 15 minutes with 5 or more minutes of use, and at most 20% of
  # sampling attempts failed. Without it, the budget and trend checks below are not proven.
  $alive = @($S | Where-Object { $_.alive })
  $pace = [double]$Opt.SampleSeconds * $Budget.SampleDrift
  $expected = if ($pace -gt 0) { $inUseS / $pace } else { 0 }
  $binsHit = @{}
  foreach ($x in $cap) { $binsHit[[int][Math]::Floor($x.t / 900)] = $true }
  $uncovered = @($binUse.Keys | Where-Object { $binUse[$_] -ge 300 -and -not $binsHit.ContainsKey($_) })
  $coverage = if ($expected -gt 0) { $cap.Count / $expected } else { 0 }
  $failedSamples = if ($Opt.Contains('FailedSamples')) { [int]$Opt.FailedSamples } else { 0 }
  $failedShare = if (($failedSamples + $S.Count) -gt 0) { $failedSamples / ($failedSamples + $S.Count) } else { 0 }
  $covered = $expected -gt 0 -and $coverage -ge 0.8 -and $uncovered.Count -eq 0 -and $failedShare -le 0.2
  $checks.Add((New-Check 'process samples cover the camera time' $(if ($covered) { 'PASS' } else { 'INCONCLUSIVE' }) ('{0} samples while the camera page was in use, {1:N0}% of the {2:N0} expected at one every {3:N1} s (the configured {6} s plus 25% drift; at least 80% needed); 15-minute stretches with 5 or more minutes of use and no sample: {4}; failed sampling attempts: {5} ({7:N0}%, at most 20%)' -f $cap.Count, (100 * $coverage), $expected, $pace, $uncovered.Count, $failedSamples, $Opt.SampleSeconds, (100 * $failedShare))))
  $facts['sample_coverage_pct'] = [Math]::Round(100 * $coverage, 1)
  # A check that would PASS on samples that do not cover the run is not proven.
  function Get-Covered([string]$v) { if ($v -eq 'PASS' -and -not $covered) { 'INCONCLUSIVE' } else { $v } }
  if ($cap.Count -lt 3) {
    $checks.Add((New-Check 'PRD budget: memory while capturing' $(if ($req) { 'INCONCLUSIVE' } else { 'INFO' }) "$($cap.Count) samples while capturing" $req))
    $checks.Add((New-Check 'PRD budget: CPU while capturing' $(if ($req) { 'INCONCLUSIVE' } else { 'INFO' }) "$($cap.Count) samples while capturing" $req))
  }
  else {
    $ws = [double[]]@($cap | ForEach-Object { $_.working_set_mb })
    $wsMed = Get-Median $ws; $wsMax = ($ws | Measure-Object -Maximum).Maximum
    # The PRD cap: combined working set at most 350 MB during check-ins, so the highest
    # sample counts (the median is reported).
    $mv = if ($wsMax -gt $Budget.MemoryMB) { if ($req) { 'FAIL' } else { 'INFO' } } elseif ($req) { Get-Covered 'PASS' } else { 'INFO' }
    $checks.Add((New-Check 'PRD budget: memory while capturing' $mv ('process tree working set highest {1:N1} MB (cap {2} MB, companion and inference together), median {0:N1} MB{3}' -f $wsMed, $wsMax, $Budget.MemoryMB, $(if (-not $req) { '; informational: a runner is not an office PC' })) $req))
    $cpuS = [double[]]@($cap | Where-Object { $null -ne $_.cpu_pct } | ForEach-Object { $_.cpu_pct })
    if ($cpuS.Count -lt 3) { $checks.Add((New-Check 'PRD budget: CPU while capturing' $(if ($req) { 'INCONCLUSIVE' } else { 'INFO' }) 'no CPU samples' $req)) }
    else {
      $cpuAvg = ($cpuS | Measure-Object -Average).Average; $cpuMax = ($cpuS | Measure-Object -Maximum).Maximum
      $cv = if (-not $req) { 'INFO' } elseif ($cpuAvg -le $Budget.CpuPct) { Get-Covered 'PASS' } else { 'FAIL' }
      $checks.Add((New-Check 'PRD budget: CPU while capturing' $cv ('process tree average {0:N2}% of the machine, highest sample {1:N2}% (budget {2}% averaged){3}' -f $cpuAvg, $cpuMax, $Budget.CpuPct, $(if (-not $req) { '; informational: a runner is not an office PC' })) $req))
      $facts['cpu_avg_pct'] = [Math]::Round($cpuAvg, 2)
    }
    $facts['ws_median_mb'] = [Math]::Round($wsMed, 1)
    $facts['ws_max_mb'] = [Math]::Round($wsMax, 1)
  }

  # Growth trends while capturing, after the warm-up.
  $skip = [int][Math]::Floor($cap.Count * [double]$Opt.WarmupFraction)
  $w = @($cap | Select-Object -Skip $skip)
  $h = [double[]]@($w | ForEach-Object { $_.t / 3600 })
  foreach ($m in @(@('working set (MB)', 'working_set_mb', 'MemoryMB'), @('private bytes (MB)', 'private_bytes_mb', 'MemoryMB'), @('handles', 'handles', 'Handles'), @('threads', 'threads', 'Threads'))) {
    $vals = [double[]]@($w | ForEach-Object { $_.($m[1]) })
    $t = Test-Trend $m[0] $h $vals $Trend[$m[2]].Abs $Trend[$m[2]].Rel
    $checks.Add((New-Check "no growth trend while capturing: $($t.Name)" (Get-Covered (Get-Verdict $t.Pass)) $t.Detail))
  }

  # Network: counts of TCP connections by class, at every sample.
  $tcp = @($S | Where-Object { $_.alive })
  if ($tcp.Count -eq 0 -or @($tcp | Where-Object { -not $_.tcp_ok }).Count) {
    $checks.Add((New-Check 'no internet connection' 'INCONCLUSIVE' "TCP connections could not be read at $(@($tcp | Where-Object { -not $_.tcp_ok }).Count) of $($tcp.Count) samples"))
  }
  else {
    $inet = @($tcp | Where-Object { $_.tcp_internet -gt 0 })
    $lanMax = ($tcp | ForEach-Object { $_.tcp_lan } | Measure-Object -Maximum).Maximum
    $checks.Add((New-Check 'no internet connection' $(if ($inet.Count) { 'FAIL' } else { 'PASS' }) ("$($tcp.Count) samples of the tree's TCP connections: internet addresses at $($inet.Count) samples; LAN connections at most $lanMax at once (the office server); counts only, sampled every $SampleSeconds s")))
  }
  return [pscustomobject]@{ Checks = $checks; Facts = $facts }
}

# --------------------------------------------------------------------- sampling

function Get-Tree([int[]]$Roots) {
  $all = @(Get-CimInstance Win32_Process -Property ProcessId, ParentProcessId, Name, WorkingSetSize, HandleCount, ThreadCount, PrivatePageCount)
  $ids = New-Object System.Collections.Generic.HashSet[int]
  foreach ($r in $Roots) { [void]$ids.Add($r) }
  $added = $true
  while ($added) {
    $added = $false
    foreach ($p in $all) { if ($ids.Contains([int]$p.ParentProcessId) -and -not $ids.Contains([int]$p.ProcessId) -and $p.ProcessId -ne 0) { [void]$ids.Add([int]$p.ProcessId); $added = $true } }
  }
  return @($all | Where-Object { $ids.Contains([int]$_.ProcessId) })
}

$script:CpuSeen = @{}
$script:LastWall = $null
$script:SampleErrors = 0
$script:InternetSeen = @{}
$script:CiOrigin = (Get-Date).ToUniversalTime()
$Cores = [Environment]::ProcessorCount

function Get-TcpCounts([int[]]$Ids) {
  try { $c = @(Get-NetTCPConnection -ErrorAction Stop | Where-Object { $Ids -contains [int]$_.OwningProcess }) }
  catch { return [pscustomobject]@{ Ok = $false; Internet = 0; Lan = 0; Loopback = 0 } }
  $n = @{ internet = 0; lan = 0; loopback = 0; unspecified = 0 }
  foreach ($x in $c) {
    if ([string]$x.State -eq 'Listen' -or [string]$x.State -eq 'Bound') { continue }
    $class = Get-AddressClass ([string]$x.RemoteAddress)
    $n[$class]++
    # On a CI runner only (no person's data there), name what connected, for diagnosis.
    if ($class -eq 'internet' -and $CiSelfTest) {
      $key = "$($x.OwningProcess)|$($x.RemoteAddress):$($x.RemotePort)"
      $at = [Math]::Round(((Get-Date).ToUniversalTime() - $script:CiOrigin).TotalSeconds)
      if ($script:InternetSeen.Contains($key)) { $script:InternetSeen[$key].last = $at; $script:InternetSeen[$key].n++ }
      else {
        $proc = Get-CimInstance Win32_Process -Filter "ProcessId = $([int]$x.OwningProcess)" -ErrorAction SilentlyContinue
        $type = if ($proc -and $proc.CommandLine -match '--type=(\S+)') { $Matches[1] } else { 'browser' }
        if ($proc -and $proc.CommandLine -match '--utility-sub-type=(\S+)') { $type += " $($Matches[1])" }
        $ptr = try { (Resolve-DnsName -Name ([string]$x.RemoteAddress) -Type PTR -QuickTimeout -ErrorAction Stop | Select-Object -First 1).NameHost } catch { 'no PTR' }
        $script:InternetSeen[$key] = @{ what = "$(if ($proc) { $proc.Name } else { '?' }) ($type) -> $($x.RemoteAddress):$($x.RemotePort) [$ptr] $($x.State)"; first = $at; last = $at; n = 1 }
      }
    }
  }
  return [pscustomobject]@{ Ok = $true; Internet = $n.internet; Lan = $n.lan; Loopback = $n.loopback }
}

function Get-ProcessSample([int]$Root, [datetime]$Origin) {
  $now = Get-Date
  $tree = Get-Tree @($Root)
  $ids = @($tree | ForEach-Object { [int]$_.ProcessId })
  $cpuNow = @{}
  foreach ($id in $ids) {
    $p = Get-Process -Id $id -ErrorAction SilentlyContinue
    if ($p) { try { $cpuNow[$id] = $p.TotalProcessorTime.TotalSeconds } catch { } }
  }
  $delta = 0.0
  foreach ($id in $cpuNow.Keys) {
    if ($script:CpuSeen.ContainsKey($id)) { $delta += [Math]::Max(0, $cpuNow[$id] - $script:CpuSeen[$id]) }
    elseif ($script:LastWall) { $delta += $cpuNow[$id] }
  }
  $cpu = $null
  if ($script:LastWall) {
    $wall = ($now - $script:LastWall).TotalSeconds
    if ($wall -gt 0) { $cpu = [Math]::Round(100 * $delta / ($wall * $Cores), 3) }
  }
  $script:CpuSeen = $cpuNow
  $script:LastWall = $now
  $tcp = Get-TcpCounts $ids
  return [ordered]@{
    t = [Math]::Round(($now.ToUniversalTime() - $Origin).TotalSeconds, 1)
    utc = $now.ToUniversalTime().ToString('o')
    alive = ($ids -contains $Root)
    processes = $tree.Count
    working_set_mb = [Math]::Round((($tree | Measure-Object WorkingSetSize -Sum).Sum) / 1MB, 1)
    private_bytes_mb = [Math]::Round((($tree | Measure-Object PrivatePageCount -Sum).Sum) / 1MB, 1)
    handles = [int](($tree | Measure-Object HandleCount -Sum).Sum)
    threads = [int](($tree | Measure-Object ThreadCount -Sum).Sum)
    cpu_pct = $cpu
    locked = [bool](Get-Process -Name LogonUI -ErrorAction SilentlyContinue)
    tcp_ok = $tcp.Ok; tcp_internet = $tcp.Internet; tcp_lan = $tcp.Lan; tcp_loopback = $tcp.Loopback
  }
}

function Read-CameraLog([string]$Path, [datetime]$Origin) {
  $raw = @()
  if ($Path -and (Test-Path -LiteralPath $Path)) { $raw = @(Get-Content -LiteralPath $Path) }
  return ConvertFrom-CameraLog $raw $Origin
}

# ------------------------------------------------------------------ the report

$Lines = New-Object System.Collections.Generic.List[string]
$AllChecks = New-Object System.Collections.Generic.List[object]
function Add-Line([string]$Text) { $Lines.Add($Text); Write-Host $Text }
function Add-Checks($Checks) {
  foreach ($c in $Checks) {
    $AllChecks.Add($c)
    $tag = if ($c.Required) { $c.Verdict } else { 'INFO' }
    Add-Line ("[{0}] {1} - {2}" -f $tag, $c.Name, $c.Detail)
  }
}

function Write-Environment([string]$Mode) {
  Add-Line "HealthierYou camera soak - $Mode - $((Get-Date).ToString('yyyy-MM-dd HH:mm'))"
  try {
    $os = Get-CimInstance Win32_OperatingSystem
    $cpuName = (Get-CimInstance Win32_Processor | Select-Object -First 1).Name
    $ram = [Math]::Round((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory / 1GB, 1)
    Add-Line ("Windows: {0} build {1}; processor: {2}; {3} logical processors; {4} GB memory" -f $os.Caption, $os.BuildNumber, $cpuName.Trim(), $Cores, $ram)
  }
  catch { Add-Line "Windows details unavailable: $($_.Exception.Message)" }
  Add-Line "Sampling: every $SampleSeconds s, whole process tree (WebView2 included), Win32_Process and Get-NetTCPConnection (counts by class only); the app's camera log every few seconds (status, rate, counters)."
}

function Save-Report($Extra) {
  $overall = Get-OverallVerdict $AllChecks
  Add-Line ''
  if ($script:SampleErrors) { Add-Line "$($script:SampleErrors) process samples failed (a process ended while Windows listed them) and were skipped." }
  Add-Line "Overall: $overall"
  Add-Line 'The report names no user, PC, path, address or device id and holds no frame or posture number. Check it before sharing.'
  if ($Report) {
    $dir = Split-Path -Parent $Report
    if ($dir) { New-Item -ItemType Directory -Force $dir | Out-Null }
    $Lines | Set-Content -Encoding UTF8 $Report
    $o = [ordered]@{ verdict = $overall; checks = $AllChecks }
    foreach ($k in $Extra.Keys) { $o[$k] = $Extra[$k] }
    $o | ConvertTo-Json -Depth 6 | Set-Content -Encoding UTF8 ($Report + '.json')
  }
  return $overall
}
function Exit-With([string]$Verdict) {
  switch ($Verdict) { 'PASS' { exit 0 } 'FAIL' { exit 1 } default { exit 2 } }
}

# The CI run's report, from the samples, the app's report and its camera log (a function so
# the self-test can run it on data shaped as the app writes it).
function Invoke-CiReport($samples, $app, [int]$ExitCode, [bool]$timedOut, [object[]]$LogRaw, [datetime]$origin) {
  $appChecks = New-Object System.Collections.Generic.List[object]
  $appOk = (-not $timedOut) -and $ExitCode -eq 0 -and $app -and $app.ok
  $appChecks.Add((New-Check 'the app''s self-test and camera soak checks' $(if ($appOk) { 'PASS' } else { 'FAIL' }) "exit code $($ExitCode)$(if ($timedOut) { "; still running after $([int]($Minutes * 2 + 15)) minutes" })"))
  $soak = if ($app) { Get-Field $app 'camera_soak' } else { $null }
  if (-not $soak) { $appChecks.Add((New-Check 'the app ran its camera soak' 'FAIL' 'no camera soak summary in the app report')) }
  Add-Checks $appChecks
  if ($app) {
    foreach ($c in @($app.checks | Where-Object { $_.name -like 'camera_soak_*' })) { Add-Line ("       app check {0}: {1} - {2}" -f $c.name, $(if ($c.ok) { 'ok' } else { 'FAILED' }), $c.detail) }
    foreach ($c in @($app.checks | Where-Object { -not $_.ok -and $_.required -and $_.name -notlike 'camera_soak_*' })) { Add-Line ("       failed app check {0}: {1}" -f $c.name, $c.detail) }
  }
  $log = ConvertFrom-CameraLog $LogRaw $origin
  # Ordered dictionaries in a generic list: copied one by one (@() on such a list fails
  # with "Argument types do not match").
  $L = @($log.Lines); $S = @(foreach ($x in $samples) { $x })
  if ($soak) {
    $from = ((ConvertTo-UtcTime $soak.started_utc) - $origin).TotalSeconds
    $to = ((ConvertTo-UtcTime $soak.ended_utc) - $origin).TotalSeconds
    $L = @($L | Where-Object { $_.t -ge $from -and $_.t -le $to })
    $S = @($S | Where-Object { $_.t -ge $from -and $_.t -le $to })
    Add-Line ("Camera soak phase: {0:N1} min; {1} of {2} log lines and {3} of {4} process samples in it; {5} unreadable log lines" -f (($to - $from) / 60), $L.Count, $log.Lines.Count, $S.Count, $samples.Count, $log.Bad)
  }
  # The app holds the camera released for about a quarter of each cycle: half the soak's
  # time must be camera time. No stall is allowed on a synthetic camera.
  $opt = @{ NeedCaptureS = $Minutes * 60 * 0.5; StallsAllowed = 0; BudgetsRequired = $false; NeedHold = $true; WarmupFraction = 0.2; SampleSeconds = $SampleSeconds; FailedSamples = $script:SampleErrors }
  $r = Invoke-CameraAnalysis $L $S $opt
  Add-Checks $r.Checks
  Add-Line ("Numbers: " + (($r.Facts.Keys | ForEach-Object { $v = $r.Facts[$_]; if ($v -is [hashtable]) { $v = (@($v.Keys | Sort-Object | ForEach-Object { "$_ $($v[$_])" }) -join ' ') }; "$_ $v" }) -join '; '))
  if ($env:GITHUB_ACTIONS) {
    # Annotations are readable from the checks API when the job log is not.
    if ($app) { foreach ($c in @($app.checks | Where-Object { $_.name -like 'camera_soak_*' })) { Write-Host ("::notice title=Camera soak (app)::{0} {1}: {2}" -f $(if ($c.ok) { 'ok' } else { 'FAILED' }), $c.name, ($c.detail -replace '[\r\n]+', ' ')) } }
    Write-Host ("::notice title=Camera soak (script)::" + ($Lines[$Lines.Count - 1] -replace '[\r\n]+', ' '))
    if ($script:InternetSeen.Count) {
      $soakAt = if ($soak) { [Math]::Round(((ConvertTo-UtcTime $soak.started_utc) - $origin).TotalSeconds) } else { -1 }
      $seen = @($script:InternetSeen.Values | Select-Object -First 15 | ForEach-Object { "$($_.what) seen at $($_.n) samples from $($_.first) s to $($_.last) s" }) -join '; '
      Write-Host ("::warning title=Camera soak internet connections (runner)::camera soak from {0} s; {1}" -f $soakAt, $seen)
    }
    foreach ($c in @($AllChecks | Where-Object { $_.Required -and $_.Verdict -ne 'PASS' })) { Write-Host ("::error title=Camera soak {0}::{1}: {2}" -f $c.Verdict, $c.Name, ($c.Detail -replace '[\r\n]+', ' ')) }
  }
  return [pscustomobject]@{ Facts = $r.Facts; Log = $log.Lines; Soak = $soak }
}

# --------------------------------------------------------------------- self-test

function New-SynthLine([double]$t, [datetime]$Origin, [string]$Kind, [string]$Detail, [int]$Fps, [double]$Frames, [int]$Stalls, [int]$Stops, [int]$Forced, $MaxStop, $LastStop, [bool]$PageOpen) {
  return [pscustomobject]@{
    t_s = $t; utc = $Origin.AddSeconds($t).ToString('o'); kind = $Kind; detail = $(if ($Detail) { $Detail } else { $null })
    on = ($Kind -ne 'off'); capture = ($Capturing -contains $Kind); page_open = $PageOpen; fps = $Fps; frames = $Frames; valid_frames = 0
    stalls = $Stalls; worker_restarts = $Stalls; stops = $Stops; forced_stops = $Forced; max_stop_ms = $MaxStop; last_stop_ms = $LastStop; inference_ms = 35.0
  }
}

# A healthy run of 2.8 hours, a log line every 30 s: capturing, with a lock of 10 minutes every
# 40 minutes (each released in 400 ms), then Off at the end.
function New-SynthRun([datetime]$Origin, [hashtable]$Change = @{}) {
  $log = New-Object System.Collections.Generic.List[object]
  $frames = 0.0; $stops = 0; $forced = 0; $max = $null; $last = $null; $stalls = 0
  $every = 30; $n = [int](2.8 * 3600 / $every)
  $fps = 3
  for ($i = 0; $i -le $n; $i++) {
    $t = $i * $every
    $min = $t / 60
    $kind = 'no_reading'; $detail = 'absent'; $page = $true
    $inHold = (($min % 40) -ge 30) -and -not $Change["NoHolds"]
    if ($inHold) { $kind = 'held'; $detail = 'away'; $page = $false }
    if ($Change["StartingMostly"] -and -not $inHold -and ($min % 40) -lt 25) { $kind = 'starting'; $detail = $null }
    if ($i -eq $n) { $kind = 'off'; $detail = $null; $page = $false }
    $prevHeld = $log.Count -and ($Capturing -contains $log[$log.Count - 1].kind) -and ($kind -eq 'held' -or $kind -eq 'off')
    if ($prevHeld) {
      $stops++; $ms = 400
      if ($Change["SlowStopAt"] -and $stops -eq $Change["SlowStopAt"]) { $ms = 2600 }
      if ($Change["ForcedAt"] -and $stops -eq $Change["ForcedAt"]) { $forced++; $ms = 1000 }
      $last = $ms; $max = if ($null -eq $max) { $ms } else { [Math]::Max($max, $ms) }
    }
    if ($Change["HoldPageOpen"] -and $inHold) { $page = $true }
    if ($kind -eq 'no_reading') {
      $rate = if ($Change["SlowFrames"]) { 1.0 } else { 2.95 }
      $frames += $rate * $every
    }
    if ($Change["StalledAt"] -and $i -eq $Change["StalledAt"]) { $kind = 'unavailable'; $detail = 'stalled'; $stalls += 2 }
    if ($Change["Stalls"] -and $i -eq 50) { $stalls += $Change["Stalls"] }
    $log.Add((New-SynthLine $t $Origin $kind $detail $fps $frames $stalls $stops $forced $max $last $page))
  }
  return $log
}

function New-SynthSamples([double]$Seconds, [hashtable]$Change = @{}) {
  $rand = New-Object Random 11
  $s = New-Object System.Collections.Generic.List[object]
  for ($t = 0.0; $t -le $Seconds; $t += 60) {
    $h = $t / 3600
    $ws = 260 + $rand.NextDouble() * 15
    if ($Change["Leak"]) { $ws += 40 * $h }
    if ($Change["Spike"] -and $t -eq 3000) { $ws = 420 }
    if ($Change["Gap"] -and $t -gt 900 -and $t -lt 8400) { continue }
    if ($Change["Sparse"] -and ($t % 600) -ne 0) { continue }
    $s.Add([ordered]@{
        t = $t; alive = (-not ($Change["DiesAt"] -and $t -ge $Change["DiesAt"])); working_set_mb = $ws; private_bytes_mb = $ws - 60; handles = 1800 + ($t % 7); threads = 120
        cpu_pct = $(if ($Change["Busy"]) { 14.0 } else { 3.5 + $rand.NextDouble() })
        tcp_ok = (-not $Change["NoTcp"]); tcp_internet = $(if ($Change["Internet"] -and $t -eq 600) { 1 } else { 0 }); tcp_lan = 1; tcp_loopback = 0
      })
  }
  return $s
}

function Invoke-SelfTest {
  $script:selfBad = 0
  function Expect([string]$Name, $Got, $Want) {
    $ok = ($Got -eq $Want)
    if (-not $ok) { $script:selfBad++ }
    Write-Host ("[{0}] {1}{2}" -f $(if ($ok) { 'ok' } else { 'WRONG' }), $Name, $(if ($ok) { '' } else { " (got $Got, want $Want)" }))
  }
  $origin = [datetime]::SpecifyKind([datetime]'2026-11-20T04:30:00', [DateTimeKind]::Utc)
  $opt = @{ NeedCaptureS = 7200; StallsPerHour = 1.0; BudgetsRequired = $true; NeedHold = $true; WarmupFraction = 0.1; SampleSeconds = 60 }
  function Run([hashtable]$LogChange, [hashtable]$SampleChange, [hashtable]$OptChange = @{}) {
    $raw = @(New-SynthRun $origin $LogChange | ForEach-Object { $_ | ConvertTo-Json -Compress })
    $log = ConvertFrom-CameraLog $raw $origin
    $o = @{}; foreach ($k in $opt.Keys) { $o[$k] = $opt[$k] }; foreach ($k in $OptChange.Keys) { $o[$k] = $OptChange[$k] }
    $r = Invoke-CameraAnalysis $log.Lines (New-SynthSamples (2.8 * 3600) $SampleChange) $o
    return $r
  }
  function Verdict-Of($r, [string]$Name) { (@($r.Checks | Where-Object { $_.Name -eq $Name }) | Select-Object -First 1).Verdict }

  $ok = Run @{} @{}
  foreach ($c in @($ok.Checks | Where-Object { $_.Verdict -ne 'PASS' })) { Write-Host "  (healthy run: $($c.Verdict) $($c.Name) - $($c.Detail))" }
  Expect 'a healthy 2-hour run passes' (Get-OverallVerdict $ok.Checks) 'PASS'
  Expect 'its camera time is counted without the holds' ($ok.Facts.capture_h -ge 2.0 -and $ok.Facts.capture_h -lt 2.4) $true
  Expect 'its releases are counted (four locks and the Off)' $ok.Facts.releases 5
  $r = Run @{} @{ Leak = $true }
  Expect 'memory growing 40 MB/h while capturing fails' (Verdict-Of $r 'no growth trend while capturing: working set (MB)') 'FAIL'
  Expect '... and the run fails' (Get-OverallVerdict $r.Checks) 'FAIL'
  $r = Run @{ StalledAt = 120 } @{}
  Expect 'a stop for "not responding" fails' (Verdict-Of $r 'never stopped as not responding') 'FAIL'
  $r = Run @{ Stalls = 5 } @{}
  Expect 'five stalls in about 2 hours fail' (Verdict-Of $r 'watchdog stalls bounded') 'FAIL'
  $r = Run @{ Stalls = 1 } @{}
  Expect 'one stall in about 2 hours passes' (Verdict-Of $r 'watchdog stalls bounded') 'PASS'
  $r = Run @{ SlowStopAt = 2 } @{}
  Expect 'a release taking 2.6 s fails' (Verdict-Of $r 'every release within 2 s') 'FAIL'
  $r = Run @{ ForcedAt = 3 } @{}
  Expect 'a release the page did not confirm fails' (Verdict-Of $r 'every release within 2 s') 'FAIL'
  $r = Run @{ NoHolds = $true } @{}
  Expect 'a run with no hold is not proven' (Verdict-Of $r 'holds release the camera') 'INCONCLUSIVE'
  Expect '... and is never PASS' (Get-OverallVerdict $r.Checks) 'INCONCLUSIVE'
  $r = Run @{ HoldPageOpen = $true } @{}
  Expect 'a hold that keeps the camera page fails' (Verdict-Of $r 'holds release the camera') 'FAIL'
  $r = Run @{ SlowFrames = $true } @{}
  Expect 'one frame a second while 3 are set fails the frame-rate budget' (Verdict-Of $r 'frame-rate budget held') 'FAIL'
  $r = Run @{} @{ Busy = $true }
  Expect 'CPU averaging 14% while capturing fails' (Verdict-Of $r 'PRD budget: CPU while capturing') 'FAIL'
  $r = Run @{} @{ Busy = $true } @{ BudgetsRequired = $false }
  Expect '... and is informational on a runner' (Get-OverallVerdict $r.Checks) 'PASS'
  $r = Run @{} @{ Internet = $true }
  Expect 'one connection to an internet address fails' (Verdict-Of $r 'no internet connection') 'FAIL'
  $r = Run @{} @{ NoTcp = $true }
  Expect 'unreadable TCP connections are not proven' (Verdict-Of $r 'no internet connection') 'INCONCLUSIVE'
  $r = Run @{} @{ DiesAt = 5000 }
  Expect 'the companion ending mid-run fails' (Verdict-Of $r 'no crash or restart') 'FAIL'
  $r = Run @{} @{} @{ NeedCaptureS = 4 * 3600 }
  Expect 'too little camera time is not proven' (Verdict-Of $r 'camera time covered') 'INCONCLUSIVE'
  $r = Run @{ StartingMostly = $true } @{}
  Expect 'long camera starts with a few healthy intervals do not count as camera time' (Verdict-Of $r 'camera time covered') 'INCONCLUSIVE'
  Expect '... and the run is never PASS' ((Get-OverallVerdict $r.Checks) -ne 'PASS') $true
  Expect '... while the few healthy intervals still pass the frame-rate check' (Verdict-Of $r 'frame-rate budget held') 'PASS'
  $r = Run @{} @{ Gap = $true }
  Expect 'two hours of camera log with process samples missing for two hours is not proven' (Verdict-Of $r 'process samples cover the camera time') 'INCONCLUSIVE'
  Expect '... nor are the trends from the samples left' (Verdict-Of $r 'no growth trend while capturing: working set (MB)') 'INCONCLUSIVE'
  Expect '... nor the memory budget' (Verdict-Of $r 'PRD budget: memory while capturing') 'INCONCLUSIVE'
  Expect '... and the run is never PASS' ((Get-OverallVerdict $r.Checks) -ne 'PASS') $true
  $r = Run @{} @{ Sparse = $true } @{ SampleSeconds = 30; FailedSamples = 0 }
  Expect 'a 30 s sampler with one sample surviving every 10 minutes is not proven' (Verdict-Of $r 'process samples cover the camera time') 'INCONCLUSIVE'
  Expect '... nor are its trends' (Verdict-Of $r 'no growth trend while capturing: handles') 'INCONCLUSIVE'
  Expect '... and the run is never PASS' ((Get-OverallVerdict $r.Checks) -ne 'PASS') $true
  $r = Run @{} @{} @{ FailedSamples = 120 }
  Expect 'a sampler with 40% of its attempts failed is not proven' (Verdict-Of $r 'process samples cover the camera time') 'INCONCLUSIVE'
  $r = Run @{} @{} @{ FailedSamples = 10 }
  Expect 'a few failed attempts are tolerated' (Verdict-Of $r 'process samples cover the camera time') 'PASS'
  $r = Run @{} @{ Spike = $true }
  Expect 'one sample over 350 MB fails the cap though the median is far below' (Verdict-Of $r 'PRD budget: memory while capturing') 'FAIL'
  $empty = Invoke-CameraAnalysis @() @() $opt
  Expect 'no log and no samples is never PASS' (Get-OverallVerdict $empty.Checks) 'INCONCLUSIVE'
  $old = ConvertFrom-CameraLog @(New-SynthRun $origin @{} | ForEach-Object { $_.PSObject.Properties.Remove('stops'); $_ | ConvertTo-Json -Compress }) $origin
  $r = Invoke-CameraAnalysis $old.Lines (New-SynthSamples (2.8 * 3600)) $opt
  Expect 'a log without release counters (old build) is not proven' (Verdict-Of $r 'every release within 2 s') 'INCONCLUSIVE'
  $junk = ConvertFrom-CameraLog @('not json', '{"kind":"off"}', '') $origin
  Expect 'unreadable log lines are counted, not used' ($junk.Bad -eq 2 -and $junk.Lines.Count -eq 0) $true
  Expect 'loopback is loopback' (Get-AddressClass '127.0.0.1') 'loopback'
  Expect 'IPv6 loopback' (Get-AddressClass '::1') 'loopback'
  Expect 'a 192.168 server is LAN' (Get-AddressClass '192.168.50.10') 'lan'
  Expect '172.20 is LAN' (Get-AddressClass '172.20.1.4') 'lan'
  Expect '172.32 is internet' (Get-AddressClass '172.32.1.4') 'internet'
  Expect 'link-local IPv6 is LAN' (Get-AddressClass 'fe80::1c2:3') 'lan'
  Expect 'a mapped private IPv4 is LAN' (Get-AddressClass '::ffff:10.1.2.3') 'lan'
  Expect 'a public IPv4 is internet' (Get-AddressClass '13.107.42.14') 'internet'
  Expect 'a public IPv6 is internet' (Get-AddressClass '2603:1030::5') 'internet'
  $rust = '2026-11-20T04:30:00.123456789+00:00'
  Expect 'the app''s RFC 3339 time (nanoseconds) parses' (((ConvertTo-UtcTime $rust) - $origin).TotalMilliseconds -ge 123) $true
  Expect 'a DateTime (PowerShell 7 JSON) is taken as is' ((ConvertTo-UtcTime $origin) -eq $origin) $true
  Expect 'any FAIL makes the run FAIL' (Get-OverallVerdict @((New-Check 'a' 'PASS' ''), (New-Check 'b' 'INCONCLUSIVE' ''), (New-Check 'c' 'FAIL' ''))) 'FAIL'
  Expect 'an informational miss does not' (Get-OverallVerdict @((New-Check 'a' 'PASS' ''), (New-Check 'b' 'INFO' '' $false))) 'PASS'
  # The CI report on data shaped exactly as the app writes it (its JSON report and its
  # camera log lines, read back as text), over a 10-minute soak with a release every 33 s.
  $kinds = @(@('held', 'away'), @('held', 'presentation'), @('held', 'away'), @('held', 'meeting'), @('off', $null))
  $raw = New-Object System.Collections.Generic.List[string]
  $frames = 0; $stops = 0; $k = 0; $prevKind = ''
  $soakFrom = $origin.AddSeconds(130); $soakTo = $origin.AddSeconds(730)
  for ($t = 0; $t -le 760; $t += 2) {
    $in = $t - 130; $phase = if ($in -lt 0) { -1 } else { $in % 33 }
    $kind = 'no_reading'; $detail = 'absent'; $page = $true
    if ($phase -ge 22 -and $phase -lt 26) { $kind = $kinds[$k % 5][0]; $detail = $kinds[$k % 5][1]; $page = $false }
    elseif ($phase -ge 26 -and $phase -lt 28) { $kind = 'starting'; $detail = $null }
    if ($phase -ge 32) { $k = [int][Math]::Floor($in / 33) + 1 }
    if (($kind -eq 'held' -or $kind -eq 'off') -and $prevKind -eq 'no_reading') { $stops++ }
    $prevKind = $kind
    if ($kind -eq 'no_reading') { $frames += 6 }
    $line = [ordered]@{ t_s = $t; utc = $origin.AddSeconds($t).ToString('yyyy-MM-ddTHH:mm:ss.fffZ'); kind = $kind; detail = $detail; on = ($kind -ne 'off'); capture = ($Capturing -contains $kind); page_open = $page
      fps = 3; frames = $frames; valid_frames = 0; stalls = 2; worker_restarts = 1; stops = $stops + 3; forced_stops = 1; max_stop_ms = 1000; last_stop_ms = 350; inference_ms = 41.2 }
    $raw.Add(($line | ConvertTo-Json -Compress))
  }
  $appJson = '{"ok":true,"checks":[{"name":"camera_soak_completed","ok":true,"required":true,"detail":"17 cycles"},{"name":"camera_soak_frame_rate","ok":true,"required":true,"detail":"rates [2.95]"}],"monitors":[],"webview2":"1.0","soak":null,"camera_soak":{"started_utc":"' + $soakFrom.ToString('yyyy-MM-ddTHH:mm:ss.fffZ') + '","ended_utc":"' + $soakTo.ToString('yyyy-MM-ddTHH:mm:ss.fffZ') + '","real_s":600,"cycles":17,"releases":[],"windows":[],"windows_rate_changed":0,"frames":3000,"stalls":0,"worker_restarts":0,"stops":17,"forced_stops":0,"max_stop_ms":420}}'
  $ciSamples = New-Object System.Collections.Generic.List[object]
  foreach ($x in (New-SynthSamples 760)) { $ciSamples.Add($x) }
  for ($t = 0; $t -le 760; $t += 5) { $ciSamples.Add([ordered]@{ t = [double]$t; alive = $true; working_set_mb = 300.0; private_bytes_mb = 200.0; handles = 2000; threads = 150; cpu_pct = 6.0; tcp_ok = $true; tcp_internet = 0; tcp_lan = 0; tcp_loopback = 2 }) }
  $picked = @($ciSamples | Where-Object { $_.t % 5 -eq 0 } | Sort-Object { $_.t } | Group-Object { $_.t } | ForEach-Object { $_.Group[-1] })
  # As the sampling loop collects them: ordered dictionaries in a generic list.
  $ciSamples = New-Object System.Collections.Generic.List[object]
  foreach ($x in $picked) { $ciSamples.Add($x) }
  $before = $AllChecks.Count; $linesBefore = $Lines.Count
  # Synthetic numbers must not reach the runner's annotations.
  $gha = $env:GITHUB_ACTIONS; $env:GITHUB_ACTIONS = $null
  try { $ci = Invoke-CiReport $ciSamples ($appJson | ConvertFrom-Json) 0 $false @($raw) $origin }
  finally { $env:GITHUB_ACTIONS = $gha }
  $new = @($AllChecks | Select-Object -Skip $before)
  foreach ($c in @($new | Where-Object { $_.Required -and $_.Verdict -ne 'PASS' })) { Write-Host "  (CI report: $($c.Verdict) $($c.Name) - $($c.Detail))" }
  Expect 'the CI report on app-shaped data passes' (Get-OverallVerdict $new) 'PASS'
  Expect '... counting only the soak window''s releases (counters from before ignored)' $ci.Facts.releases 18
  while ($AllChecks.Count -gt $before) { $AllChecks.RemoveAt($AllChecks.Count - 1) }
  while ($Lines.Count -gt $linesBefore) { $Lines.RemoveAt($Lines.Count - 1) }
  if ($script:selfBad) { throw "Camera soak analysis self-test: $($script:selfBad) case(s) wrong" }
  Write-Host 'Camera soak analysis self-test: all cases right.'
}

if ($SelfTest) { Invoke-SelfTest; return }

$exe = Join-Path $env:LOCALAPPDATA 'HealthierYou\hy-companion.exe'

# ------------------------------------------------------------- CI: camera soak

if ($CiSelfTest) {
  $SampleSeconds = [Math]::Min($SampleSeconds, 5)
  Write-Environment 'CI, accelerated, synthetic camera'
  if (-not (Test-Path $exe)) { throw 'The companion is not installed' }
  $selfReport = Join-Path ([IO.Path]::GetTempPath()) "hy-camera-soak-selftest-$PID.json"
  $logFile = Join-Path ([IO.Path]::GetTempPath()) "hy-camera-soak-$PID.jsonl"
  New-Item -ItemType File -Force -Path $logFile | Out-Null
  $env:HY_SELFTEST = $selfReport
  $env:HY_SELFTEST_IDLE_S = '0'
  $env:HY_SELFTEST_CAMERA_SOAK_S = [string][int]($Minutes * 60)
  $env:HY_CAMERA_SOAK_LOG = $logFile
  $env:HY_CAMERA_SOAK_EVERY_S = '2'
  $origin = (Get-Date).ToUniversalTime()
  $script:CiOrigin = $origin
  $p = Start-Process -FilePath $exe -PassThru
  $null = $p.Handle
  foreach ($v in 'HY_SELFTEST', 'HY_SELFTEST_IDLE_S', 'HY_SELFTEST_CAMERA_SOAK_S', 'HY_CAMERA_SOAK_LOG', 'HY_CAMERA_SOAK_EVERY_S') { Remove-Item "Env:$v" -ErrorAction SilentlyContinue }
  $samples = New-Object System.Collections.Generic.List[object]
  $deadline = (Get-Date).AddMinutes($Minutes * 2 + 15)
  while (-not $p.HasExited -and (Get-Date) -lt $deadline) {
    Start-Sleep -Seconds $SampleSeconds
    if ($p.HasExited) { break }
    # A process ending while Windows lists them can fail one sample: skip it, count it.
    try { $samples.Add((Get-ProcessSample $p.Id $origin)) } catch { $script:SampleErrors++ }
  }
  $timedOut = -not $p.HasExited
  if ($timedOut) { Stop-Process -Id $p.Id -Force }
  $p.WaitForExit()
  # A sample taken while the app was exiting by itself is not a crash.
  while ($samples.Count -and -not $samples[$samples.Count - 1].alive) { $samples.RemoveAt($samples.Count - 1) }
  $app = if (Test-Path $selfReport) { Get-Content -Raw $selfReport | ConvertFrom-Json } else { $null }
  $raw = @(); if (Test-Path -LiteralPath $logFile) { $raw = @(Get-Content -LiteralPath $logFile) }
  Remove-Item $logFile, $selfReport -ErrorAction SilentlyContinue
  $ci = Invoke-CiReport $samples $app $p.ExitCode $timedOut $raw $origin
  $verdict = Save-Report @{ facts = $ci.Facts; samples = $samples; camera_log = $ci.Log; app_camera_soak = $ci.Soak }
  Exit-With $verdict
}

# ---------------------------------------------------------------- office PC run

if (-not $Report) { throw 'Pass -Report <file> (a text report; a .json with the samples is written beside it)' }
if (-not (Test-Path $exe)) { throw 'HealthierYou is not installed for this user' }
function Ask([string]$Question) {
  Write-Host ''
  Write-Host $Question -ForegroundColor Cyan
  $a = Read-Host '  y = yes, n = no, s = skip'
  if ($a -match '^[yY]') { return 'PASS' } elseif ($a -match '^[nN]') { return 'FAIL' } else { return 'INCONCLUSIVE' }
}
function Wait-Step([string]$Text) {
  Write-Host ''
  Write-Host $Text -ForegroundColor Yellow
  $null = Read-Host '  Press Enter when done'
}
if (-not $Webcam) {
  $w = Read-Host 'Which webcam will HealthierYou use? b = built into the PC, u = USB (second webcam), o = other'
  $Webcam = switch -Regex ($w) { '^[bB]' { 'built-in' } '^[uU]' { 'usb' } default { 'other' } }
}
Write-Environment "office PC, $Hours h of camera time"
Add-Line "Webcam in use (as named by the person running the test): $Webcam"
Add-Line "Companion version: $((Get-Item $exe).VersionInfo.ProductVersion)"

while (Get-Process -Name hy-companion -ErrorAction SilentlyContinue) {
  Wait-Step 'HealthierYou is running. Quit it from its tray icon (right-click > Quit) so it can start again with its camera log on.'
}
$logFile = Join-Path ([IO.Path]::GetTempPath()) "hy-camera-soak-$PID.jsonl"
New-Item -ItemType File -Force -Path $logFile | Out-Null
$env:HY_CAMERA_SOAK_LOG = $logFile
$env:HY_CAMERA_SOAK_EVERY_S = '5'
$origin = (Get-Date).ToUniversalTime()
$p = Start-Process -FilePath $exe -PassThru
Remove-Item Env:HY_CAMERA_SOAK_LOG, Env:HY_CAMERA_SOAK_EVERY_S -ErrorAction SilentlyContinue
Start-Sleep -Seconds 20
if ($p.HasExited) { throw 'HealthierYou did not keep running after it was started' }
$mainId = $p.Id
$samples = New-Object System.Collections.Generic.List[object]
$baseline = New-Object System.Collections.Generic.List[object]
$log = Read-CameraLog $logFile $origin
if ($log.Lines.Count -eq 0) { Add-Line 'The app wrote no camera log line in 20 s (a build from before V2-5?). The run continues but cannot PASS.' }
$onAtStart = @($log.Lines | Where-Object { $_.on }).Count -gt 0
if (-not $onAtStart) {
  Add-Line 'Measuring 2 minutes with the camera off...'
  $end = (Get-Date).AddMinutes(2)
  while ((Get-Date) -lt $end) { $baseline.Add((Get-ProcessSample $mainId $origin)); Start-Sleep -Seconds 10 }
}
else { Add-Line 'The posture check was already on when the app started: no camera-off baseline.' }

Wait-Step ("Turn the posture check ON (pet menu > Posture check > Turn on camera) if it is not on. Then work as usual and keep this window open.`n" +
  "  During the run: lock the PC at least once for a minute (Windows+L, or at lunch), and if you can, have one call or slide show.`n" +
  "  The run ends by itself after $Hours h of camera time (at most $([Math]::Round($Hours * 2 + 0.5, 1)) h); then it asks two questions.")
$needS = $Hours * 3600
$capS = $needS * 2 + 1800
$start = Get-Date
$lastLog = $null
while ($true) {
  try { $x = Get-ProcessSample $mainId $origin } catch { $script:SampleErrors++; Start-Sleep -Seconds $SampleSeconds; continue }
  $samples.Add($x)
  if (-not $x.alive) { Add-Line "HealthierYou stopped at $([Math]::Round($x.t / 60, 1)) min."; break }
  if (((Get-Date) - $start).TotalMinutes -ge 1 -and $samples.Count % 4 -eq 0) {
    $log = Read-CameraLog $logFile $origin
    $done = 0.0
    $L = @($log.Lines)
    for ($i = 1; $i -lt $L.Count; $i++) {
      $dt = $L[$i].t - $L[$i - 1].t
      if ($dt -le 20 -and (Test-Reading $L[$i]) -and (Test-Reading $L[$i - 1])) { $done += $dt }
    }
    if ($L.Count) { $lastLog = $L[$L.Count - 1] }
    Write-Host ("  {0:HH:mm} camera time {1:N2} h of {2} h; status {3}{4}" -f (Get-Date), ($done / 3600), $Hours, $(if ($lastLog) { $lastLog.kind } else { 'no log' }), $(if ($lastLog -and $lastLog.detail) { " ($($lastLog.detail))" }))
    if ($done -ge $needS) { break }
  }
  if (((Get-Date) - $start).TotalSeconds -ge $capS) { Add-Line 'The time limit was reached before the camera time.'; break }
  Start-Sleep -Seconds $SampleSeconds
}

# The final Off: the user's action, timed by the app.
$offChecks = New-Object System.Collections.Generic.List[object]
if ((Get-Process -Id $mainId -ErrorAction SilentlyContinue)) {
  $askedAt = ((Get-Date).ToUniversalTime() - $origin).TotalSeconds
  Wait-Step 'Now turn the camera OFF (pet menu > Turn camera off) and watch the webcam light and the pet.'
  $offLine = $null
  $until = (Get-Date).AddMinutes(2)
  while ((Get-Date) -lt $until -and -not $offLine) {
    Start-Sleep -Seconds 3
    $offLine = @((Read-CameraLog $logFile $origin).Lines | Where-Object { $_.t -ge $askedAt -and $_.kind -eq 'off' -and -not $_.page_open } | Select-Object -Last 1)
    if ($offLine.Count) { $offLine = $offLine[0] } else { $offLine = $null }
  }
  if (-not $offLine) { $offChecks.Add((New-Check 'Off releases the camera within 2 s' 'INCONCLUSIVE' 'the log showed no Off within 2 minutes')) }
  else {
    $prior = @((Read-CameraLog $logFile $origin).Lines | Where-Object { $_.t -lt $askedAt } | Select-Object -Last 1)
    $ms = $offLine.last_stop_ms
    # Only a stop counted after the question is this Off's (a camera already held needs none).
    if (-not $prior.Count -or $null -eq $prior[0].stops -or $null -eq $offLine.stops -or [int]$offLine.stops -le [int]$prior[0].stops) { $ms = $null }
    $okOff = $null -ne $ms -and [double]$ms -le $Budget.StopMs
    $offChecks.Add((New-Check 'Off releases the camera within 2 s' $(if ($null -eq $ms) { 'INCONCLUSIVE' } elseif ($okOff) { 'PASS' } else { 'FAIL' }) $(if ($null -eq $ms) { 'no release was counted for this Off: the camera was not capturing when it was turned off (a hold or a conflict), so this check needs another run' } else { "the page confirmed the release in $ms ms (deadline $($Budget.StopMs) ms)" })))
  }
  $offChecks.Add((New-Check 'webcam light off within about 2 s (seen)' (Ask 'Did the webcam light go off (if it has one) and the CAM badge go within about 2 seconds?') 'as answered by the person running the test'))
  Start-Sleep -Seconds 10
  $after = New-Object System.Collections.Generic.List[object]
  for ($i = 0; $i -lt 3; $i++) { $after.Add((Get-ProcessSample $mainId $origin)); Start-Sleep -Seconds 10 }
}
$log = Read-CameraLog $logFile $origin
# Deleting the log file stops the app's camera log (it appends only to a file that exists).
Remove-Item -LiteralPath $logFile -ErrorAction SilentlyContinue

$opt = @{ NeedCaptureS = $needS; StallsPerHour = $Budget.StallsPerHour; BudgetsRequired = $true; NeedHold = $true; WarmupFraction = 0.1; SampleSeconds = $SampleSeconds; FailedSamples = $script:SampleErrors }
$r = Invoke-CameraAnalysis $log.Lines $samples $opt
Add-Checks $r.Checks
Add-Checks $offChecks
if ($baseline.Count -ge 3) {
  $offWs = Get-Median ([double[]]@($baseline | ForEach-Object { $_.working_set_mb }))
  $offCpu = (@($baseline | Where-Object { $null -ne $_.cpu_pct } | ForEach-Object { $_.cpu_pct }) | Measure-Object -Average).Average
  Add-Line ("       camera off before the run: working set median {0:N1} MB, CPU {1:N2}%; the camera added about {2:N1} MB and {3:N2}%" -f $offWs, $offCpu, ([double]$r.Facts['ws_median_mb'] - $offWs), ([double]$r.Facts['cpu_avg_pct'] - $offCpu))
  if (Get-Variable after -ErrorAction SilentlyContinue) {
    $afterWs = Get-Median ([double[]]@($after | ForEach-Object { $_.working_set_mb }))
    Add-Checks @((New-Check 'memory returned after Off' $(if ($afterWs -le $offWs + 60) { 'PASS' } else { 'FAIL' }) ('working set {0:N1} MB after Off (was {1:N1} MB before; within 60 MB)' -f $afterWs, $offWs) $false))
  }
}
$verdict = Save-Report @{ webcam = $Webcam; facts = $r.Facts; samples = $samples; baseline = $baseline; camera_log = $log.Lines }
Exit-With $verdict
