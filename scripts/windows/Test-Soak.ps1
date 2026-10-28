<#
.SYNOPSIS
  The HealthierYou soak (slice V1-7; architecture 10, 11 "Soak" and assumption A2; PRD
  performance targets): an unattended run of the installed companion or office server,
  sampled on a schedule, with a pass/fail summary and a report safe to share.

.DESCRIPTION
  Office PC runs (the real soak; the owner runs them):

    -Role Companion   on a companion PC, as the office user (no administrator). Uses the
                      installed app, or installs -Installer silently per user first, and
                      starts it if it is not running. Leave the PC as it is used overnight
                      or at a desk: locking it and letting it sleep are fine (the run
                      notices both and counts awake time only).
    -Role Server      on the server PC, in an elevated PowerShell. Samples the
                      HealthierYouServer service and its data folder while the office's
                      companions sync as usual.

  Every -SampleSeconds (60) it records, for the program's whole process tree (the
  companion and its bundled WebView2 processes): working set, private working set,
  private bytes, handle and thread counts, and CPU as a share of the whole machine; the
  database and write-ahead log sizes; log sizes (and on the server, backups, mirror and
  request counts from its log); whether the session was locked; and gaps in sampling
  (sleep). Every 15 minutes the companion's own store statistics are read with
  `hy-companion.exe --hy-soak-stats` (read-only; counts and times only).

  At the end it checks, for at least -Hours of awake time (8 by default):

    - no crash or restart: the same main process throughout;
    - no growth trend in memory, handles or threads after a warm-up (the last third of
      the run against the first, and the fitted slope);
    - PRD budgets for the companion: the app's own memory (private working set, summed
      over the tree) at most 150 MB, median and highest sample (DEC14; the whole-tree
      working set, which counts shared pages, is reported for information), CPU at most 2%
      of the machine averaged over every 30 minutes, no spike over 5% lasting 3 samples;
    - the database's write-ahead log at most 16 MiB and the database growing at most
      4 MiB, logs within their rotation;
    - reminders never closer than the 5-minute calm gap and at most 6 in any hour;
    - server: no panic, no 5xx answers, the service never restarted.

  CI runs (the Windows job; shortened, with accelerated clocks):

    -CiSelfTest -Minutes 12   starts the installed companion with HY_SELFTEST and
                              HY_SELFTEST_SOAK_S: after its self-test the app runs 72
                              simulated office hours (reminders answered on the real
                              bubble, lock, sleep, windows opened and closed) within the
                              given minutes, and checks cadence and store bounds itself;
                              this script samples it and checks for growth trends.
    -Sim hy-sim.exe -Minutes 8 -SimDays 30
                              runs `hy-sim soak`: the real server in process with 10
                              simulated companions over 30 simulated office days, which
                              checks every row, backups and sync times itself; this script
                              samples it and checks for growth trends.
    -SelfTest                 checks this script's own analysis on synthetic series.

  The report (text, plus JSON with every sample) holds no user name, PC name, path,
  address or device id. It records the Windows build, processor and memory, as the PRD
  asks ("record processor, sampling method and conditions").

.EXAMPLE
  .\Test-Soak.ps1 -Role Companion -Report .\hy-soak-companion.txt
.EXAMPLE
  .\Test-Soak.ps1 -Role Companion -Installer .\HealthierYou_0.1.0_x64-setup.exe -Hours 8 -Report .\hy-soak.txt
.EXAMPLE
  # Elevated, on the server PC.
  .\Test-Soak.ps1 -Role Server -Report .\hy-soak-server.txt
#>
[CmdletBinding()]
param(
  [ValidateSet('Companion', 'Server')][string]$Role = 'Companion',
  [double]$Hours = 8,
  [int]$SampleSeconds = 60,
  [string]$Installer,
  [string]$ServerData,
  [switch]$CiSelfTest,
  [string]$Sim,
  [double]$Minutes = 12,
  [int]$SimDays = 30,
  [switch]$SelfTest,
  [string]$Report
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2

# Budgets and bounds (PRD "Performance"; architecture 4.3 and 10). Trend tolerances are
# generous so only real growth fails: a leak must both grow past the tolerance between the
# first and last third of the run and show a slope that adds up to it.
$Budget = @{
  WorkingSetMB  = 150   # PRD: V1 companion at most 150 MB of its own memory (private working set, DEC14)
  CpuAvgPct     = 2.0   # PRD: average idle CPU at most 2% of the machine (30-minute windows)
  CpuSpikePct   = 5.0   # PRD: no sustained idle spike over 5%
  SpikeSamples  = 3
  WalMaxBytes   = 16MB
  DbGrowthBytes = 4MB
  LogGrowthBytes = 12MB  # one 10 MB rotation plus slack
  MinOfferGapMin = 5.0  # the engine's calm gap
  MaxOffersPerHour = 6
}
$Trend = @{
  MemoryMB = @{ Abs = 20; Rel = 0.10 }
  Handles  = @{ Abs = 200; Rel = 0.15 }
  Threads  = @{ Abs = 15; Rel = 0.20 }
}

# ------------------------------------------------------------------- analysis

# The running server keeps server.log open for writing, and [IO.File]::ReadLines asks
# for read-only sharing, which Windows refuses while a writer holds the file. Open it
# the way a log viewer does: read access, sharing read, write and delete.
function Read-SharedLines([string]$Path) {
  $fs = [IO.FileStream]::new($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]'ReadWrite, Delete')
  try {
    $sr = [IO.StreamReader]::new($fs)
    while ($null -ne ($line = $sr.ReadLine())) { $line }
  } finally { $fs.Dispose() }
}

function Get-Median([double[]]$v) {
  if (-not $v -or $v.Count -eq 0) { return [double]::NaN }
  $s = $v | Sort-Object
  $n = $s.Count
  if ($n % 2) { return [double]$s[($n - 1) / 2] }
  return ([double]$s[$n / 2 - 1] + [double]$s[$n / 2]) / 2
}

