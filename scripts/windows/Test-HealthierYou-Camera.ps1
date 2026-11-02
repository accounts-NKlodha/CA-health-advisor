<#
.SYNOPSIS
  Guided checks of the HealthierYou posture check's camera on a real PC with a real webcam
  (slices V2-1, V2-3 and V2-5, "needs physical testing"). CI runs the camera page on a
  synthetic camera; this covers what only a real webcam, a person and time can show.

.DESCRIPTION
  Run it in Windows PowerShell 5.1 or PowerShell 7 as the office user, with HealthierYou
  installed and running. It samples the companion's process tree (working set, private
  bytes and CPU, the whole PC's share) every few seconds into a CSV, and asks a yes/no
  question for each thing only a person can see. It never takes, stores or sends a picture:
  the camera is used only by HealthierYou itself. The report and CSV hold the Windows
  build, the results, your notes and the measurements: no user name, PC name, folder path
  or serial number. Read them before sharing.

  Phases:
    Session   (V2-1, V2-3; about 20 minutes) consent, CAM badge, reading at a desk, no
              reading when away or covered, camera taken by another app, Windows camera
              access off, presentation and meeting holds, lock and sleep, Off within two
              seconds, a second webcam unplugged and plugged.
    Soak      (V2-5; 2 hours by default) leave the posture check on during normal work;
              the script only measures, then asks four questions at the end.
    Calibration (V2-2; about 15 minutes) calibrate, calm desk work with no cue, a
              sustained slump that gives one cue, a moved camera that pauses cues and asks
              to recalibrate, and an app restart at the same desk.
    Desk      (V2-3; about 40 minutes) one labelled desk session of the PRD V2 gate: the
              script tells you what to do and when (calibrate, comfortable reading and
              typing, a second display, reaches, five 90-second staged drifts, away,
              covered, a lock, the camera moved), writes the labels (seconds and a word,
              nothing else), then asks HealthierYou for the session's raw counts from its
              own posture records (hy-companion.exe --hy-posture-report). Run it for each
              of 5 setups x 2 sessions: -Setup 1..5 -Session 1..2.
    DeskReport (V2-3) adds up every hy-desk-*.json in the report's folder (or
              -SessionsDir) per setup and checks the PRD V2 gate: coverage at least 70%,
              at least 10 staged drifts per setup with 80% detected and the cue within
              30 s, at most 1 false cue per 30 minutes of comfortable work, absence and
              occlusion shown as no reading, a moved camera paused.

  The Desk phase never records a picture, a landmark or a posture number: only the times
  you were asked to do something, and the counts HealthierYou derives from its own records
  on this PC. Nothing is sent anywhere. The thresholds it measures are provisional until
  these sessions pass and the owner approves them (docs/V2_Quality.md).

  Budget (architecture 4.5, PRD V2): the camera adds at most 10% of the PC's CPU, and the
  app's own memory with the camera on is at most 350 MB, counted by DEC14 as the private
  working set summed over the process tree (every process read, readings covering the
  samples). The whole-tree working set the camera adds (on minus off) is reported beside
  it as information only.

.EXAMPLE
  .\Test-HealthierYou-Camera.ps1 -Phase Session -Report .\hy-camera-session.txt
  .\Test-HealthierYou-Camera.ps1 -Phase Soak -Minutes 120 -Report .\hy-camera-soak.txt
  .\Test-HealthierYou-Camera.ps1 -Phase Calibration -Report .\hy-camera-calibration.txt
  .\Test-HealthierYou-Camera.ps1 -Phase Desk -Setup 1 -Session 1 -Report .\desk\hy-desk-1-1.txt
  .\Test-HealthierYou-Camera.ps1 -Phase DeskReport -Report .\desk\hy-desk-gate.txt
  .\Test-HealthierYou-Camera.ps1 -SelfTest
#>
[CmdletBinding()]
param(
  [ValidateSet('Session', 'Soak', 'Calibration', 'Desk', 'DeskReport')][string]$Phase = 'Session',
  [int]$Minutes = 120,
  [int]$SampleSeconds = 5,
  [ValidateRange(1, 5)][int]$Setup = 1,
  [ValidateRange(1, 2)][int]$Session = 1,
  [string]$SessionsDir,
  [string]$Exe,
  [switch]$SelfTest,
  [string]$Report
)
$ErrorActionPreference = 'Stop'

# ------------------------------------------------------------------ desk session (V2-3)

# The desk protocol (docs/V2_Quality.md; the same steps as the synthetic sessions in
# crates/hy-core/src/posture/desk.rs). Each step: the label, seconds, what to do. A step
# with 0 seconds lasts until you press Enter again (the lock).
function Get-DeskSteps([int]$SessionNo) {
  $last = if ($SessionNo % 2 -eq 0) { 'drift_forward' } else { 'drift_lateral' }
  $move = if ($SessionNo % 2 -eq 1) { 'Push the laptop back about a hand width (or move the external camera back)' } else { 'Tilt the screen back clearly (or tilt the external camera up)' }
  @(
    @('calibrate', 30, 'On the camera page choose Calibrate (Recalibrate) and sit as you normally do, still, facing the screen.'),
    @('reading', 240, 'Read something on screen comfortably, as you normally sit.'),
    @('drift_forward', 90, 'Slump or lean clearly towards the screen and STAY like that until the beep.'),
    @('typing', 240, 'Type comfortably (an email, notes), as you normally sit.'),
    @('second_display', 75, 'Work while glancing at a second display or a document beside you now and then.'),
    @('drift_lateral', 90, 'Lean clearly to one side (elbow on the armrest, head tilted) and STAY until the beep.'),
    @('reach', 10, 'Reach for something on the desk (a cup, a phone), then sit back.'),
    @('typing', 120, 'Type comfortably.'),
    @('reach', 10, 'Stretch your arms or reach for something, then sit back.'),
    @('absent', 45, 'Stand up and step out of the camera view; come back at the beep.'),
    @('reading', 180, 'Read comfortably.'),
    @('drift_forward', 90, 'Slump or lean towards the screen and STAY until the beep.'),
    @('paused', 0, 'Lock the PC (Windows+L), wait about a minute, unlock, then press Enter here.'),
    @('typing', 180, 'Type comfortably.'),
    @('occluded', 30, 'Hold a hand or a sheet of paper in front of your face (not over the whole camera) until the beep.'),
    @('second_display', 75, 'Work while glancing at a second display or a document now and then.'),
    @('drift_lateral', 90, 'Lean clearly to one side and STAY until the beep.'),
    @('reading', 180, 'Read comfortably.'),
    @($last, 90, 'Drift again (slump forward or lean sideways as it says) and STAY until the beep.'),
    @('typing', 120, 'Type comfortably.'),
    @('relocated', 160, "$move. Press Enter once it is in its new place, then sit as usual until the beep."),
    @('calibrate', 30, 'Put the camera back as it was, then choose Calibrate on the camera page and sit still.'),
    @('typing', 60, 'Type comfortably.')
  )
}

# One label line: seconds from the session start (3 decimals) and the word. Nothing else.
function Format-Label([long]$SinceMs, [long]$StartMs, [long]$EndMs, [string]$Label) {
  $inv = [Globalization.CultureInfo]::InvariantCulture
  '{0},{1},{2}' -f (($StartMs - $SinceMs) / 1000.0).ToString('0.000', $inv), (($EndMs - $SinceMs) / 1000.0).ToString('0.000', $inv), $Label
}

function Get-NowMs { [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() }

# The report's full path (relative to the current location), with its folder created if it
# is not there yet (for example .\desk\hy-desk-1-1.txt), so every file beside it can be written.
function Initialize-ReportPath([string]$Path) {
  $full = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Path)
  $parent = Split-Path -Parent $full
  if ($parent -and -not (Test-Path -LiteralPath $parent -PathType Container)) {
    $null = New-Item -ItemType Directory -Path $parent -Force
  }
  $full
}

# ------------------------------------------------------------------ memory budget (DEC14)

# The app's own memory in one sample (DEC14), as Test-CameraSoak.ps1 counts it: the private
# working set summed over the process tree, only when EVERY process of the tree has a
# reading (a process missing from the counters would lower the sum unseen). Same rule as
# Get-PrivateWsSum there; keep the two in step.
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

# DEC14's coverage rule (as Get-PrivateCoverage in Test-CameraSoak.ps1): complete readings
# in at least 80% of the samples, in time order, and never more than 2 in a row missing.
function Get-PrivateCoverage([object[]]$Values) {
  $have = 0; $run = 0; $longest = 0
  foreach ($v in $Values) {
    if ($null -ne $v) { $have++; $run = 0 } else { $run++; if ($run -gt $longest) { $longest = $run } }
  }
  $n = $Values.Count
  $share = if ($n -gt 0) { $have / $n } else { 0 }
  return [pscustomobject]@{
    Ok = ($n -gt 0 -and $share -ge 0.8 -and $longest -le 2)
    Detail = ('complete private readings in {0} of {1} samples ({2:N0}%, at least 80%), at most {3} in a row without one (at most 2)' -f $have, $n, (100 * $share), $longest)
  }
}