# Growth of one series after a warm-up: the median of the last third against the first,
# and the least-squares slope per hour. Fails only if both exceed the tolerance.
function Test-Trend([string]$Name, [double[]]$Hours, [double[]]$Values, [double]$Abs, [double]$Rel) {
  $n = $Values.Count
  if ($n -lt 9) { return [pscustomobject]@{ Name = $Name; Pass = $true; Detail = "too few samples ($n) for a trend" } }
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

# Gaps in sampling (sleep, or the script itself suspended): deltas well over the interval.
function Get-AwakeTime([double[]]$Seconds, [int]$Interval) {
  $limit = [Math]::Max(3 * $Interval, $Interval + 60)
  $awake = 0.0; $gaps = 0; $gapSec = 0.0
  for ($i = 1; $i -lt $Seconds.Count; $i++) {
    $d = $Seconds[$i] - $Seconds[$i - 1]
    if ($d -gt $limit) { $gaps++; $gapSec += $d } else { $awake += $d }
  }
  return [pscustomobject]@{ AwakeS = $awake; Gaps = $gaps; GapSeconds = $gapSec }
}

# CPU budget: the highest 30-minute average and the longest run of samples over the spike
# limit.
function Test-Cpu([double[]]$Seconds, [double[]]$Cpu, [double]$AvgLimit, [double]$SpikeLimit, [int]$SpikeSamples) {
  $n = $Cpu.Count
  $avg = if ($n) { ($Cpu | Measure-Object -Average).Average } else { 0 }
  # Every full 30-minute window (25 minutes or more of samples); a shorter run is one window.
  $worst = if ($n -and ($Seconds[$n - 1] - $Seconds[0]) -lt 1500) { $avg } else { 0.0 }
  for ($i = 0; $i -lt $n; $i++) {
    $sum = 0.0; $k = 0; $j = $i
    while ($j -lt $n -and $Seconds[$j] - $Seconds[$i] -le 1800) { $sum += $Cpu[$j]; $k++; $j++ }
    if ($Seconds[$j - 1] - $Seconds[$i] -ge 1500) { $worst = [Math]::Max($worst, $sum / $k) }
  }
  $run = 0; $longest = 0
  foreach ($c in $Cpu) { if ($c -gt $SpikeLimit) { $run++; $longest = [Math]::Max($longest, $run) } else { $run = 0 } }
  return [pscustomobject]@{
    Pass = ($worst -le $AvgLimit -and $longest -lt $SpikeSamples)
    Detail = ('average {0:N2}%, highest 30-minute average {1:N2}% (budget {2}%); longest run over {3}%: {4} samples (fails at {5})' -f $avg, $worst, $AvgLimit, $SpikeLimit, $longest, $SpikeSamples)
  }
}

# The app's own memory in one sample (DEC14): the private working set summed over the
# process tree, only when EVERY process of the tree has a reading. A process missing from
# the counters (a WebView2 child, or one that exited meanwhile) would lower the sum
# unseen, so such a sample has no private value and counts the processes missed.
function Get-PrivateWsSum([int[]]$Ids, $Rows) {
  $read = @{}
  foreach ($r in @($Rows)) {
    if ($null -eq $r) { continue }
    $id = [int]$r.IDProcess
    if ($Ids -contains $id) { $read[$id] = [double]$r.WorkingSetPrivate }
  }
  $missing = @($Ids | Where-Object { -not $read.ContainsKey($_) }).Count
  $mb = $null
  if ($Ids.Count -gt 0 -and $missing -eq 0) { $mb = [Math]::Round((($read.Values | Measure-Object -Sum).Sum) / 1MB, 1) }
  return [pscustomobject]@{ Mb = $mb; Missing = $missing }
}

# Coverage of the private readings over the samples the budget is judged on, in time
# order (a missing reading is $null): at least 80% of the samples, and never more than 2
# in a row without one (three intervals is where the sampler counts a gap), so a high
# stretch cannot hide in a hole of the readings while low samples elsewhere pass.
function Get-PrivateCoverage([object[]]$Values) {
  $have = 0; $run = 0; $longest = 0
  foreach ($v in $Values) {
    if ($null -ne $v) { $have++; $run = 0 } else { $run++; if ($run -gt $longest) { $longest = $run } }
  }
  $n = $Values.Count
  $share = if ($n -gt 0) { $have / $n } else { 0 }
  return [pscustomobject]@{
    Have = $have; Total = $n; Share = $share; LongestMissing = $longest
    Ok = ($n -gt 0 -and $share -ge 0.8 -and $longest -le 2)
    Detail = ('complete private readings in {0} of {1} samples ({2:N0}%, at least 80%), at most {3} in a row without one (at most 2)' -f $have, $n, (100 * $share), $longest)
  }
}

# Memory budget (DEC14): the app's own memory, the private working set summed over the
# process tree, at most the limit at the median and at the highest sample, with complete
# readings covering the samples (Get-PrivateCoverage; this script has no INCONCLUSIVE, so
# short coverage fails). The whole-tree working set also counts pages shared with other
# programs (WebView2 and system DLLs, which Windows trims and re-adds at will), so it is
# reported for information only. $PrivateWs: one value per sample in time order, $null
# where the sample has no complete reading.
function Test-MemoryBudget([object[]]$PrivateWs, [double[]]$TotalWs, [double]$LimitMB) {
  $total = if ($TotalWs.Count) { '; whole-tree working set (shared pages included, for information) median {0:N1} MB, highest {1:N1} MB' -f (Get-Median $TotalWs), (($TotalWs | Measure-Object -Maximum).Maximum) } else { '' }
  $cov = Get-PrivateCoverage $PrivateWs
  $vals = [double[]]@($PrivateWs | Where-Object { $null -ne $_ })
  if (-not $vals.Count) { return [pscustomobject]@{ Pass = $false; Detail = "private working set unavailable (no sample had a complete reading)$total" } }
  $med = Get-Median $vals
  $max = ($vals | Measure-Object -Maximum).Maximum
  return [pscustomobject]@{
    Pass = ($cov.Ok -and $med -le $LimitMB -and $max -le $LimitMB)
    Detail = ('private working set median {0:N1} MB, highest {1:N1} MB (limit {2} MB for both); {3}{4}{5}' -f $med, $max, $LimitMB, $cov.Detail, $(if ($cov.Ok) { '' } else { ' - not enough to prove the budget' }), $total)
  }
}

# The private value of each sample, in order, $null where it has none.
function Get-PrivateValues($Samples) {
  $list = New-Object System.Collections.Generic.List[object]
  # Copied one by one (@() on a generic list of ordered dictionaries can fail).
  foreach ($x in $Samples) { if ($x.Contains('private_ws_mb')) { $list.Add($x['private_ws_mb']) } else { $list.Add($null) } }
  return ,($list.ToArray())
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
$Cores = [Environment]::ProcessorCount

function Get-ProcessSample([int[]]$Roots, [datetime]$Start) {
  $now = Get-Date
  $tree = Get-Tree $Roots
  $ids = @($tree | ForEach-Object { [int]$_.ProcessId })
  $privWs = $null; $privMissing = $null
  try {
    $pr = Get-PrivateWsSum $ids @(Get-CimInstance Win32_PerfFormattedData_PerfProc_Process -Property IDProcess, WorkingSetPrivate)
    $privWs = $pr.Mb; $privMissing = $pr.Missing
  }
  catch { }
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
  $locked = [bool](Get-Process -Name LogonUI -ErrorAction SilentlyContinue)
  return [ordered]@{
    t_s = [Math]::Round(($now - $Start).TotalSeconds, 1)
    processes = $tree.Count
    working_set_mb = [Math]::Round((($tree | Measure-Object WorkingSetSize -Sum).Sum) / 1MB, 1)
    private_ws_mb = $privWs
    private_ws_missing = $privMissing
    private_bytes_mb = [Math]::Round((($tree | Measure-Object PrivatePageCount -Sum).Sum) / 1MB, 1)
    handles = [int](($tree | Measure-Object HandleCount -Sum).Sum)
    threads = [int](($tree | Measure-Object ThreadCount -Sum).Sum)
    cpu_pct = $cpu
    locked = $locked
    roots_alive = (@($Roots | Where-Object { $ids -contains $_ }).Count -eq $Roots.Count)
  }
}

# A CI run ends when the program exits by itself: a sample taken while it was exiting is
# not a crash (its exit code and report decide that).
function Remove-TrailingExit($Samples) {
  $list = @(foreach ($x in $Samples) { $x })
  $n = $list.Count
  while ($n -gt 0 -and -not $list[$n - 1].roots_alive) { $n-- }
  if ($n -eq 0) { return @() }
  return @($list[0..($n - 1)])
}

function Get-Size([string]$Path) {
  if ($Path -and (Test-Path -LiteralPath $Path -PathType Leaf)) { return (Get-Item -LiteralPath $Path).Length }
  return 0
}
function Get-FolderSize([string]$Path, [string]$Filter = '*') {
  if (-not $Path -or -not (Test-Path -LiteralPath $Path)) { return 0 }
  $s = (Get-ChildItem -LiteralPath $Path -File -Filter $Filter -ErrorAction SilentlyContinue | Measure-Object Length -Sum).Sum
  if ($s) { return [int64]$s } else { return 0 }
}

# ------------------------------------------------------------------ the report

$Lines = New-Object System.Collections.Generic.List[string]
$Checks = New-Object System.Collections.Generic.List[object]
function Add-Line([string]$Text) { $Lines.Add($Text); Write-Host $Text }
function Add-Check([string]$Name, [bool]$Pass, [string]$Detail, [bool]$Required = $true) {
  $Checks.Add([pscustomobject]@{ Name = $Name; Pass = $Pass; Required = $Required; Detail = $Detail })
  $tag = if ($Pass) { 'PASS' } elseif ($Required) { 'FAIL' } else { 'INFO' }
  Add-Line ("[{0}] {1} - {2}" -f $tag, $Name, $Detail)
}

function Write-Environment {
  Add-Line "HealthierYou soak - role $Role$(if ($CiSelfTest -or $Sim) { ' (CI, accelerated)' }) - $((Get-Date).ToString('yyyy-MM-dd HH:mm'))"
  try {
    $os = Get-CimInstance Win32_OperatingSystem
    $cpuName = (Get-CimInstance Win32_Processor | Select-Object -First 1).Name
    $ram = [Math]::Round((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory / 1GB, 1)
    Add-Line ("Windows: {0} build {1}; processor: {2}; {3} logical processors; {4} GB memory" -f $os.Caption, $os.BuildNumber, $cpuName.Trim(), $Cores, $ram)
  }
  catch { Add-Line "Windows details unavailable: $($_.Exception.Message)" }
  Add-Line "Sampling: every $SampleSeconds s, whole process tree (WebView2 included), Win32_Process and Win32_PerfFormattedData_PerfProc_Process; CPU from process times over wall time and $Cores logical processors."
}

# Trend, crash and awake checks over the samples (after the warm-up).
function Test-Samples($Samples, [double]$WarmupFraction, [double]$NeedAwakeH) {
  $s = @(foreach ($x in $Samples) { $x })
  $dead = @($s | Where-Object { -not $_.roots_alive })
  Add-Check 'no crash or restart' ($dead.Count -eq 0 -and $s.Count -gt 0) $(if ($dead.Count) { "the main process was gone from $($dead[0].t_s) s" } else { "$($s.Count) samples, same main process throughout" })
  $secs = [double[]]@($s | ForEach-Object { $_.t_s })
  $aw = Get-AwakeTime $secs $SampleSeconds
  $lockedN = @($s | Where-Object { $_.locked }).Count
  if ($NeedAwakeH -gt 0) {
    Add-Check 'awake time covered' ($aw.AwakeS / 3600 -ge $NeedAwakeH * 0.95) ('{0:N2} h awake of {1} h asked; {2} sleep or suspend gaps ({3:N1} h); {4} samples while locked' -f ($aw.AwakeS / 3600), $NeedAwakeH, $aw.Gaps, ($aw.GapSeconds / 3600), $lockedN)
  }
  else { Add-Line ('Awake {0:N2} h; {1} gaps; {2} samples while locked' -f ($aw.AwakeS / 3600), $aw.Gaps, $lockedN) }
  $skip = [int][Math]::Floor($s.Count * $WarmupFraction)
  $w = @($s | Select-Object -Skip $skip | Where-Object { $_.roots_alive })
  $h = [double[]]@($w | ForEach-Object { $_.t_s / 3600 })
  foreach ($m in @(@('working set (MB)', 'working_set_mb', 'MemoryMB'), @('private bytes (MB)', 'private_bytes_mb', 'MemoryMB'), @('handles', 'handles', 'Handles'), @('threads', 'threads', 'Threads'))) {
    $vals = [double[]]@($w | ForEach-Object { $_[$m[1]] })
    $t = Test-Trend $m[0] $h $vals $Trend[$m[2]].Abs $Trend[$m[2]].Rel
    Add-Check "no growth trend: $($t.Name)" $t.Pass $t.Detail
  }
  return $w
}

function Save-Report($Extra) {
  $required = @($Checks | Where-Object { $_.Required })
  $ok = ($required.Count -gt 0) -and -not ($required | Where-Object { -not $_.Pass })
  Add-Line ''
  Add-Line "Overall: $(if ($ok) { 'PASS' } else { 'FAIL' })"
  Add-Line 'The report names no user, PC, path, address or device id. Check it before sharing.'
  if ($Report) {
    $dir = Split-Path -Parent $Report
    if ($dir) { New-Item -ItemType Directory -Force $dir | Out-Null }
    $Lines | Set-Content -Encoding UTF8 $Report
    $o = [ordered]@{ role = $Role; ok = $ok; checks = $Checks }
    foreach ($k in $Extra.Keys) { $o[$k] = $Extra[$k] }
    $o | ConvertTo-Json -Depth 6 | Set-Content -Encoding UTF8 ($Report + '.json')
  }
  return $ok
}

# --------------------------------------------------------------------- self-test

function Invoke-SelfTest {
  $bad = 0
  function Expect([string]$name, [bool]$got, [bool]$want) {
    if ($got -ne $want) { $script:selfBad++ }
    Write-Host ("[{0}] {1}" -f $(if ($got -eq $want) { 'ok' } else { 'WRONG' }), $name)
  }
  $script:selfBad = 0
  $hrs = [double[]](0..95 | ForEach-Object { $_ * 5 / 60 })
  $rand = New-Object Random 7
  $flat = [double[]]($hrs | ForEach-Object { 120 + $rand.NextDouble() * 6 })
  $leak = [double[]]($hrs | ForEach-Object { 120 + 8 * $_ + $rand.NextDouble() * 3 })
  $step = [double[]](0..95 | ForEach-Object { if ($_ -lt 30) { 120 } else { 160 } })
  $sawtooth = [double[]](0..95 | ForEach-Object { 120 + 30 * (($_ % 12) / 12) })
  Expect 'flat noisy memory passes' (Test-Trend 'm' $hrs $flat 20 0.1).Pass $true
  Expect 'memory growing 8 MB/h over 8 h fails' (Test-Trend 'm' $hrs $leak 20 0.1).Pass $false
  Expect 'a sawtooth (garbage collection) passes' (Test-Trend 'm' $hrs $sawtooth 20 0.1).Pass $true
  Expect 'a lasting step up of 40 MB fails' (Test-Trend 'm' $hrs $step 20 0.1).Pass $false
  $hs = [double[]](0..95 | ForEach-Object { 900 + ($_ % 5) })
  Expect 'steady handles pass' (Test-Trend 'h' $hrs $hs 200 0.15).Pass $true
  $hl = [double[]](0..95 | ForEach-Object { 900 + 5 * $_ })
  Expect 'handles growing 5 a sample fail' (Test-Trend 'h' $hrs $hl 200 0.15).Pass $false
  $secs = [double[]](0, 60, 120, 180, 7380, 7440, 7500)
  $a = Get-AwakeTime $secs 60
  Expect 'a 2-hour sleep is a gap, not awake time' ($a.Gaps -eq 1 -and $a.AwakeS -eq 300) $true
  $s30 = [double[]](0..60 | ForEach-Object { $_ * 60 })
  $calm = [double[]](0..60 | ForEach-Object { 0.5 })
  Expect 'calm CPU passes' (Test-Cpu $s30 $calm 2 5 3).Pass $true
  $busy = [double[]](0..60 | ForEach-Object { if ($_ -ge 20 -and $_ -lt 50) { 3.0 } else { 0.5 } })
  Expect 'a 30-minute average of 3% fails' (Test-Cpu $s30 $busy 2 5 3).Pass $false
  $spike = [double[]](0..60 | ForEach-Object { if ($_ -ge 10 -and $_ -lt 13) { 9.0 } else { 0.2 } })
  Expect 'three samples over 5% fail' (Test-Cpu $s30 $spike 2 5 3).Pass $false
  $blip = [double[]](0..60 | ForEach-Object { if ($_ -eq 10) { 30.0 } else { 0.2 } })
  Expect 'one short spike passes' (Test-Cpu $s30 $blip 2 5 3).Pass $true
  # Memory budget (DEC14): the private working set decides, the whole tree is information.
  $treeHigh = [double[]](0..95 | ForEach-Object { if ($_ -lt 64) { 218 + ($_ % 7) * 17 } else { 97 } })
  $privLow = [double[]](0..95 | ForEach-Object { 67 + ($_ % 5) })
  Expect 'a high whole-tree working set with low private working set passes' (Test-MemoryBudget $privLow $treeHigh 150).Pass $true
  Expect '... and the whole tree is reported' ((Test-MemoryBudget $privLow $treeHigh 150).Detail -like '*whole-tree working set*') $true
  $privHigh = [double[]](0..95 | ForEach-Object { 160 + ($_ % 5) })
  Expect 'a private working set median over 150 MB fails' (Test-MemoryBudget $privHigh $treeHigh 150).Pass $false
  $privPeak = [double[]](0..95 | ForEach-Object { if ($_ -eq 40) { 180 } else { 70 } })
  Expect 'one private working set sample over 150 MB fails' (Test-MemoryBudget $privPeak $treeHigh 150).Pass $false
  Expect 'no private working set samples fail' (Test-MemoryBudget ([object[]]@()) $treeHigh 150).Pass $false
  # Coverage: sparse readings, or a hole in them where memory may have been high, never pass.
  $sparse = New-Object 'object[]' 96
  for ($i = 0; $i -lt 96; $i++) { if ($i % 3 -eq 0) { $sparse[$i] = 70.0 } }
  Expect 'sparse private readings (a third of the samples) fail' (Test-MemoryBudget $sparse $treeHigh 150).Pass $false
  $hole = New-Object 'object[]' 96
  for ($i = 0; $i -lt 96; $i++) { if ($i -lt 40 -or $i -ge 50) { $hole[$i] = 70.0 } }
  $treeHole = [double[]](0..95 | ForEach-Object { if ($_ -ge 40 -and $_ -lt 50) { 480 } else { 120 } })
  Expect 'a high stretch with no private readings fails though the low samples around it pass' (Test-MemoryBudget $hole $treeHole 150).Pass $false
  $blipMiss = New-Object 'object[]' 96
  for ($i = 0; $i -lt 96; $i++) { if ($i -ne 40) { $blipMiss[$i] = 70.0 } }
  Expect 'one sample without a reading is tolerated' (Test-MemoryBudget $blipMiss $treeHigh 150).Pass $true
  # A sample counts only when every process of the tree has a reading.
  $rows = @([pscustomobject]@{ IDProcess = 10; WorkingSetPrivate = 50MB }, [pscustomobject]@{ IDProcess = 11; WorkingSetPrivate = 20MB }, [pscustomobject]@{ IDProcess = 99; WorkingSetPrivate = 500MB })
  $part = Get-PrivateWsSum @(10, 11, 12) $rows
  Expect 'a tree process missing from the counters leaves the sample without a private value' ($null -eq $part.Mb -and $part.Missing -eq 1) $true
  $full = Get-PrivateWsSum @(10, 11) $rows
  Expect 'a complete reading sums the tree only' ($full.Mb -eq 70 -and $full.Missing -eq 0) $true
  $ordered = New-Object System.Collections.Generic.List[object]
  $ordered.Add([ordered]@{ private_ws_mb = 60.0 }); $ordered.Add([ordered]@{ private_ws_mb = $null }); $ordered.Add([ordered]@{ working_set_mb = 1.0 })
  $pv = Get-PrivateValues $ordered
  Expect 'samples keep their place when they have no private value' ($pv.Count -eq 3 -and $pv[0] -eq 60 -and $null -eq $pv[1] -and $null -eq $pv[2]) $true
  # The samples as the sampling loops collect them: ordered dictionaries in a generic list.
  $list = New-Object System.Collections.Generic.List[object]
  foreach ($i in 0..11) { $list.Add([ordered]@{ t_s = 10.0 * $i; roots_alive = ($i -lt 10); locked = $false; working_set_mb = 100.0; private_bytes_mb = 80.0; handles = 500; threads = 30 }) }
  $kept = @(Remove-TrailingExit $list)
  Expect 'samples taken while the program exited are dropped' ($kept.Count -eq 10 -and $kept[9].t_s -eq 90) $true
  $before = $Checks.Count
  $w = @(Test-Samples $list 0.0 0)
  $new = @($Checks | Select-Object -Skip $before)
  Expect 'collected samples analyse; an exit mid-run is a crash' ($w.Count -eq 10 -and $new.Count -eq 5 -and -not $new[0].Pass -and -not ($new | Select-Object -Skip 1 | Where-Object { -not $_.Pass })) $true
  while ($Checks.Count -gt $before) { $Checks.RemoveAt($Checks.Count - 1) }
  if ($script:selfBad) { throw "Soak analysis self-test: $($script:selfBad) case(s) wrong" }
  Write-Host 'Soak analysis self-test: all cases right.'
}

if ($SelfTest) { Invoke-SelfTest; return }

# ----------------------------------------------------------- CI: companion soak

if ($CiSelfTest) {
  $SampleSeconds = [Math]::Min($SampleSeconds, 10)
  Write-Environment
  $exe = Join-Path $env:LOCALAPPDATA 'HealthierYou\hy-companion.exe'
  if (-not (Test-Path $exe)) { throw 'The companion is not installed' }
  $selfReport = Join-Path ([IO.Path]::GetTempPath()) "hy-soak-selftest-$PID.json"
  $env:HY_SELFTEST = $selfReport
  $env:HY_SELFTEST_SOAK_S = [string][int]($Minutes * 60)
  $env:HY_SELFTEST_SOAK_SIM_H = '72'
  $env:HY_SELFTEST_IDLE_S = '0'
  $start = Get-Date
  $p = Start-Process -FilePath $exe -PassThru
  $null = $p.Handle
  $samples = New-Object System.Collections.Generic.List[object]
  $deadline = $start.AddMinutes($Minutes * 2 + 10)
  while (-not $p.HasExited -and (Get-Date) -lt $deadline) {
    Start-Sleep -Seconds $SampleSeconds
    if ($p.HasExited) { break }
    $x = Get-ProcessSample @($p.Id) $start
    $x['utc'] = (Get-Date).ToUniversalTime().ToString('o')
    $samples.Add($x)
  }
  if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force; Add-Check 'the app finished its soak' $false "still running after $([int]($Minutes * 2 + 10)) minutes" }
  $p.WaitForExit()
  $samples = Remove-TrailingExit $samples
  foreach ($v in 'HY_SELFTEST', 'HY_SELFTEST_SOAK_S', 'HY_SELFTEST_SOAK_SIM_H', 'HY_SELFTEST_IDLE_S') { Remove-Item "Env:$v" -ErrorAction SilentlyContinue }
  $app = if (Test-Path $selfReport) { Get-Content -Raw $selfReport | ConvertFrom-Json } else { $null }
  Add-Check 'the app''s self-test and soak checks' ($p.ExitCode -eq 0 -and $app -and $app.ok) "exit code $($p.ExitCode)"
  if ($app) {
    foreach ($c in @($app.checks | Where-Object { $_.name -like 'soak_*' })) { Add-Line ("       app check {0}: {1} - {2}" -f $c.name, $(if ($c.ok) { 'ok' } else { 'FAILED' }), $c.detail) }
    foreach ($c in @($app.checks | Where-Object { -not $_.ok -and $_.required })) { Add-Line ("       failed app check {0}: {1}" -f $c.name, $c.detail) }
  }
  # Only the soak phase's samples count (the self-test before it opens and closes windows).
  $window = @($samples)
  if ($app -and $app.soak) {
    $from = [datetime]::Parse($app.soak.started_utc).ToUniversalTime()
    $to = [datetime]::Parse($app.soak.ended_utc).ToUniversalTime()
    $window = @($samples | Where-Object { $u = [datetime]::Parse($_.utc).ToUniversalTime(); $u -ge $from -and $u -le $to })
    Add-Line "Soak phase: $($window.Count) of $($samples.Count) samples; $($app.soak.reminders_shown) reminders shown, $($app.soak.window_cycles) window cycles, $($app.soak.locks) locks, $($app.soak.sleeps) sleeps"
  }
  else { Add-Check 'the app ran its soak phase' $false 'no soak summary in the app report' }
  $w = Test-Samples $window 0.2 0
  $ws = [double[]]@($w | ForEach-Object { $_.working_set_mb })
  $mem = Test-MemoryBudget (Get-PrivateValues $w) $ws $Budget.WorkingSetMB
  Add-Check 'memory budget: app''s own memory (informational on a CI runner)' $mem.Pass ('{0} (PRD budget {1} MB idle; the soak is not idle and a runner is not an office PC)' -f $mem.Detail, $Budget.WorkingSetMB) $false
  $ok = Save-Report @{ samples = $samples; app = $app }
  Remove-Item $selfReport -ErrorAction SilentlyContinue
  if (-not $ok) { exit 1 }
  return
}

# -------------------------------------------------------------- CI: server soak

if ($Sim) {
  $SampleSeconds = [Math]::Min($SampleSeconds, 10)
  Write-Environment
  $simReport = Join-Path ([IO.Path]::GetTempPath()) "hy-soak-sim-$PID.json"
  $simOut = Join-Path ([IO.Path]::GetTempPath()) "hy-soak-sim-$PID.out"
  $start = Get-Date
  $p = Start-Process -FilePath $Sim -ArgumentList @('soak', [string][int]($Minutes * 60), [string]$SimDays, "`"$simReport`"") -PassThru -NoNewWindow -RedirectStandardOutput $simOut
  $null = $p.Handle
  $samples = New-Object System.Collections.Generic.List[object]
  $deadline = $start.AddMinutes($Minutes * 2 + 5)
  while (-not $p.HasExited -and (Get-Date) -lt $deadline) {
    Start-Sleep -Seconds $SampleSeconds
    if ($p.HasExited) { break }
    $samples.Add((Get-ProcessSample @($p.Id) $start))
  }
  if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force; Add-Check 'the simulator finished' $false 'still running at the deadline' }
  $p.WaitForExit()
  $samples = Remove-TrailingExit $samples
  if (Test-Path $simOut) { Get-Content $simOut | ForEach-Object { Add-Line "       $_" } }
  $simJson = if (Test-Path $simReport) { Get-Content -Raw $simReport | ConvertFrom-Json } else { $null }
  Add-Check 'hy-sim soak: every row, backup, write-ahead log and sync-time check' ($p.ExitCode -eq 0 -and $simJson -and $simJson.ok) "exit code $($p.ExitCode); $(if ($simJson) { @($simJson.failures).Count } else { 'no' }) failures"
  if ($simJson) { foreach ($f in @($simJson.failures)) { Add-Line "       $f" } }
  $null = Test-Samples $samples 0.2 0
  $ok = Save-Report @{ samples = $samples; sim = $simJson }
  Remove-Item $simReport, $simOut -ErrorAction SilentlyContinue
  if (-not $ok) { exit 1 }
  return
}

# ------------------------------------------------------- office PC: companion

$start = Get-Date
$startMs = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
Write-Environment
$samples = New-Object System.Collections.Generic.List[object]
$statsLog = New-Object System.Collections.Generic.List[object]

if ($Role -eq 'Companion') {
  $root = Join-Path $env:LOCALAPPDATA 'HealthierYou'
  $exe = Join-Path $root 'hy-companion.exe'
  $db = Join-Path $root 'data\hy.db'
  if (-not (Test-Path $exe)) {
    if (-not $Installer) { throw 'HealthierYou is not installed: pass -Installer with the setup .exe (and its .sig beside it)' }
    Add-Line 'Installing per user (silent)...'
    $i = Start-Process -FilePath $Installer -ArgumentList '/S' -PassThru -Wait
    if ($i.ExitCode -ne 0 -or -not (Test-Path $exe)) { throw "The installer did not install the companion (exit code $($i.ExitCode))" }
  }
  $version = (Get-Item $exe).VersionInfo.ProductVersion
  Add-Line "Companion version: $version"
  function Get-Main { @(Get-CimInstance Win32_Process -Filter "Name = 'hy-companion.exe'" | Where-Object { $p = $_; -not (Get-CimInstance Win32_Process -Filter "ProcessId = $($p.ParentProcessId)" | Where-Object { $_.Name -eq 'hy-companion.exe' }) } | Sort-Object CreationDate | Select-Object -First 1) }
  $main = Get-Main
  if (-not $main) {
    Add-Line 'Starting the companion...'
    Start-Process -FilePath $exe | Out-Null
    Start-Sleep -Seconds 30
    $main = Get-Main
  }
  if (-not $main) { throw 'The companion is not running and did not start' }
  $mainId = [int]$main[0].ProcessId
  $updateLog = Join-Path $root 'updates\update.log'
  $firstLog = Get-Size $updateLog
  # The store statistics (counts, sizes and times only), read by the app itself. A build
  # older than V1-7 has no such command and would start the app instead: give it 60 s.
  function Get-StoreStats {
    $file = Join-Path ([IO.Path]::GetTempPath()) "hy-soak-stats-$PID.json"
    Remove-Item $file -ErrorAction SilentlyContinue
    $p = Start-Process -FilePath $exe -ArgumentList @('--hy-soak-stats', [string]$startMs, '--report', "`"$file`"") -PassThru
    if (-not $p.WaitForExit(60000)) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue; return $null }
    if (-not (Test-Path $file)) { return $null }
    $o = Get-Content -Raw $file | ConvertFrom-Json
    Remove-Item $file -ErrorAction SilentlyContinue
    return $o
  }
  $needS = $Hours * 3600
  $capS = $needS * 2 + 1800
  $nextStats = 0
  $firstFiles = $null
  Add-Line "Sampling for $Hours h of awake time (at most $([Math]::Round($capS / 3600, 1)) h in all). Leave this window open."
  while ($true) {
    $x = Get-ProcessSample @($mainId) $start
    $x['db_bytes'] = Get-Size $db
    $x['wal_bytes'] = Get-Size ($db + '-wal')
    $x['update_log_bytes'] = Get-Size $updateLog
    if (-not $firstFiles) { $firstFiles = $x }
    $samples.Add($x)
    if ($x.t_s -ge $nextStats) {
      $st = Get-StoreStats
      if ($st) { $statsLog.Add([ordered]@{ t_s = $x.t_s; stats = $st }) }
      $nextStats = $x.t_s + 900
    }
    if (-not $x.roots_alive) { Add-Line "The companion stopped at $($x.t_s) s."; break }
    $aw = Get-AwakeTime ([double[]]@($samples | ForEach-Object { $_.t_s })) $SampleSeconds
    if ($aw.AwakeS -ge $needS -or $x.t_s -ge $capS) { break }
    Start-Sleep -Seconds $SampleSeconds
  }
  $final = Get-StoreStats
  if ($final) { $statsLog.Add([ordered]@{ t_s = $samples[$samples.Count - 1].t_s; stats = $final }) }
  $w = Test-Samples $samples 0.1 $Hours
  $ws = [double[]]@($w | ForEach-Object { $_.working_set_mb })
  $mem = Test-MemoryBudget (Get-PrivateValues $w) $ws $Budget.WorkingSetMB
  Add-Check "PRD budget: app's own memory (private working set) at most $($Budget.WorkingSetMB) MB" $mem.Pass $mem.Detail
  $cpuS = @($w | Where-Object { $null -ne $_.cpu_pct })
  $cpu = Test-Cpu ([double[]]@($cpuS | ForEach-Object { $_.t_s })) ([double[]]@($cpuS | ForEach-Object { $_.cpu_pct })) $Budget.CpuAvgPct $Budget.CpuSpikePct $Budget.SpikeSamples
  Add-Check 'PRD budget: CPU (2% average, no sustained spike over 5%)' $cpu.Pass $cpu.Detail
  $last = $samples[$samples.Count - 1]
  $walMax = ($samples | ForEach-Object { $_.wal_bytes } | Measure-Object -Maximum).Maximum
  $growth = ($last.db_bytes + $last.wal_bytes) - ($firstFiles.db_bytes + $firstFiles.wal_bytes)
  Add-Check 'store: write-ahead log and database growth bounded' ($walMax -le $Budget.WalMaxBytes -and $growth -le $Budget.DbGrowthBytes) ('write-ahead log at most {0:N0} bytes (bound {1:N0}); database and log grew {2:N0} bytes (bound {3:N0})' -f $walMax, $Budget.WalMaxBytes, $growth, $Budget.DbGrowthBytes)
  $logGrowth = $last.update_log_bytes - $firstLog
  Add-Check 'log size bounded' ($logGrowth -le $Budget.LogGrowthBytes) ("update log grew $logGrowth bytes. The companion keeps no other log yet (architecture 10's JSON-line log is tracked work in docs/V1_Acceptance.md)")
  if ($final) {
    $gap = $final.min_offer_gap_min
    $ok = ($null -eq $gap -or $gap -ge $Budget.MinOfferGapMin) -and $final.max_offers_in_an_hour -le $Budget.MaxOffersPerHour
    $types = (@($final.events_by_type.PSObject.Properties | ForEach-Object { "$($_.Name) $($_.Value)" }) -join ', ')
    Add-Check 'reminder cadence: no burst' $ok ("{0} reminders; shortest gap {1} min (calm gap {2}); at most {3} in an hour (bound {4}); events: {5}" -f @($final.offers_at_min).Count, $(if ($null -eq $gap) { 'n/a' } else { $gap }), $Budget.MinOfferGapMin, $final.max_offers_in_an_hour, $Budget.MaxOffersPerHour, $(if ($types) { $types } else { 'none' }))
  }
  else { Add-Check 'reminder cadence: store statistics read' $false 'hy-companion.exe --hy-soak-stats gave no answer (a build from before V1-7?)' }
  $ok = Save-Report @{ samples = $samples; store_stats = $statsLog }
  if (-not $ok) { exit 1 }
  return
}

# ---------------------------------------------------------- office PC: server

$admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $admin) { throw 'Run the server soak in an elevated PowerShell (the service and its data folder are administrator-only).' }
if (-not $ServerData) { $ServerData = Join-Path $env:ProgramData 'HealthierYou\Server' }
$svc = Get-CimInstance Win32_Service -Filter "Name = 'HealthierYouServer'"
if (-not $svc -or $svc.State -ne 'Running') { throw 'The HealthierYouServer service is not running' }
$svcId = [int]$svc.ProcessId
$mirrorFile = $null
$loc = Join-Path $ServerData 'mirror-location.txt'
if (Test-Path $loc) { $mirrorFile = Join-Path ((Get-Content -Raw $loc).Trim()) 'control-log.mirror' }
$logs = Join-Path $ServerData 'logs'
$backups = Join-Path $ServerData 'backups'
Add-Line "Server version: $((Get-ItemProperty 'HKLM:\Software\HealthierYou\Server' -ErrorAction SilentlyContinue).Version)"
$needS = $Hours * 3600
$first = $null
while ($true) {
  $x = Get-ProcessSample @($svcId) $start
  $x['db_bytes'] = Get-Size (Join-Path $ServerData 'server.db')
  $x['wal_bytes'] = Get-Size (Join-Path $ServerData 'server.db-wal')
  $x['log_bytes'] = Get-FolderSize $logs 'server.log*'
  $x['mirror_bytes'] = Get-Size $mirrorFile
  $x['backups'] = @(Get-ChildItem -LiteralPath $backups -File -ErrorAction SilentlyContinue).Count
  $x['backup_bytes'] = Get-FolderSize $backups
  if (-not $first) { $first = $x }
  $samples.Add($x)
  if (-not $x.roots_alive) { Add-Line "The service process ended at $($x.t_s) s."; break }
  $aw = Get-AwakeTime ([double[]]@($samples | ForEach-Object { $_.t_s })) $SampleSeconds
  if ($aw.AwakeS -ge $needS -or $x.t_s -ge $needS * 2 + 1800) { break }
  Start-Sleep -Seconds $SampleSeconds
}
$null = Test-Samples $samples 0.1 $Hours
$last = $samples[$samples.Count - 1]
$walMax = ($samples | ForEach-Object { $_.wal_bytes } | Measure-Object -Maximum).Maximum
Add-Check 'server store: write-ahead log bounded' ($walMax -le $Budget.WalMaxBytes) ('at most {0:N0} bytes (bound {1:N0}); database {2:N0} to {3:N0} bytes; mirror {4:N0} to {5:N0} bytes' -f $walMax, $Budget.WalMaxBytes, $first.db_bytes, $last.db_bytes, $first.mirror_bytes, $last.mirror_bytes)
Add-Check 'server log within rotation' (($last.log_bytes - $first.log_bytes) -le $Budget.LogGrowthBytes) ("grew $($last.log_bytes - $first.log_bytes) bytes")
Add-Check 'backups kept within retention' ($last.backups -le 16) ("$($last.backups) files (14 daily and 2 weekly at most), $($last.backup_bytes) bytes")
# The server's own log: requests, errors and panics since the start (endpoint names,
# statuses and latencies only, by design).
$requests = 0; $server5xx = 0; $panics = 0; $lat = New-Object System.Collections.Generic.List[double]; $endpoints = @{}
foreach ($f in @(Get-ChildItem -LiteralPath $logs -File -Filter 'server.log*' -ErrorAction SilentlyContinue)) {
  foreach ($line in (Read-SharedLines $f.FullName)) {
    try { $o = $line | ConvertFrom-Json } catch { continue }
    if (-not $o.timestamp -or [datetime]::Parse($o.timestamp).ToUniversalTime() -lt $start.ToUniversalTime()) { continue }
    $ev = $o.fields.event
    if ($ev -eq 'panic') { $panics++ }
    if ($ev -ne 'request') { continue }
    $d = $o.fields.detail | ConvertFrom-Json
    $requests++
    $endpoints[$d.endpoint] = 1 + [int]$endpoints[$d.endpoint]
    if ([int]$d.status -ge 500) { $server5xx++ }
    $lat.Add([double]$d.latency_ms)
  }
}
$p95 = if ($lat.Count) { ($lat | Sort-Object)[[int][Math]::Min($lat.Count - 1, [Math]::Floor($lat.Count * 0.95))] } else { 0 }
Add-Check 'server log: no panic and no 5xx answer' ($panics -eq 0 -and $server5xx -eq 0) ("$requests requests ($((@($endpoints.Keys | Sort-Object | ForEach-Object { "$_ $($endpoints[$_])" })) -join ', ')); p95 $p95 ms; 5xx $server5xx; panics $panics")
$ok = Save-Report @{ samples = $samples }
if (-not $ok) { exit 1 }