# The V2-1 memory budget, reconciled to DEC14: the app's own memory (private working set
# over the process tree, companion and inference together) at every sample with the camera
# on, at most 350 MB; the highest sample counts and the median is reported. A reading over
# the cap fails; otherwise the readings must cover the samples or it is INCONCLUSIVE. The
# whole-tree working set the camera adds (on minus off, the measure this check used before
# DEC14) is reported beside it, labelled as information only.
function Get-MemoryBudgetResult([object[]]$PrivateWs, [double]$OnWsMax, [double]$OffWsMax, [double]$CapMb = 350) {
  $cov = Get-PrivateCoverage $PrivateWs
  $vals = [double[]]@($PrivateWs | Where-Object { $null -ne $_ })
  $added = [Math]::Round($OnWsMax - $OffWsMax, 1)
  $info = "added whole-tree working set (on minus off) $added MB, informational vs DEC14"
  if ($vals.Count -eq 0) {
    return [pscustomobject]@{ Outcome = 'INCONCLUSIVE'; Note = "app's own memory (private working set, DEC14): no complete reading with the camera on; $($cov.Detail); $info" }
  }
  $sorted = @($vals | Sort-Object)
  $mid = [int][Math]::Floor($sorted.Count / 2)
  $median = if ($sorted.Count % 2) { $sorted[$mid] } else { ($sorted[$mid - 1] + $sorted[$mid]) / 2 }
  $max = ($vals | Measure-Object -Maximum).Maximum
  $outcome = if ($max -gt $CapMb) { 'FAIL' } elseif (-not $cov.Ok) { 'INCONCLUSIVE' } else { 'PASS' }
  [pscustomobject]@{
    Outcome = $outcome
    Note = ("app's own memory (private working set over the process tree, DEC14) highest {0:N1} MB, median {1:N1} MB (budget {2} MB, companion and inference together); {3}; {4}" -f $max, $median, $CapMb, $cov.Detail, $info)
  }
}

function Invoke-DeskSelfTest {
  $fails = 0
  function Check([string]$Name, [bool]$Ok) { if ($Ok) { Write-Host "[PASS] $Name" } else { Write-Host "[FAIL] $Name"; $script:deskFails++ } }
  $script:deskFails = 0
  foreach ($n in 1, 2) {
    $steps = @(Get-DeskSteps $n)
    $drifts = @($steps | Where-Object { $_[0] -like 'drift_*' }).Count
    Check "session $n has 5 staged drifts" ($drifts -eq 5)
    $timed = ($steps | ForEach-Object { $_[1] } | Measure-Object -Sum).Sum
    Check "session $n is about 38 minutes of timed steps ($timed s)" ($timed -ge 2100 -and $timed -le 2400)
    $known = 'calibrate', 'reading', 'typing', 'second_display', 'reach', 'drift_forward', 'drift_lateral', 'absent', 'occluded', 'relocated', 'paused'
    Check "session $n uses only known labels" (@($steps | Where-Object { $known -notcontains $_[0] }).Count -eq 0)
  }
  $line = Format-Label 1000000 1012345 1102345 'drift_forward'
  Check "a label line is seconds,seconds,word ($line)" ($line -eq '12.345,102.345,drift_forward')
  $text = (@(Get-DeskSteps 1) | ForEach-Object { $_[2] }) -join ' '
  foreach ($w in 'bad posture', 'poor', 'wrong', 'health') { Check "no judgement in the instructions: $w" (-not $text.ToLower().Contains($w)) }
  # The V2-1 memory budget (DEC14): private working set over the tree, coverage rule, and the
  # whole-tree working set the camera adds shown as information only.
  $s = Get-PrivateWsSum @(10, 11) @([pscustomobject]@{ IDProcess = 10; WorkingSetPrivate = 100MB }, [pscustomobject]@{ IDProcess = 11; WorkingSetPrivate = 50MB }, [pscustomobject]@{ IDProcess = 99; WorkingSetPrivate = 900MB })
  Check 'private working set: summed over the tree only (other processes ignored)' ($s.Mb -eq 150 -and $s.Missing -eq 0)
  $s = Get-PrivateWsSum @(10, 11) @([pscustomobject]@{ IDProcess = 10; WorkingSetPrivate = 100MB })
  Check 'private working set: a process without a reading leaves the sample without a value' ($null -eq $s.Mb -and $s.Missing -eq 1)
  $ok = @(1..36 | ForEach-Object { 200 + ($_ % 5) })
  $m = Get-MemoryBudgetResult $ok 520 120
  Check "budget: private working set under 350 MB passes though the whole tree adds 400 MB ($($m.Outcome))" ($m.Outcome -eq 'PASS')
  Check 'budget: the report names the measure (private working set, DEC14) and the cap' ($m.Note -like "*app's own memory (private working set over the process tree, DEC14) highest 204*" -and $m.Note -like '*budget 350 MB*')
  Check 'budget: the added whole-tree working set is labelled informational vs DEC14' ($m.Note -like '*added whole-tree working set (on minus off) 400 MB, informational vs DEC14*')
  $spike = @($ok); $spike[20] = 380
  Check 'budget: one sample over 350 MB fails though the median is far below' ((Get-MemoryBudgetResult $spike 400 120).Outcome -eq 'FAIL')
  $sparse = New-Object System.Collections.Generic.List[object]
  foreach ($i in 1..36) { if ($i % 3 -eq 0) { $sparse.Add(210) } else { $sparse.Add($null) } }
  Check 'budget: readings in a third of the samples are INCONCLUSIVE' ((Get-MemoryBudgetResult $sparse.ToArray() 300 120).Outcome -eq 'INCONCLUSIVE')
  $hole = New-Object System.Collections.Generic.List[object]
  foreach ($i in 1..36) { if ($i -ge 10 -and $i -le 12) { $hole.Add($null) } else { $hole.Add(210) } }
  Check 'budget: three samples in a row without a reading are INCONCLUSIVE (92% covered)' ((Get-MemoryBudgetResult $hole.ToArray() 300 120).Outcome -eq 'INCONCLUSIVE')
  $blip = New-Object System.Collections.Generic.List[object]
  foreach ($i in 1..36) { if ($i -eq 10 -or $i -eq 20) { $blip.Add($null) } else { $blip.Add(210) } }
  Check 'budget: an isolated missing reading still passes' ((Get-MemoryBudgetResult $blip.ToArray() 300 120).Outcome -eq 'PASS')
  $none = Get-MemoryBudgetResult @($null, $null, $null) 300 120
  Check 'budget: no reading at all is INCONCLUSIVE and still shows the added working set as informational' ($none.Outcome -eq 'INCONCLUSIVE' -and $none.Note -like '*informational vs DEC14*')
  # A report in a folder that does not exist yet: the folder is made, and files beside the
  # report can be written.
  $tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("hy-camera-selftest-" + [guid]::NewGuid().ToString('N'))
  try {
    $full = Initialize-ReportPath (Join-Path (Join-Path $tmp 'desk') 'hy-desk-1-1.txt')
    Check 'a missing report folder is created' (Test-Path -LiteralPath (Join-Path $tmp 'desk') -PathType Container)
    $csvPath = [System.IO.Path]::ChangeExtension($full, '.csv')
    'time,label' | Set-Content $csvPath
    Check 'the measurements file beside the report can be written' (Test-Path -LiteralPath $csvPath)
    Push-Location $tmp
    try {
      $rel = Initialize-ReportPath (Join-Path (Join-Path '.' 'desk2') 'r.txt')
      Check 'a relative report path is made full, from the current location' ($rel -eq (Join-Path (Join-Path $tmp 'desk2') 'r.txt') -and (Test-Path -LiteralPath (Join-Path $tmp 'desk2') -PathType Container))
    } finally { Pop-Location }
  } finally {
    Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
  }
  if ($script:deskFails -gt 0) { throw "$script:deskFails self-test check(s) failed" }
  Write-Host 'Self-test passed.'
}

if ($SelfTest) { Invoke-DeskSelfTest; return }
if (-not $Report) { throw 'Give -Report <file> (or -SelfTest).' }
$Report = Initialize-ReportPath $Report
if (-not $Exe) { $Exe = Join-Path $env:LOCALAPPDATA 'HealthierYou\hy-companion.exe' }

$lines = [System.Collections.Generic.List[string]]::new()
function Add-Line([string]$Text) { $lines.Add($Text); Write-Host $Text }
function Result([string]$Name, [string]$Outcome, [string]$Note = '') {
  $clean = $Note -replace '[A-Za-z]:\\[^\s"]*', '<path>' -replace [regex]::Escape($env:USERNAME), '<user>' -replace [regex]::Escape($env:COMPUTERNAME), '<pc>'
  Add-Line ("[{0}] {1}{2}" -f $Outcome, $Name, $(if ($clean) { " - $clean" } else { '' }))
}
function Ask([string]$Name, [string]$Question) {
  Write-Host ''
  Write-Host $Question -ForegroundColor Cyan
  $a = Read-Host '  y = yes, n = no, s = skip'
  $outcome = switch -Regex ($a) { '^[yY]' { 'PASS' } '^[nN]' { 'FAIL' } default { 'SKIP' } }
  $note = ''
  if ($outcome -eq 'FAIL') { $note = Read-Host '  What did you see? (no names or paths)' }
  Result $Name $outcome $note
}
function Do-Step([string]$Text) {
  Write-Host ''
  Write-Host $Text -ForegroundColor Yellow
  $null = Read-Host '  Press Enter when done'
}

# ------------------------------------------------------------------ measurements

$cores = [Environment]::ProcessorCount
function Get-Tree {
  $all = Get-CimInstance Win32_Process
  $root = $all | Where-Object { $_.Name -eq 'hy-companion.exe' } | Select-Object -First 1
  if (-not $root) { return @() }
  $ids = @($root.ProcessId); $added = $true
  while ($added) {
    $added = $false
    foreach ($p in $all) {
      if ($ids -contains $p.ParentProcessId -and -not ($ids -contains $p.ProcessId)) { $ids += $p.ProcessId; $added = $true }
    }
  }
  $all | Where-Object { $ids -contains $_.ProcessId }
}
function Get-Usage {
  $tree = @(Get-Tree)
  # The app's own memory (DEC14): private working set over the tree, $null unless every
  # process of the tree has a reading.
  $pws = $null
  try {
    $ids = [int[]]@($tree | ForEach-Object { [int]$_.ProcessId })
    $pws = (Get-PrivateWsSum $ids @(Get-CimInstance Win32_PerfFormattedData_PerfProc_Process -Property IDProcess, WorkingSetPrivate)).Mb
  }
  catch { }
  [pscustomobject]@{
    processes = $tree.Count
    # Whole-tree working set: shared pages included (information only, DEC14).
    ws_mb     = [math]::Round((($tree | Measure-Object WorkingSetSize -Sum).Sum) / 1MB, 1)
    priv_ws_mb = $pws
    priv_mb   = [math]::Round((($tree | Measure-Object PrivatePageCount -Sum).Sum) / 1MB, 1)
    # Kernel plus user time, 100 ns units.
    cpu_100ns = [double](($tree | ForEach-Object { [double]$_.KernelModeTime + [double]$_.UserModeTime } | Measure-Object -Sum).Sum)
  }
}
$csv = [System.IO.Path]::ChangeExtension($Report, '.csv')
'time,label,processes,working_set_mb,private_ws_mb,private_mb,cpu_pct' | Set-Content $csv
$script:prev = $null
function Sample([string]$Label) {
  $u = Get-Usage
  $now = Get-Date
  $cpu = ''
  if ($script:prev) {
    $wall = ($now - $script:prev.t).TotalSeconds
    if ($wall -gt 0) { $cpu = [math]::Round((($u.cpu_100ns - $script:prev.u.cpu_100ns) / 1e7) / ($wall * $cores) * 100, 2) }
  }
  $script:prev = @{ t = $now; u = $u }
  "{0},{1},{2},{3},{4},{5},{6}" -f $now.ToString('HH:mm:ss'), $Label, $u.processes, $u.ws_mb, $u.priv_ws_mb, $u.priv_mb, $cpu | Add-Content $csv
  $u | Add-Member -NotePropertyName cpu_pct -NotePropertyValue $cpu -PassThru
}
function Measure-For([string]$Label, [int]$Seconds) {
  $null = Sample $Label
  $rows = @()
  # Every sample's private working set in time order, $null kept in place (DEC14 coverage).
  $pws = New-Object System.Collections.Generic.List[object]
  $end = (Get-Date).AddSeconds($Seconds)
  while ((Get-Date) -lt $end) {
    Start-Sleep -Seconds $SampleSeconds
    $u = Sample $Label
    $rows += $u
    $pws.Add($u.priv_ws_mb)
  }
  $rows = @($rows | Where-Object { $_.cpu_pct -ne '' })
  $pv = [double[]]@($pws | Where-Object { $null -ne $_ })
  [pscustomobject]@{
    ws_mb   = ($rows | Measure-Object ws_mb -Maximum).Maximum
    priv_ws = $pws.ToArray()
    priv_ws_max = $(if ($pv.Count) { ($pv | Measure-Object -Maximum).Maximum } else { $null })
    cpu_pct = [math]::Round(($rows | Measure-Object cpu_pct -Average).Average, 2)
  }
}

Add-Line "HealthierYou camera checks, phase $Phase, $(Get-Date -Format 'yyyy-MM-dd HH:mm')"
Add-Line "Windows $([Environment]::OSVersion.Version), $cores logical processors"
if ($Phase -eq 'DeskReport') {
  $dir = if ($SessionsDir) { $SessionsDir } else { Split-Path -Parent ([System.IO.Path]::GetFullPath($Report)) }
  $files = @(Get-ChildItem -Path $dir -Filter 'hy-desk-*-*.json' | Where-Object { $_.Name -match '^hy-desk-\d-\d\.json$' } | Sort-Object Name | ForEach-Object { $_.FullName })
  if ($files.Count -eq 0) { throw "No hy-desk-<setup>-<session>.json files in that folder. Run -Phase Desk first." }
  $json = [System.IO.Path]::ChangeExtension($Report, '.json')
  $txt = [System.IO.Path]::ChangeExtension($Report, '.gate.txt')
  $argsList = @('--hy-posture-report', 'gate', '--report', "`"$json`"", '--text', "`"$txt`"") + @($files | ForEach-Object { "`"$_`"" })
  $p = Start-Process -FilePath $Exe -ArgumentList $argsList -PassThru -Wait -NoNewWindow
  if ($p.ExitCode -ne 0) { throw "hy-companion.exe --hy-posture-report gate failed (exit $($p.ExitCode)); see $json" }
  Add-Line "Sessions added up: $($files.Count)"
  Get-Content $txt | ForEach-Object { Add-Line $_ }
  $lines | Set-Content $Report
  Write-Host "Report: $Report (and $json). Numbers only; read before sharing." -ForegroundColor Green
  return
}
if (-not (Get-Process hy-companion -ErrorAction SilentlyContinue)) {
  throw 'HealthierYou is not running. Start it, then run this script again.'
}

if ($Phase -eq 'Session') {
  Do-Step 'Make sure the posture check is OFF (pet menu shows "Posture check"). Sit at the desk as usual.'
  $off = Measure-For 'camera-off' 60
  Add-Line "Camera off: app's own memory (private working set) peak $($off.priv_ws_max) MB; whole-tree working set peak $($off.ws_mb) MB (information); CPU $($off.cpu_pct)% of the PC"

  Do-Step 'Pet menu > Posture check. Read the consent text, then choose "Turn on camera".'
  Ask 'consent-text' 'Did the consent text say: not medical, frames never saved or sent, CAM badge while on, everything else works without it?'
  Ask 'windows-camera-light' 'Did the webcam light come on (if it has one) and the pet show "CAM" with a dot at its top left?'
  $on = Measure-For 'camera-on-desk' 180
  Add-Line "Camera on (3 min at the desk): app's own memory (private working set) peak $($on.priv_ws_max) MB; whole-tree working set peak $($on.ws_mb) MB (information); CPU $($on.cpu_pct)% of the PC"
  $addCpu = [math]::Round($on.cpu_pct - $off.cpu_pct, 2)
  # DEC14: the 350 MB budget is the app's own memory with the camera on (not the added
  # whole-tree working set, which is shown in the note as information only).
  $mem = Get-MemoryBudgetResult $on.priv_ws $on.ws_mb $off.ws_mb
  Result 'budget-memory' $mem.Outcome $mem.Note
  Result 'budget-cpu' $(if ($addCpu -le 10) { 'PASS' } else { 'FAIL' }) "camera adds $addCpu% of the PC (budget 10%)"
  Ask 'reading-at-desk' 'Settings > Posture check: does the status say "Camera on" while you sit normally in view?'

  Do-Step 'Leave the desk (out of view) for about 30 seconds, then come back.'
  Ask 'no-reading-away' 'While you were away, did the pet tag say "Cam: no reading", and go back to normal when you returned?'
  Do-Step 'Cover the webcam with your hand or its shutter for 20 seconds, then uncover it.'
  Ask 'no-reading-covered' 'While covered, did the status say "Cam: no reading"? Did it read again after?'

  Do-Step 'Open the Windows Camera app (or Teams video preview) so it takes the webcam, wait 20 seconds, then close it.'
  Ask 'busy' 'Did the posture check say "Camera in use by another app" (the pet still working), and come back within about a minute after you closed the other app?'

  Do-Step 'Start a Teams/Zoom call or a PowerPoint slide show for 30 seconds, then end it.'
  Ask 'meeting-hold' 'During it, did the CAM badge go away (camera released), and come back after it ended?'

  Do-Step 'Lock the PC (Windows+L) for 30 seconds, then unlock.'
  Ask 'lock-hold' 'Was the webcam light off while locked, and the CAM badge back after unlocking?'

  Do-Step 'Windows Settings > Privacy & security > Camera: turn OFF "Let desktop apps access your camera". Then pet menu > Posture check > Try again.'
  Ask 'denied' 'Did it say Windows blocked the camera, with the pet and reminders still working?'
  Do-Step 'Turn desktop camera access back ON, then choose Try again.'
  Ask 'denied-recovers' 'Did it read again ("Camera on")?'

  Do-Step 'If you have a second (USB) webcam: unplug the one in use for 20 seconds, then plug it back. Otherwise press Enter.'
  Ask 'unplug' 'Did it say no camera / camera in use, then recover by itself within about a minute after plugging back?'

  Do-Step 'Pet menu > Turn camera off. Watch the webcam light and the pet.'
  Ask 'off-2s' 'Did the light go off and the CAM badge disappear within about 2 seconds, with "Camera off. Capture stopped." beside the pet?'
  Start-Sleep -Seconds 10
  $after = Measure-For 'camera-off-after' 30
  # Memory returned after off: the app's own memory (DEC14) when both phases have readings,
  # else the whole-tree working set (named as such).
  if ($null -ne $after.priv_ws_max -and $null -ne $off.priv_ws_max) {
    Result 'memory-returned' $(if ($after.priv_ws_max -le $off.priv_ws_max + 60) { 'PASS' } else { 'FAIL' }) "app's own memory (private working set) $($after.priv_ws_max) MB after off (was $($off.priv_ws_max) MB; within 60 MB)"
  } else {
    Result 'memory-returned' $(if ($after.ws_mb -le $off.ws_mb + 60) { 'PASS' } else { 'FAIL' }) "whole-tree working set $($after.ws_mb) MB after off (was $($off.ws_mb) MB; no complete private working set reading)"
  }
} elseif ($Phase -eq 'Calibration') {
  Do-Step 'Turn the posture check ON. Open the camera page (pet menu > Posture check). Sit as you normally do and choose Calibrate; keep still until it finishes.'
  Ask 'calibration-usable' 'Did it say "Calibrated" within about 30 seconds?'
  Do-Step 'Optional: lean far back or move partly out of view, then choose Recalibrate.'
  Ask 'calibration-reason' 'If it refused, did it say "Calibration not usable" with a reason that matched what you did? (s if skipped)'
  Do-Step 'If it refused above, calibrate again sitting normally. Then work calmly at the desk for 5 minutes (reading, typing, glancing at a second screen).'
  Ask 'no-false-cue' 'In those 5 minutes, did the pet give NO posture cue?'
  Do-Step 'Now slump or lean towards the screen clearly and stay like that for 90 seconds.'
  Ask 'cue-after-a-minute' 'Did one "Posture check" card appear after about one to one and a half minutes, and only one?'
  Ask 'cue-wording' 'Was the card neutral (no judgement, no health claim), with OK and Recalibrate?'
  Do-Step 'Sit normally again. Then push the laptop back (or tilt the external camera) and leave it there for 3 minutes, sitting normally.'
  Ask 'moved-camera' 'Did the camera page say cues are paused within about 20 seconds, and after about 2 minutes did a notice ask you once to recalibrate, with no posture cue meanwhile?'
  Do-Step 'Put the camera back, choose Calibrate on the camera page, then quit HealthierYou from the tray and start it again at the same desk. Wait 1 minute.'
  Ask 'restart-accepted' 'Does the camera page show "Calibrated" again (not "Recalibrate")?'
} elseif ($Phase -eq 'Desk') {
  # Files beside the report: hy-desk-<setup>-<session>.labels.csv and .json (numbers only).
  $dir = Split-Path -Parent ([System.IO.Path]::GetFullPath($Report))
  $base = Join-Path $dir ("hy-desk-{0}-{1}" -f $Setup, $Session)
  $csvLabels = "$base.labels.csv"
  $json = "$base.json"
  $txt = "$base.session.txt"
  Add-Line "Desk session: setup $Setup, session $Session"
  $setupNote = Read-Host 'Describe the setup in a few words (camera, light, glasses; no names)'
  Result 'setup' 'INFO' $setupNote
  Do-Step 'Turn the posture check ON and open the camera page (pet menu > Posture check). Sit at the desk as usual. Keep this window where you can see it; it beeps at the end of each step.'
  $since = Get-NowMs
  'start_s,end_s,label' | Set-Content $csvLabels
  $steps = @(Get-DeskSteps $Session)
  $i = 0
  foreach ($st in $steps) {
    $i++
    $label = $st[0]; $secs = [int]$st[1]; $what = $st[2]
    Write-Host ''
    Write-Host ("Step {0} of {1} ({2}{3}): {4}" -f $i, $steps.Count, $label, $(if ($secs -gt 0) { ", $secs s" } else { '' }), $what) -ForegroundColor Yellow
    $null = Read-Host '  Press Enter as you start'
    $start = Get-NowMs
    if ($secs -gt 0) {
      $end = $start + 1000 * $secs
      while ((Get-NowMs) -lt $end) {
        $left = [math]::Ceiling(($end - (Get-NowMs)) / 1000)
        Write-Progress -Activity "Step $i of $($steps.Count): $label" -Status "$left s left" -PercentComplete ([math]::Min(100, 100 * ($secs - $left) / $secs))
        Start-Sleep -Milliseconds 500
      }
      Write-Progress -Activity "Step $i" -Completed
      [Console]::Beep(880, 300)
    } else {
      $null = Read-Host '  Press Enter when you are back and unlocked'
      $end = Get-NowMs
    }
    Format-Label $since $start $end $label | Add-Content $csvLabels
  }
  Do-Step 'Session done. Turn the camera OFF now (pet menu > Turn camera off), so HealthierYou writes its open records.'
  Start-Sleep -Seconds 3
  $until = Get-NowMs
  $argsList = @('--hy-posture-report', 'session', '--since', [string]$since, '--until', [string]$until, '--labels', "`"$csvLabels`"", '--setup', [string]$Setup, '--session', [string]$Session, '--report', "`"$json`"", '--text', "`"$txt`"")
  $p = Start-Process -FilePath $Exe -ArgumentList $argsList -PassThru -Wait -NoNewWindow
  if ($p.ExitCode -ne 0) {
    Result 'session-report' 'FAIL' "hy-companion.exe --hy-posture-report session exited $($p.ExitCode) (a build from before V2-3?)"
  } else {
    Get-Content $txt | ForEach-Object { Add-Line $_ }
    Result 'session-report' 'PASS' "raw counts in $(Split-Path -Leaf $json)"
  }
  Ask 'desk-cue-wording' 'Whenever a "Posture check" card appeared, was it neutral (no judgement, no health claim)?'
  Ask 'desk-no-cue-unseen' 'While you were away, covered, or the camera was moved, did NO posture card appear?'
} else {
  Write-Host 'For the V2-5 soak use Test-CameraSoak.ps1 (it adds the app''s camera log, holds, releases and verdicts); this phase is the V2-1 quick look.' -ForegroundColor Yellow
  Do-Step 'Turn the posture check ON (pet menu > Posture check). Then work normally; keep this window open.'
  $res = Measure-For 'soak' ($Minutes * 60)
  Add-Line "Soak ($Minutes min): app's own memory (private working set) peak $($res.priv_ws_max) MB; whole-tree working set peak $($res.ws_mb) MB (information); mean CPU $($res.cpu_pct)% of the PC"
  $rows = Import-Csv $csv | Where-Object { $_.label -eq 'soak' }
  $first = [double]($rows | Select-Object -First 12 | Measure-Object working_set_mb -Average).Average
  $last = [double]($rows | Select-Object -Last 12 | Measure-Object working_set_mb -Average).Average
  $growth = [math]::Round($last - $first, 1)
  Result 'soak-memory-growth' $(if ($growth -le 50) { 'PASS' } else { 'FAIL' }) "whole-tree working set grew $growth MB from the first to the last minute (quick look; the V2-5 budget and trends are Test-CameraSoak.ps1's)"
  Ask 'soak-stops' 'Settings > Posture check: is it still on (not stopped for "not responding")?'
  Ask 'soak-holds' 'During the soak, did meetings, lock and lunch pause it and did it come back by itself?'
  Ask 'soak-feel' 'Did the PC feel as responsive as without the camera?'
  Ask 'soak-off' 'Turn camera off now: did the light go off within about 2 seconds?'
}

$lines | Set-Content $Report
Write-Host ''
Write-Host "Report: $Report; measurements: $csv. Read both before sharing." -ForegroundColor Green
