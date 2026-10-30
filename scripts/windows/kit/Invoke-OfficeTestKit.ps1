<#
.SYNOPSIS
  The HealthierYou office test kit: runs every owner test of one office PC in order, step
  by step, and packs the reports into one zip on the Desktop to upload.

.DESCRIPTION
  Started by Run-PC1-Server.cmd (-Role Server) or Run-PC2-Camera.cmd (-Role Camera). It
  calls the repository's own test scripts (Test-Soak.ps1, Test-NetworkCapture.ps1,
  Test-CameraSoak.ps1, Test-HealthierYou-Camera.ps1, Test-HealthierYou-OfficePC.ps1) with
  the parameters docs/V1_Acceptance.md gives, and takes each step's verdict from that
  script's own report: a step is PASS only when the script's evidence says so. Where no
  script gives a verdict (a guided step with skipped questions, a session that only
  records data) the step is INCONCLUSIVE, never PASS.

  PC1 (server): check the PC and the kit; install the office server as a Windows service;
  set the admin passphrase and show the server's six words; install the companion; pair
  PC2 (and this PC) with the server, with evidence from the server log; the 8-hour server
  soak (S2) and companion soak (S1) side by side.

  PC2 (camera): check the PC (Windows version, memory, webcams) and pick the acceptance
  rows; the guided companion install (I1 to I4, I6); the network capture with the pairing
  inside it (S3, and S4 on Windows 10); start at sign-in (I5); the 2-hour camera soak
  (CS1 on Windows 11, CS2 on Windows 10, CS3 with a USB webcam; CS4 is its first 15
  minutes); a second soak with a USB webcam when one is found (CS3); the desk sessions
  (V2-3 gate), as many as practical; an overnight companion soak (S1) on a Windows 10 or
  small-memory PC; the uninstall check (I7, I9b).

  Progress is kept in a state file (%LOCALAPPDATA%\HealthierYou-TestKit\<role>\state.json):
  after a reboot, sleep, sign-out or a closed window, starting the kit again carries on at
  the step that was not finished. While it runs, the kit starts itself again at the next
  sign-in (HKCU RunOnce), and removes that when it is done.

  Administrator steps (server install and soak, the network capture) run in a second
  window started once with one UAC prompt at the start. That window runs a protected copy
  of this script (under %ProgramData%, Administrators only), made only when its bytes match
  the ones this window started with; the user's own copy is never run elevated. It only
  runs a fixed list of steps with checked parameters. Every other step runs as the
  signed-in user.

  The zip (HealthierYou-results-<PC1|PC2>-<date>.zip on the Desktop) holds only .txt,
  .json and .csv reports. Each file is scanned first: user, PC, domain and server names,
  the pairing label, folder paths, IP and MAC addresses, e-mail addresses and GUIDs are
  replaced by placeholders; a file that still holds one after that is left out, and so is
  any image or binary file. Raw captures (.etl, .pcapng) are never put in it.

  Pairing retest (-Retest Pairing; Run-PC1-Pairing-Retest.cmd, Run-PC2-Pairing-Retest.cmd):
  redoes only the pairing evidence with this kit's own builds, after a first run whose
  pairing could not work. It keeps its own state, results and zip
  (%LOCALAPPDATA%\HealthierYou-TestKit\<role>-retest-pairing, and
  HealthierYou-results-<PC1|PC2>-pairing-retest-<date>.zip), so the first run's are left as
  they are. PC1: check the PC and the kit; replace the office server with the kit's build
  when the installed hy-server.exe is not byte for byte the one the kit's installer holds
  (an installed server refuses the same version over itself, by design, so it is
  uninstalled keeping its data, then installed again: keys, database and admin passphrase
  stay; the passphrase is asked for only if they did not); the same for HealthierYou on
  this PC (per user, keeping its data); then the pairing. PC2: check the PC; install
  HealthierYou, or replace it when it is not the kit's build; then the network capture with
  the pairing. The administrator steps run in their own window and protected folder
  (HealthierYou-TestKit-Admin\<role>-Retest), never in a first run's window still open.

  -DryRun runs the whole sequence of a role with every test script and administrator step
  replaced by a stub and every question answered with its default, in a temporary folder.
  -SelfTest checks the step sequencing, resume after an interruption, the administrator
  job files, the verdict rules and the zip and redaction, with stubbed steps (CI runs it
  under PowerShell 7 and Windows PowerShell 5.1).

.EXAMPLE
  .\Invoke-OfficeTestKit.ps1 -Role Server
.EXAMPLE
  .\Invoke-OfficeTestKit.ps1 -Role Camera -DryRun
.EXAMPLE
  .\Invoke-OfficeTestKit.ps1 -Role Server -Retest Pairing
.EXAMPLE
  .\Invoke-OfficeTestKit.ps1 -SelfTest
#>
[CmdletBinding()]
param(
  [ValidateSet('Server', 'Camera')][string]$Role,
  # Redo only one part with this kit's builds, in its own state (Pairing: the pairing).
  [ValidateSet('Pairing')][string]$Retest,
  [switch]$SelfTest,
  [switch]$DryRun,
  # Internal: the administrator window.
  [switch]$Agent,
  [string]$StateDir,
  [string]$OutDir,
  [string]$InstallersDir,
  [string]$ScriptsDir,
  # Run this step again (its id, for example P2-05), then carry on.
  [string]$Redo,
  # Internal: the kit window's process id, given to the administrator window.
  [int]$OwnerPid = 0,
  # Internal (administrator window, from the bootstrap only): the SHA-256 of each file it may
  # stage, pinned by the kit window before the UAC prompt (name=HEX;name=HEX).
  [string]$StagePins,
  # Internal (self-test of the bootstrap): the protected folder, and process the requests
  # already waiting once, then exit.
  [string]$AdminRoot,
  [switch]$AgentOnce
)
$ErrorActionPreference = 'Stop'
$KitVersion = '1.0'
$script:IsWin = ($PSVersionTable.PSVersion.Major -le 5) -or [bool]$IsWindows

# ====================================================================== small helpers

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

function Get-NowText { (Get-Date).ToString('yyyy-MM-dd HH:mm:ss') }

function Write-Log([string]$Text, [string]$Color = 'Gray') {
  $line = "[$(Get-NowText)] $Text"
  Write-Host $line -ForegroundColor $Color
  if ($script:LogFile) { try { Add-Content -LiteralPath $script:LogFile -Value $line -Encoding UTF8 } catch { } }
}

function Show-Banner([string]$Title, [string[]]$Lines = @(), [string]$Color = 'Yellow') {
  $w = 78
  $bar = '#' * $w
  Write-Host ''
  Write-Host $bar -ForegroundColor $Color
  Write-Host ('##  ' + $Title.ToUpperInvariant()) -ForegroundColor $Color
  Write-Host $bar -ForegroundColor $Color
  foreach ($l in $Lines) { Write-Host ('   ' + $l) -ForegroundColor $Color }
  if ($Lines.Count) { Write-Host $bar -ForegroundColor $Color }
  Write-Host ''
  if ($script:LogFile) { try { Add-Content -LiteralPath $script:LogFile -Value "[$(Get-NowText)] == $Title" -Encoding UTF8 } catch { } }
}

# Questions. In -DryRun and -SelfTest the answers come from $script:Answers (a queue of
# strings) or, when it is empty, the default.
$script:Answers = New-Object System.Collections.Queue
$script:Unattended = $false
function Read-Answer([string]$Prompt, [string]$Default = '') {
  if ($script:Unattended) {
    $a = if ($script:Answers.Count) { [string]$script:Answers.Dequeue() } else { $Default }
    Write-Host "$Prompt [auto: $a]"
    return $a
  }
  $a = Read-Host $Prompt
  if ([string]::IsNullOrWhiteSpace($a)) { return $Default }
  return $a.Trim()
}
function Read-YesNo([string]$Question, [bool]$Default = $true) {
  $d = if ($Default) { 'y' } else { 'n' }
  $a = Read-Answer "$Question (y = yes, n = no; Enter = $d)" $d
  return ($a -match '^[yY]')
}
function Wait-Enter([string]$Text = 'Press Enter when done') { $null = Read-Answer "  $Text" '' }

function Test-Elevated {
  if (-not $script:IsWin) { return $false }
  $id = [Security.Principal.WindowsIdentity]::GetCurrent()
  return ([Security.Principal.WindowsPrincipal]$id).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function ConvertTo-HashtableDeep($o) {
  if ($null -eq $o) { return $null }
  if ($o -is [System.Collections.IDictionary]) {
    $h = @{}; foreach ($k in @($o.Keys)) { $h[[string]$k] = ConvertTo-HashtableDeep $o[$k] }; return $h
  }
  if ($o -is [System.Management.Automation.PSCustomObject]) {
    $h = @{}; foreach ($p in $o.PSObject.Properties) { $h[$p.Name] = ConvertTo-HashtableDeep $p.Value }; return $h
  }
  if (($o -is [System.Collections.IEnumerable]) -and -not ($o -is [string])) {
    $list = New-Object System.Collections.ArrayList
    foreach ($i in $o) { [void]$list.Add((ConvertTo-HashtableDeep $i)) }
    return , $list.ToArray()
  }
  return $o
}

function Write-JsonFile([string]$Path, $Object) {
  $dir = Split-Path -Parent $Path
  if ($dir -and -not (Test-Path -LiteralPath $dir)) { New-Item -ItemType Directory -Force -Path $dir | Out-Null }
  $tmp = "$Path.tmp"
  ($Object | ConvertTo-Json -Depth 12) | Set-Content -LiteralPath $tmp -Encoding UTF8
  Move-Item -LiteralPath $tmp -Destination $Path -Force
}
function Read-JsonFile([string]$Path) {
  if (-not (Test-Path -LiteralPath $Path)) { return $null }
  try { return ConvertTo-HashtableDeep (Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json) } catch { return $null }
}

# Quote arguments for a Windows command line (Start-Process joins them as given).
function ConvertTo-ArgString([string[]]$Arguments) {
  $out = foreach ($a in $Arguments) {
    if ($null -eq $a -or $a -eq '') { '""' }
    elseif ($a -match '[\s"]') { '"' + (($a -replace '(\\*)"', '$1$1\"') -replace '(\\+)$', '$1$1') + '"' }
    else { $a }
  }
  return ($out -join ' ')
}

function Get-HostExe {
  try { return (Get-Process -Id $PID).Path } catch { return 'powershell.exe' }
}

function New-Result([string]$Verdict, [string]$Detail = '', [string[]]$Rows = @(), $Sub = @()) {
  return @{ verdict = $Verdict; detail = $Detail; rows = @($Rows); sub = @($Sub) }
}

# ===================================================================== verdict rules
# Each one reuses the script's own verdict, and gives PASS only with its evidence.

function Get-ExitVerdict([int]$ExitCode, [string]$JsonPath, [string]$JsonField) {
  $j = Read-JsonFile $JsonPath
  if (-not $j) { return @{ verdict = 'INCONCLUSIVE'; detail = "no report was written (exit code $ExitCode)" } }
  $fromJson = [string]$j[$JsonField]
  $fromExit = switch ($ExitCode) { 0 { 'PASS' } 1 { 'FAIL' } 2 { 'INCONCLUSIVE' } default { 'INCONCLUSIVE' } }
  if ($fromJson -eq 'FAIL' -or $fromExit -eq 'FAIL') { return @{ verdict = 'FAIL'; detail = "the script's verdict: FAIL (exit code $ExitCode)" } }
  if ($fromJson -eq 'PASS' -and $fromExit -eq 'PASS') { return @{ verdict = 'PASS'; detail = "the script's verdict: PASS" } }
  return @{ verdict = 'INCONCLUSIVE'; detail = "the script's verdict: $(if ($fromJson) { $fromJson } else { 'none' }) (exit code $ExitCode)" }
}

function Get-SoakVerdict([string]$JsonPath, [int]$ExitCode) {
  $j = Read-JsonFile $JsonPath
  if (-not $j) { return @{ verdict = 'INCONCLUSIVE'; detail = "no soak report was written (exit code $ExitCode)" } }
  $failed = @($j.checks | Where-Object { $_ -and $_.Required -and -not $_.Pass } | ForEach-Object { $_.Name })
  if ($j.ok -eq $true -and $ExitCode -eq 0) { return @{ verdict = 'PASS'; detail = "Overall PASS ($(@($j.checks).Count) checks)" } }
  return @{ verdict = 'FAIL'; detail = "Overall FAIL: $($failed -join '; ')" }
}

# Guided reports ([PASS] / [FAIL] / [SKIP] lines): FAIL if any FAIL; PASS only when every
# question was answered and passed; skipped questions leave it INCONCLUSIVE.
function Get-GuidedVerdict([string]$ReportPath) {
  if (-not (Test-Path -LiteralPath $ReportPath)) { return @{ verdict = 'INCONCLUSIVE'; detail = 'no report was written' } }
  $lines = @(Get-Content -LiteralPath $ReportPath)
  $pass = @($lines | Where-Object { $_ -like '`[PASS`]*' }).Count
  $fail = @($lines | Where-Object { $_ -like '`[FAIL`]*' }).Count
  $skip = @($lines | Where-Object { $_ -like '`[SKIP`]*' }).Count
  $d = "$pass passed, $fail failed, $skip skipped"
  if ($fail -gt 0) { return @{ verdict = 'FAIL'; detail = $d } }
  if ($pass -gt 0 -and $skip -eq 0) { return @{ verdict = 'PASS'; detail = $d } }
  return @{ verdict = 'INCONCLUSIVE'; detail = $d }
}

# The desk sessions' PRD V2 gate (hy-companion.exe --hy-posture-report gate).
function Get-GateVerdict([string]$JsonPath) {
  $j = Read-JsonFile $JsonPath
  if (-not $j) { return @{ verdict = 'INCONCLUSIVE'; detail = 'no gate report was written' } }
  $setups = @($j.setups | Where-Object { $_ })
  $complete = @($setups | Where-Object { [int]$_.sessions -ge 2 })
  $failed = @($complete | Where-Object { -not $_.pass })
  $d = "$($complete.Count) of 5 setups complete (2 sessions each); $(@($complete | Where-Object { $_.pass }).Count) passed"
  if ($j.pass -eq $true) { return @{ verdict = 'PASS'; detail = "PRD V2 gate passed: $d" } }
  if ($failed.Count) { return @{ verdict = 'FAIL'; detail = "setup(s) $(@($failed | ForEach-Object { $_.setup }) -join ', ') failed the gate; $d" } }
  return @{ verdict = 'INCONCLUSIVE'; detail = "gate incomplete: $d" }
}

function Join-Verdicts([string[]]$Verdicts) {
  $v = @($Verdicts | Where-Object { $_ -and $_ -ne 'SKIPPED' })
  if (-not $v.Count) { return 'SKIPPED' }
  if ($v -contains 'FAIL') { return 'FAIL' }
  if (@($v | Where-Object { $_ -ne 'PASS' }).Count) { return 'INCONCLUSIVE' }
  return 'PASS'
}

# ===================================================================== this PC

function Get-WebcamCount {
  if ($script:Sim) { return [int]$script:Sim.webcams }
  $devs = @()
  try { $devs = @(Get-PnpDevice -PresentOnly -ErrorAction Stop | Where-Object { $_.Status -eq 'OK' -and ($_.Class -eq 'Camera' -or ($_.Class -eq 'Image' -and $_.FriendlyName -match 'cam|video')) }) }
  catch {
    try { $devs = @(Get-CimInstance Win32_PnPEntity | Where-Object { $_.Status -eq 'OK' -and ($_.PNPClass -eq 'Camera' -or ($_.PNPClass -eq 'Image' -and $_.Name -match 'cam|video')) }) } catch { $devs = @() }
  }
  return $devs.Count
}

function Get-EnvInfo {
  if ($script:Sim) { return $script:Sim.Clone() }
  $e = @{}
  $os = Get-CimInstance Win32_OperatingSystem
  $e.windows = [string]$os.Caption
  $e.build = [int]$os.BuildNumber
  $e.windows11 = ([int]$os.BuildNumber -ge 22000)
  $e.ram_gb = [Math]::Round((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory / 1GB, 1)
  $e.cores = [Environment]::ProcessorCount
  $laptop = $false
  try {
    $ch = @((Get-CimInstance Win32_SystemEnclosure).ChassisTypes)
    if (@($ch | Where-Object { @(8, 9, 10, 11, 12, 14, 18, 21, 30, 31, 32) -contains [int]$_ }).Count) { $laptop = $true }
    if (@(Get-CimInstance Win32_Battery -ErrorAction SilentlyContinue).Count) { $laptop = $true }
  } catch { }
  $e.laptop = $laptop
  $e.webcams = Get-WebcamCount
  $e.pktmon = [bool](Get-Command pktmon.exe -ErrorAction SilentlyContinue)
  $e.elevated = Test-Elevated
  $member = $null
  try {
    $sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
    $member = [bool](@(Get-LocalGroupMember -SID 'S-1-5-32-544' -ErrorAction Stop | Where-Object { $_.SID.Value -eq $sid }).Count)
  } catch { }
  $e.admin_member = $member
  $e.powershell = $PSVersionTable.PSVersion.ToString()
  return $e
}

# Which acceptance rows this PC runs (docs/V1_Acceptance.md sections 1, 3 and 9).
function Select-Rows([hashtable]$Env, [string]$RoleName) {
  $r = @{ notes = @() }
  if ($RoleName -eq 'Server') {
    $r.soak = @('S2', 'S1')
    $r.notes += 'S1 runs on the server PC beside S2; docs/V1_Acceptance.md prefers the 4 GB Windows 10 companion PC for S1 (PC2 offers it when it is that PC).'
    return $r
  }
  $w = [int]$Env.webcams
  $r.capture = @('S3')
  if (-not $Env.windows11) { $r.capture += 'S4' }
  if ($w -le 0) {
    $r.camera_kind = ''
    $r.camera_row = @()
    $r.notes += 'No webcam found: the camera soak and desk sessions cannot run (CS2: "if its webcam is not detected, record that and run CS3 there instead").'
  }
  elseif ($Env.laptop -or $w -ge 2) {
    $r.camera_kind = 'built-in'
    $r.camera_row = @($(if ($Env.windows11) { 'CS1' } else { 'CS2' }), 'CS4')
  }
  else {
    # A desktop PC with one webcam: it is a USB webcam.
    $r.camera_kind = 'usb'
    $r.camera_row = @('CS3', 'CS4')
    $r.notes += "A desktop PC with one webcam: it is taken as a USB webcam, so the soak counts as CS3 on Windows $(if ($Env.windows11) { '11' } else { '10' })."
  }
  $r.second_webcam = ($w -ge 2 -and $r.camera_kind -eq 'built-in')
  $r.companion_soak = ((-not $Env.windows11) -or [double]$Env.ram_gb -le 4.5)
  return $r
}

# ===================================================================== kit files

function Initialize-KitPaths {
  $root = $PSScriptRoot
  if (-not $script:ScriptsDirResolved) {
    $cands = @($ScriptsDir, (Join-Path $root 'scripts'), (Split-Path -Parent $root), $root) | Where-Object { $_ }
    $script:ScriptsDirResolved = @($cands | Where-Object { Test-Path -LiteralPath (Join-Path $_ 'Test-Soak.ps1') } | Select-Object -First 1)
    if ($script:ScriptsDirResolved.Count) { $script:ScriptsDirResolved = [string]$script:ScriptsDirResolved[0] } else { $script:ScriptsDirResolved = $root }
  }
  if (-not $script:InstallersDirResolved) {
    $script:InstallersDirResolved = if ($InstallersDir) { $InstallersDir } else { Join-Path $root 'installers' }
  }
  $m = Join-Path $root 'kit-manifest.json'
  $script:Manifest = Read-JsonFile $m
}

function Get-Installer([string]$Kind) {
  if ($Kind -eq 'companion-newer') {
    if (-not ($script:Manifest -and $script:Manifest.companion_newer)) { return $null }
    $p = Join-Path $script:InstallersDirResolved $script:Manifest.companion_newer
    if (Test-Path -LiteralPath $p) { return $p } else { return $null }
  }
  $pattern = switch ($Kind) { 'server' { 'HealthierYou-Server_*_x64-setup.exe' } 'companion' { 'HealthierYou_*_x64-setup.exe' } }
  $all = @(Get-ChildItem -LiteralPath $script:InstallersDirResolved -Filter $pattern -File -ErrorAction SilentlyContinue | Sort-Object Name)
  if ($Kind -eq 'companion') {
    $all = @($all | Where-Object { $_.Name -notlike 'HealthierYou-Server*' })
    if ($script:Manifest -and $script:Manifest.companion) { $all = @($all | Where-Object { $_.Name -eq $script:Manifest.companion }) }
  }
  if (-not $all.Count) { return $null }
  return $all[0].FullName
}

function Test-InstallerHash([string]$Path) {
  if (-not $Path -or -not (Test-Path -LiteralPath $Path)) { return @{ ok = $false; detail = 'installer missing' } }
  $name = Split-Path -Leaf $Path
  if (-not $script:Manifest -or -not $script:Manifest.sha256 -or -not $script:Manifest.sha256[$name]) { return @{ ok = $null; detail = "$name has no entry in kit-manifest.json" } }
  $h = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash
  $ok = ($h -eq ([string]$script:Manifest.sha256[$name]).ToUpperInvariant())
  return @{ ok = $ok; detail = "$name SHA-256 $(if ($ok) { 'matches' } else { 'does NOT match' }) kit-manifest.json" }
}

# ===================================================================== state file

function Read-State {
  $s = Read-JsonFile $script:StatePath
  if (-not $s) { $s = @{ kit = $KitVersion; role = $script:RoleName; created = (Get-NowText); steps = @{}; data = @{}; env = @{} } }
  foreach ($k in 'steps', 'data', 'env') { if (-not ($s[$k] -is [hashtable])) { $s[$k] = @{} } }
  return $s
}
function Save-State { Write-JsonFile $script:StatePath $script:State }

# ===================================================================== child scripts

# Run one of the repository's test scripts in this console (prompts and colours work).
# With -Staged (the administrator window), only the protected, hash-checked copy is run.
# Returns the exit code.
function Invoke-KitScript([string]$Name, [string[]]$Arguments, [switch]$Staged) {
  $path = if ($Staged) { Get-StagedFile $Name } else { Join-Path $script:ScriptsDirResolved $Name }
  $script:LastScriptPath = $path
  if ($script:ScriptStub) { return [int](& $script:ScriptStub $Name $Arguments $path) }
  if (-not (Test-Path -LiteralPath $path)) { throw "The kit is incomplete: $Name is missing" }
  $all = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $path) + $Arguments
  Write-Log "Running $Name $($Arguments -join ' ')" 'DarkGray'
  $sp = @{ FilePath = (Get-HostExe); ArgumentList = (ConvertTo-ArgString $all); NoNewWindow = $true; PassThru = $true }
  if ($Staged) { $sp.WorkingDirectory = (Get-AdminDir 'stage') }
  $p = Start-Process @sp
  $null = $p.Handle
  $p.WaitForExit()
  return [int]$p.ExitCode
}

# Run a script for its report(s): old copies are removed first, so only a report written
# by this run counts. Returns @{ code; fresh }.
function Invoke-ReportedScript([string]$Name, [string[]]$Arguments, [string[]]$Reports, [switch]$Staged) {
  foreach ($r in $Reports) { Remove-Item -LiteralPath $r -Force -ErrorAction SilentlyContinue }
  $since = (Get-Date).AddSeconds(-2)
  $code = Invoke-KitScript $Name $Arguments -Staged:$Staged
  $stale = @($Reports | Where-Object { -not ((Test-Path -LiteralPath $_) -and (Get-Item -LiteralPath $_).LastWriteTime -ge $since) })
  return @{ code = $code; fresh = ($stale.Count -eq 0); missing = @($stale | ForEach-Object { Split-Path -Leaf $_ }) }
}

# A verdict needs this run's own report, and a script that exited with an error is never PASS.
function Limit-Verdict($Verdict, $Run) {
  if (-not $Run.fresh) { return @{ verdict = 'INCONCLUSIVE'; detail = "this run wrote no report ($(@($Run.missing) -join ', ') missing; exit code $($Run.code))" } }
  if ([int]$Run.code -ne 0 -and $Verdict.verdict -eq 'PASS') { return @{ verdict = 'INCONCLUSIVE'; detail = "$($Verdict.detail); but the script exited with code $($Run.code)" } }
  return $Verdict
}

function Start-KeepAwake {
  if (-not $script:IsWin -or $script:Unattended) { return }
  try {
    if (-not ('HyKit.Power' -as [type])) { Add-Type -Namespace HyKit -Name Power -MemberDefinition '[DllImport("kernel32.dll")] public static extern uint SetThreadExecutionState(uint esFlags);' }
    $null = [HyKit.Power]::SetThreadExecutionState([uint32]2147483649)  # ES_CONTINUOUS | ES_SYSTEM_REQUIRED
  } catch { }
}
function Stop-KeepAwake {
  if (-not $script:IsWin -or $script:Unattended) { return }
  try { $null = [HyKit.Power]::SetThreadExecutionState([uint32]2147483648) } catch { }
}

function Get-CompanionExe { if ($env:LOCALAPPDATA) { Join-Path $env:LOCALAPPDATA 'HealthierYou\hy-companion.exe' } else { '' } }
function Test-CompanionRunning {
  if ($script:Sim) { return $true }
  return [bool](Get-Process -Name 'hy-companion' -ErrorAction SilentlyContinue)
}
function Start-CompanionIfNeeded {
  if ($script:Sim) { return $true }
  if (Test-CompanionRunning) { return $true }
  $exe = Get-CompanionExe
  if (-not (Test-Path -LiteralPath $exe)) { return $false }
  Start-Process -FilePath $exe | Out-Null
  for ($i = 0; $i -lt 30 -and -not (Test-CompanionRunning); $i++) { Start-Sleep -Seconds 1 }
  Start-Sleep -Seconds 10
  return (Test-CompanionRunning)
}

# A retest has its own entry, so a first run's entry (and its resume) is left as it is.
function Get-AutoResumeName { if ($script:RetestName) { return "HealthierYouTestKit$($script:RetestName)Retest" } return 'HealthierYouTestKit' }
function Set-AutoResume([bool]$On) {
  if ($script:Unattended -or -not $script:IsWin) { return }
  $key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\RunOnce'
  try {
    if ($On) {
      $launcher = $env:HYKIT_LAUNCHER
      $cmd = if ($launcher -and (Test-Path -LiteralPath $launcher)) { "`"$launcher`"" } else { "`"$(Get-HostExe)`" -NoProfile -ExecutionPolicy Bypass -File `"$PSCommandPath`" -Role $script:RoleName$(if ($script:RetestName) { " -Retest $($script:RetestName)" })" }
      if (-not (Test-Path $key)) { New-Item -Path $key -Force | Out-Null }
      Set-ItemProperty -Path $key -Name (Get-AutoResumeName) -Value $cmd
    }
    else { Remove-ItemProperty -Path $key -Name (Get-AutoResumeName) -ErrorAction SilentlyContinue }
  } catch { }
}

# ===================================================================== administrator window
# The agent is this script run elevated (-Agent), started once with one UAC prompt.
#
# Trust boundary, part one: the kit window hashes this script's bytes when it starts. The
# elevated window is started with -EncodedCommand holding a small bootstrap (never -File on
# the user's writable kit folder): it reads the user's copy once, refuses unless those bytes
# have that hash, writes them into <admin>\boot (Administrators and SYSTEM only) and runs
# that copy (New-AgentBootstrap below). A copy replaced while the UAC prompt waits never runs.
#
# Part two: the moment the person approves the UAC prompt. Right then the agent
# copies everything it will ever execute (the test script its role needs and, on PC1, the
# server installer) into a folder under %ProgramData% that only Administrators and SYSTEM
# can write, requires each copy's SHA-256 to equal the one the kit window pinned before the
# UAC prompt (and checked against kit-manifest.json; no pin or a different file: refused),
# and from then on runs only those copies, after checking the hash
# again. Nothing in the user's (writable) kit folder or profile is executed elevated later.
# The user's kit window talks to it through <admin>\queue (the only folder the user may
# write: requests are data, an allowlisted operation name and parameters that are checked);
# results and reports come back through <admin>\out (users may read, not write); raw
# captures go to <admin>\raw (Administrators and SYSTEM only). Report paths are chosen by
# the agent, never taken from a request. Child PowerShell processes get a module path
# without the user's own module folders.

$AdminOps = @('server-install', 'server-replace', 'server-passphrase', 'server-pairing-evidence', 'soak-server', 'network-capture', 'echo')

# The steps the administrator window takes in this mode: a first run never replaces the
# server; a pairing retest runs only what the pairing needs.
function Get-AllowedAdminOps {
  if ($script:RetestName -eq 'Pairing') {
    if ($script:RoleName -eq 'Server') { return @('server-replace', 'server-passphrase', 'server-pairing-evidence', 'echo') }
    return @('network-capture', 'echo')
  }
  return @($AdminOps | Where-Object { $_ -ne 'server-replace' })
}

# The protected folder's name: a retest never shares a first run's folder (or its window).
function Get-AdminSubName { if ($script:RetestName) { return "$($script:RoleName)-Retest" } return $script:RoleName }

function Get-AdminRoot {
  if ($script:AdminRootOverride) { return $script:AdminRootOverride }
  # The known folder, not %ProgramData% (a user may set that variable for their own account).
  return (Join-Path (Join-Path ([Environment]::GetFolderPath('CommonApplicationData')) 'HealthierYou-TestKit-Admin') (Get-AdminSubName))
}
function Get-AdminDir([string]$Part) { Join-Path (Get-AdminRoot) $Part }
function Get-AdminResultsDir { Join-Path (Get-AdminDir 'out') 'results' }

function Test-AclApplies { return ($script:IsWin -and ((-not $script:Sim) -or $script:TestAcl)) }

# Owner Administrators; only SYSTEM and Administrators may write. Mode: read (users may
# read), queue (users may also create and delete files), private (no users at all).
function Set-AdminAcl([string]$Path, [string]$Mode) {
  if (-not (Test-AclApplies)) { return }
  $grant = @('*S-1-5-18:(OI)(CI)F', '*S-1-5-32-544:(OI)(CI)F')
  if ($Mode -eq 'read') { $grant += '*S-1-5-32-545:(OI)(CI)RX' }
  if ($Mode -eq 'queue') { $grant += '*S-1-5-32-545:(OI)(CI)M' }
  & icacls.exe $Path /setowner '*S-1-5-32-544' /Q | Out-Null
  if ($LASTEXITCODE -ne 0) { throw "could not make Administrators the owner of the kit's administrator folder (icacls exit $LASTEXITCODE)" }
  $a = @($Path, '/inheritance:r', '/grant:r') + $grant + @('/Q')
  & icacls.exe @a | Out-Null
  if ($LASTEXITCODE -ne 0) { throw "could not protect the kit's administrator folder (icacls exit $LASTEXITCODE)" }
}

# A folder an ordinary user may have made first (or a link) is moved aside, never used.
function Initialize-SafeDir([string]$Path, [string]$Mode, [switch]$Fresh) {
  if (Test-Path -LiteralPath $Path) {
    $item = Get-Item -LiteralPath $Path -Force
    $untrusted = $false
    if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { $untrusted = $true }
    elseif (Test-AclApplies) {
      $owner = (Get-Acl -LiteralPath $Path).GetOwner([Security.Principal.SecurityIdentifier]).Value
      if (@('S-1-5-32-544', 'S-1-5-18') -notcontains $owner) { $untrusted = $true }
    }
    if ($untrusted) {
      $aside = "$Path.untrusted-" + [guid]::NewGuid().ToString('N').Substring(0, 8)
      Move-Item -LiteralPath $Path -Destination $aside -Force
      Write-Log 'A folder made before the administrator window existed was moved aside (not used).' 'Yellow'
    }
    elseif ($Fresh) { Remove-Item -LiteralPath $Path -Recurse -Force }
  }
  if (-not (Test-Path -LiteralPath $Path)) { New-Item -ItemType Directory -Force -Path $Path | Out-Null }
  Set-AdminAcl $Path $Mode
}

function Get-StageList {
  $list = @()
  if ($script:RoleName -eq 'Server') {
    # A pairing retest runs no soak: only the server installer is staged.
    if (-not $script:RetestName) { $list += @{ name = 'Test-Soak.ps1'; src = (Join-Path $script:ScriptsDirResolved 'Test-Soak.ps1') } }
    $inst = Get-Installer 'server'
    if ($inst) { $list += @{ name = (Split-Path -Leaf $inst); src = $inst; installer = $true } }
  }
  else { $list += @{ name = 'Test-NetworkCapture.ps1'; src = (Join-Path $script:ScriptsDirResolved 'Test-NetworkCapture.ps1') } }
  return $list
}

# The elevated side only: make the protected folders and stage the files it will run.
function Initialize-AdminArea {
  $script:AdminReady = $false
  $script:AdminSetupError = $null
  $script:StageHashes = @{}
  $script:StagedInstaller = $null
  try {
    if (-not $script:AdminRootOverride) { Initialize-SafeDir (Split-Path -Parent (Get-AdminRoot)) 'read' }
    Initialize-SafeDir (Get-AdminRoot) 'read'
    Initialize-SafeDir (Get-AdminDir 'queue') 'queue'
    Initialize-SafeDir (Get-AdminDir 'out') 'read'
    Initialize-SafeDir (Get-AdminResultsDir) 'read'
    Initialize-SafeDir (Get-AdminDir 'raw') 'private'
    Initialize-SafeDir (Get-AdminDir 'stage') 'read' -Fresh
    # Every file is checked against the SHA-256 the kit window pinned before the UAC prompt
    # (checked there against kit-manifest.json). No pin, or a different file: refused.
    $pins = $script:PinnedDigests
    if (-not ($pins -is [System.Collections.IDictionary]) -or $pins.Count -eq 0) { throw 'no SHA-256 was pinned by the kit window for the files to stage; refused' }
    $list = @(Get-StageList)
    foreach ($n in @($pins.Keys)) { if (@($list | Where-Object { $_.name -eq $n }).Count -eq 0) { throw "$n was checked when the kit window started but is not in the kit now; refused" } }
    foreach ($f in $list) {
      if (-not $pins.Contains($f.name)) { throw "$($f.name) has no SHA-256 pinned by the kit window (it was not in the kit when the kit window checked it); refused" }
      if (-not (Test-Path -LiteralPath $f.src)) { throw "the kit is incomplete: $($f.name) is missing" }
      $dest = Join-Path (Get-AdminDir 'stage') $f.name
      Copy-Item -LiteralPath $f.src -Destination $dest -Force
      $h = Get-FileSha256 $dest
      if ($h -cne [string]$pins[$f.name]) { Remove-Item -LiteralPath $dest -Force -ErrorAction SilentlyContinue; throw "$($f.name) changed after the kit window checked it against kit-manifest.json; refused" }
      $script:StageHashes[$f.name] = $h
      if ($f.installer) { $script:StagedInstaller = $f.name }
    }
    $script:AdminReady = $true
  }
  catch {
    $script:StageHashes = @{}; $script:StagedInstaller = $null
    $script:AdminSetupError = $_.Exception.Message; Write-Log "The administrator folder could not be prepared: $($script:AdminSetupError)" 'Red'
  }
}

# A staged file, only if it is still exactly what was staged.
function Get-StagedFile([string]$Name) {
  if (-not $script:StageHashes -or -not $script:StageHashes.ContainsKey($Name)) { throw "$Name was not staged for the administrator window" }
  $p = Join-Path (Get-AdminDir 'stage') $Name
  if (-not (Test-Path -LiteralPath $p) -or (Get-FileHash -LiteralPath $p -Algorithm SHA256).Hash -ne $script:StageHashes[$Name]) {
    throw "the staged copy of $Name was changed after it was staged; refused"
  }
  return $p
}

# Children of the administrator window load modules from system folders only.
function Set-SafeModulePath {
  if (-not $script:IsWin) { return }
  $paths = @((Join-Path $PSHOME 'Modules'), (Join-Path ([Environment]::GetFolderPath('ProgramFiles')) 'WindowsPowerShell\Modules'), (Join-Path ([Environment]::SystemDirectory) 'WindowsPowerShell\v1.0\Modules'))
  $env:PSModulePath = (@($paths | Select-Object -Unique) -join ';')
}

# A server name PC2 may type: a computer name, a DNS name or an IPv4 address. Letters,
# digits, dots, hyphens and underscores (Windows computer names often hold one), 1 to 253
# characters, not starting or ending with a dot or hyphen: the same rule as the pairing
# slip's host (hy-core slip::host_ok). Never spaces or other characters.
function Test-ServerName([string]$Name) {
  return [bool]($Name -match '^(?=[A-Za-z0-9._-]{1,253}\z)[A-Za-z0-9_](?:[A-Za-z0-9._-]*[A-Za-z0-9_])?\z')
}
$script:ServerNameRule = 'Letters, digits, dots, hyphens and underscores only (no spaces).'

# The server puts this PC's computer name into every pairing slip (hy-server --host
# defaults to it), and slips accept only names that pass Test-ServerName. A name that
# fails cannot pair at all, so the pairing step stops with this reason instead of
# sending the owner into a flow that cannot work. $null means the name is usable.
function Get-ServerNameProblem([string]$ComputerName) {
  if ($ComputerName -and (Test-ServerName $ComputerName)) { return $null }
  return "this PC's computer name has characters a pairing slip cannot hold, so the admin page cannot issue a slip. Rename the PC (Settings > System > About > Rename this PC) using only letters, digits, dots, hyphens and underscores, restart, then run this step again with: $(Get-Launcher 'Server') -Redo $(if ($script:RetestName) { 'R1-05' } else { 'P1-05' })"
}

$MirrorPattern = '^(?:[A-Za-z]:\\|\\\\[A-Za-z0-9._-]+\\)[A-Za-z0-9._\\-]{0,200}$'

function Test-AdminParams([string]$Op, [hashtable]$P) {
  switch ($Op) {
    'server-install' { if ($P.mirror -and $P.mirror -notmatch $MirrorPattern) { return 'the mirror folder must be a drive or share path without spaces' } }
    'server-replace' {
      if ($P.mirror -and $P.mirror -notmatch $MirrorPattern) { return 'the mirror folder must be a drive or share path without spaces' }
      if ([string]$P.exe_sha256 -cnotmatch '^[0-9A-F]{64}\z') { return 'the kit''s program SHA-256 is missing or malformed' }
      foreach ($k in @($P.Keys)) { if (@('mirror', 'exe_sha256') -notcontains $k) { return "unexpected parameter $k" } }
    }
    'soak-server' { $h = 0.0; if (-not [double]::TryParse([string]$P.hours, [ref]$h) -or $h -le 0 -or $h -gt 24) { return 'hours must be between 0 and 24' } }
    'network-capture' {
      if (-not (Test-ServerName ([string]$P.server))) { return 'the server name may hold only letters, digits, dots, hyphens and underscores' }
      if ([string]$P.canary -notmatch '^[A-Z0-9-]{4,40}$') { return 'bad pairing label' }
    }
    'server-pairing-evidence' { $t = [datetime]::MinValue; if (-not [datetime]::TryParse([string]$P.since, [ref]$t)) { return 'bad time' } }
  }
  return $null
}

# ------------------------------------------------- the kit's own builds (pairing retest)

# What to do with an installed program, from its SHA-256 and the SHA-256 of the program the
# kit's installer holds (kit-manifest.json): install (none installed), keep (byte for byte
# the kit's build), replace (anything else). Callers refuse before deciding when the kit
# records no valid SHA-256, so nothing is ever uninstalled without one. Version numbers are
# not compared: a first run's build and this kit's can carry the same one.
function Get-ReplaceDecision([string]$InstalledHash, [string]$KitHash) {
  if (-not $InstalledHash) { return 'install' }
  if ($KitHash -and ($InstalledHash.ToUpperInvariant() -ceq $KitHash.ToUpperInvariant())) { return 'keep' }
  return 'replace'
}

# The SHA-256 of the program inside a kit installer, as CI recorded it ('' when absent).
function Get-KitBuildHash([string]$Kind) {
  if (-not $script:Manifest) { return '' }
  $v = [string]$script:Manifest["$($Kind)_exe_sha256"]
  if ($v -cmatch '^[0-9A-Fa-f]{64}\z') { return $v.ToUpperInvariant() }
  return ''
}

function Get-FileSha256OrEmpty([string]$Path) {
  if ($Path -and (Test-Path -LiteralPath $Path -PathType Leaf)) { return (Get-FileSha256 $Path) }
  return ''
}

# A first run's soak or camera session still running would break if its program were
# replaced under it.
function Get-RunningKitTests {
  if ($script:Sim) { return @($script:Sim.running_tests | Where-Object { $_ }) }
  $found = @()
  try {
    foreach ($p in @(Get-CimInstance Win32_Process -Filter "Name = 'powershell.exe' OR Name = 'pwsh.exe'" -ErrorAction Stop)) {
      foreach ($n in 'Test-Soak.ps1', 'Test-CameraSoak.ps1', 'Test-HealthierYou-Camera.ps1') { if ([string]$p.CommandLine -match [regex]::Escape($n)) { $found += $n } }
    }
  } catch { }
  return @($found | Sort-Object -Unique)
}

# The office server's folders: known folders, never environment variables a user may set.
function Get-ServerProgramDir { Join-Path ([Environment]::GetFolderPath('ProgramFiles')) 'HealthierYou Server' }
function Get-ServerDataDir { Join-Path (Join-Path ([Environment]::GetFolderPath('CommonApplicationData')) 'HealthierYou') 'Server' }

# In -SelfTest and -DryRun $script:SimServer stands for the installed server.
function Get-InstalledServerHash {
  if ($script:SimServer) { return [string]$script:SimServer.exe }
  return (Get-FileSha256OrEmpty (Join-Path (Get-ServerProgramDir) 'hy-server.exe'))
}

function Get-InstalledServerMirror {
  if ($script:SimServer) { return [string]$script:SimServer.mirror }
  try { return [string](Get-ItemProperty -Path 'HKLM:\Software\HealthierYou\Server' -ErrorAction Stop).MirrorFolder } catch { return '' }
}

# What a reinstall must keep: the server's identity (its TLS certificate, so the six words
# stay), its database (users, devices, the admin passphrase) and its remembered mirror.
function Get-ServerDataMark {
  if ($script:SimServer) { return @{ cert = [string]$script:SimServer.cert; db = [bool]$script:SimServer.db; mirror_known = [bool]$script:SimServer.mirror_known } }
  $d = Get-ServerDataDir
  return @{ cert = (Get-FileSha256OrEmpty (Join-Path $d 'keys\tls.cert.der')); db = (Test-Path -LiteralPath (Join-Path $d 'server.db')); mirror_known = (Test-Path -LiteralPath (Join-Path $d 'mirror-location.txt')) }
}

# The installed server's own uninstaller (Program Files, which only administrators can
# change), silent and without /DELETEDATA, so keys, database, backups and the mirror stay
# (server-installer.nsi, Section "Uninstall"). NSIS runs it from a temporary copy and
# returns at once, so wait for the service, the entry and the program to go. $null = done.
function Invoke-ServerUninstall {
  if ($script:SimServer) {
    $script:SimServer.calls += 'server-uninstall'
    if ($script:SimServer.uninstall_fails) { return 'the old server was not fully removed (simulated)' }
    $script:SimServer.exe = ''
    if ($script:SimServer.uninstall_loses_data) { $script:SimServer.cert = ''; $script:SimServer.db = $false }
    return $null
  }
  $u = Join-Path (Get-ServerProgramDir) 'uninstall.exe'
  if (-not (Test-Path -LiteralPath $u)) { return 'the installed server has no uninstall.exe' }
  Write-Log 'Removing the installed server, keeping its data (silent)...' 'Cyan'
  $p = Start-Process -FilePath $u -ArgumentList '/S' -PassThru
  $null = $p.Handle
  if (-not $p.WaitForExit(600000)) { return 'the uninstaller did not finish within 10 minutes' }
  $key = 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\HealthierYouServer'
  $deadline = (Get-Date).AddMinutes(3)
  while ((Get-Date) -lt $deadline) {
    $left = [bool](Get-Service -Name 'HealthierYouServer' -ErrorAction SilentlyContinue) -or (Test-Path $key) -or (Test-Path -LiteralPath (Join-Path (Get-ServerProgramDir) 'hy-server.exe')) -or (@(Get-Process -Name 'Au_' -ErrorAction SilentlyContinue).Count -gt 0)
    if (-not $left) { return $null }
    Start-Sleep -Milliseconds 500
  }
  return 'the old server was not fully removed within 3 minutes (its service, entry or program is still there)'
}

$SetupMeaning = @{ 0 = 'done'; 2 = 'bad options or not an administrator'; 3 = 'refused (signature or version)'; 4 = 'new version did not start, previous put back'; 5 = 'the service did not start'; 6 = 'the mirror is not the data''s own'; 7 = 'not confirmed stopped or copies not verified'; 8 = 'data-folder rights could not be set'; 9 = 'rollback could not complete' }

# The staged (hash-checked) server installer, silent. Returns its exit code.
function Invoke-ServerSetup([string]$Mirror) {
  $inst = Get-StagedFile $script:StagedInstaller
  $a = '/S'
  if ($Mirror) { $a += " /MIRROR=$Mirror" }
  if ($script:SimServer) {
    $script:SimServer.calls += "server-setup $a"
    $code = [int]$script:SimServer.setup_code
    if ($code -eq 0) { $script:SimServer.exe = [string]$script:SimServer.kit_exe; if (-not $script:SimServer.cert) { $script:SimServer.cert = 'NEW'; $script:SimServer.db = $true } }
    return $code
  }
  Write-Log "Installing the server silently ($a)..." 'Cyan'
  $proc = Start-Process -FilePath $inst -ArgumentList $a -WorkingDirectory (Get-AdminDir 'stage') -PassThru
  $null = $proc.Handle; $proc.WaitForExit()
  return [int]$proc.ExitCode
}

# The service running, its firewall rule and TCP 8443 listening.
function Get-ServerHealth {
  if ($script:SimServer) { return @{ ok = [bool]$script:SimServer.healthy; notes = @("service $(if ($script:SimServer.healthy) { 'running' } else { 'NOT running' }) (simulated)") } }
  $notes = @()
  $running = $false; $svc = $null
  for ($i = 0; $i -lt 60; $i++) {
    $svc = Get-CimInstance Win32_Service -Filter "Name = 'HealthierYouServer'" -ErrorAction SilentlyContinue
    if ($svc -and $svc.State -eq 'Running') { $running = $true; break }
    Start-Sleep -Seconds 2
  }
  $notes += "service $(if ($running) { 'running' } else { 'NOT running' }) ($(if ($svc) { $svc.StartName -replace '^.*\\', '' } else { 'missing' }))"
  $rule = @(Get-NetFirewallRule -DisplayName 'HealthierYou Server' -ErrorAction SilentlyContinue)
  $ruleOk = $rule.Count -gt 0 -and @($rule | Where-Object { $_.Enabled -eq 'True' -or $_.Enabled -eq $true }).Count -gt 0
  $profiles = @(); try { $profiles = @($rule | ForEach-Object { [string]$_.Profile }) } catch { }
  $notes += "firewall rule $(if ($ruleOk) { "present ($($profiles -join ','))" } else { 'MISSING' })"
  $listen = @(Get-NetTCPConnection -LocalPort 8443 -State Listen -ErrorAction SilentlyContinue).Count -gt 0
  $notes += "TCP 8443 $(if ($listen) { 'listening' } else { 'NOT listening' })"
  return @{ ok = ($running -and $ruleOk -and $listen); notes = $notes }
}

# The server's six words (hy-server fingerprint reads the administrators-only keys).
function Get-ServerWords {
  if ($script:SimServer) { return [string]$script:SimServer.words }
  $exe = Join-Path (Get-ServerProgramDir) 'hy-server.exe'
  if (-not (Test-Path -LiteralPath $exe)) { return '' }
  $fp = @(& $exe fingerprint 2>&1 | ForEach-Object { [string]$_ })
  return ((@($fp | Where-Object { $_ -like 'six words*' }) -replace '^six words\s+', '') -join '')
}

# The pairing retest's server step, elevated. An installed server refuses the same version
# over itself and any installer signed with another CI run's key (verify-release; by
# design), so a server that is not the kit's build is uninstalled keeping its data and
# installed again from the staged installer. The installer finds the data's remembered
# mirror (mirror-location.txt) and the service starts on the kept keys and database.
function Invoke-ServerReplace([hashtable]$P) {
  $kitExe = [string]$P.exe_sha256
  # Without the kit's program SHA-256 the result could never be shown to be the kit's
  # build, so nothing is touched (Test-AdminParams refuses this too).
  if ($kitExe -cnotmatch '^[0-9A-F]{64}\z') { return New-Result 'INCONCLUSIVE' 'not replaced: the kit records no valid hy-server.exe SHA-256 (kit-manifest.json), so nothing was changed' }
  $decision = Get-ReplaceDecision (Get-InstalledServerHash) $kitExe
  $before = Get-ServerDataMark
  $notes = @()
  if ($decision -eq 'keep') { $notes += 'the installed server is already the kit''s build (hy-server.exe SHA-256 matches); not installed again' }
  else {
    if (-not $script:StagedInstaller) { return New-Result 'INCONCLUSIVE' 'the server installer is not in the kit' }
    $busy = @(Get-RunningKitTests)
    if ($busy.Count) { return New-Result 'INCONCLUSIVE' "not replaced: $($busy -join ', ') is still running (a first-run soak or session). Start the retest again when it has finished." }
    # The staged installer is checked before the old server is removed: a changed copy
    # stops here, with the old server still in place.
    $null = Get-StagedFile $script:StagedInstaller
    # The data's own mirror is remembered with the data (mirror-location.txt) and the
    # installer finds it. Without that file: the installed server's mirror, or for a fresh
    # install the one the kit window chose. Known before anything is removed, or refused.
    $mirror = ''
    if (-not $before.mirror_known) {
      $mirror = if ($decision -eq 'replace') { Get-InstalledServerMirror } else { [string]$P.mirror }
      if (-not $mirror -or $mirror -notmatch $MirrorPattern) { return New-Result 'INCONCLUSIVE' (($notes + 'not replaced: the server data''s mirror folder is not known, so nothing was changed') -join '; ') }
    }
    if ($decision -eq 'replace') {
      $notes += "the installed server is not the kit's build ($(if ($kitExe) { 'its hy-server.exe SHA-256 differs' } else { 'the kit records no program SHA-256' })): uninstalled keeping its data, then installed from the kit"
      $why = Invoke-ServerUninstall
      if ($why) { return New-Result 'FAIL' (($notes + "uninstall: $why") -join '; ') }
      $mid = Get-ServerDataMark
      if ($before.cert -and ($mid.cert -cne $before.cert -or ($before.db -and -not $mid.db))) { return New-Result 'FAIL' (($notes + 'the server data (keys or database) did not survive the uninstall, so nothing was installed') -join '; ') }
    }
    else { $notes += 'no server was installed: installed from the kit' }
    $code = Invoke-ServerSetup $mirror
    $notes += "setup exit code $code ($($SetupMeaning[$code]))"
    if ($code -ne 0) { return New-Result 'FAIL' ($notes -join '; ') }
  }
  $h = Get-ServerHealth
  $notes += $h.notes
  $after = Get-InstalledServerHash
  $isKit = [bool]($kitExe -and $after -and ($after -ceq $kitExe))
  $notes += "installed hy-server.exe $(if ($isKit) { 'is the kit''s build' } elseif ($kitExe) { 'is NOT the kit''s build' } else { 'not compared (the kit records no program SHA-256)' })"
  $now = Get-ServerDataMark
  $kept = [bool]($before.cert -and $before.db -and ($now.cert -ceq $before.cert) -and $now.db)
  $lost = [bool]($before.cert -and -not $kept)
  if ($decision -ne 'keep') { $notes += "server data (keys, and the database with the admin passphrase) $(if ($kept) { 'kept: the six words and the passphrase are the same as before' } elseif ($lost) { 'NOT kept' } else { 'new (there was none)' })" }
  $words = Get-ServerWords
  if (-not $words) { $notes += 'six words not read' }
  $v = if (-not $h.ok -or $lost -or ($kitExe -and -not $isKit)) { 'FAIL' } elseif (-not $kitExe -or -not $words) { 'INCONCLUSIVE' } else { 'PASS' }
  $r = New-Result $v ($notes -join '; ')
  # Back to the kit window only (not written to the results).
  $r.words = $words
  $r.passphrase_needed = -not ($now.db -and ($kept -or $decision -eq 'keep'))
  $r.data_kept = $kept
  return $r
}

# Run a staged script for a report the agent names itself (old copies removed first).
function Invoke-StagedReport([string]$Script, [string[]]$Arguments, [string]$ReportName) {
  $report = Join-Path (Get-AdminResultsDir) $ReportName
  $run = Invoke-ReportedScript $Script ($Arguments + @('-Report', $report)) @($report, "$report.json") -Staged
  return @{ run = $run; report = $report; files = @($ReportName, "$ReportName.json") }
}

function Invoke-AdminOp([string]$Op, [hashtable]$P) {
  if ($AdminOps -notcontains $Op) { return New-Result 'INCONCLUSIVE' "unknown administrator step '$Op' refused" }
  if ((Get-AllowedAdminOps) -notcontains $Op) { return New-Result 'INCONCLUSIVE' "administrator step '$Op' refused: not part of this $(if ($script:RetestName) { 'retest' } else { 'run' })" }
  $bad = Test-AdminParams $Op $P
  if ($bad) { return New-Result 'INCONCLUSIVE' "refused: $bad" }
  if ($script:AdminStub) { $s = & $script:AdminStub $Op $P; if ($s) { return $s } }
  if ($Op -eq 'echo') { return New-Result 'PASS' "echo $($P.text)" }
  if (-not $script:AdminReady) { return New-Result 'INCONCLUSIVE' "the administrator folder is not ready: $($script:AdminSetupError)" }
  $serverExe = if ($env:ProgramFiles) { Join-Path $env:ProgramFiles 'HealthierYou Server\hy-server.exe' } else { '' }
  switch ($Op) {
    'server-install' {
      $svc = Get-CimInstance Win32_Service -Filter "Name = 'HealthierYouServer'" -ErrorAction SilentlyContinue
      $notes = @()
      if ($svc) {
        $notes += "already installed (version $((Get-ItemProperty 'HKLM:\Software\HealthierYou\Server' -ErrorAction SilentlyContinue).Version)); not installed again"
      }
      else {
        if (-not $script:StagedInstaller) { return New-Result 'INCONCLUSIVE' 'the server installer is not in the kit' }
        $inst = Get-StagedFile $script:StagedInstaller
        $a = '/S'
        if ($P.mirror) { $a += " /MIRROR=$($P.mirror)" }
        Write-Log "Installing the server silently ($a)..." 'Cyan'
        $proc = Start-Process -FilePath $inst -ArgumentList $a -WorkingDirectory (Get-AdminDir 'stage') -PassThru
        $null = $proc.Handle; $proc.WaitForExit()
        $code = [int]$proc.ExitCode
        $notes += "setup exit code $code ($($SetupMeaning[$code]))"
        if ($code -ne 0) { return New-Result 'FAIL' ($notes -join '; ') }
      }
      $h = Get-ServerHealth
      $notes += $h.notes
      return New-Result $(if ($h.ok) { 'PASS' } else { 'FAIL' }) ($notes -join '; ')
    }
    'server-passphrase' {
      if (-not $serverExe -or -not (Test-Path -LiteralPath $serverExe)) { return New-Result 'INCONCLUSIVE' 'the server is not installed' }
      $set = $null
      Show-Banner 'Admin passphrase for the server' @('Choose a passphrase of at least 12 characters for the admin page.', 'Write it down somewhere safe (not in a file on this PC). It is not saved by the kit.', 'Type n at the first question if a passphrase was already set on this server.')
      if (Read-YesNo 'Set (or change) the admin passphrase now?' $true) {
        for ($try = 0; $try -lt 3 -and $null -eq $set; $try++) {
          $a = Read-Host '  Type the passphrase' -AsSecureString
          $b = Read-Host '  Type it again' -AsSecureString
          $pa = [Runtime.InteropServices.Marshal]::PtrToStringBSTR([Runtime.InteropServices.Marshal]::SecureStringToBSTR($a))
          $pb = [Runtime.InteropServices.Marshal]::PtrToStringBSTR([Runtime.InteropServices.Marshal]::SecureStringToBSTR($b))
          if ($pa -ne $pb) { Write-Host '  They differ. Try again.' -ForegroundColor Red; continue }
          if ($pa.Length -lt 12) { Write-Host '  At least 12 characters, please.' -ForegroundColor Red; continue }
          $psi = New-Object Diagnostics.ProcessStartInfo
          $psi.FileName = $serverExe; $psi.Arguments = 'set-passphrase'; $psi.UseShellExecute = $false; $psi.RedirectStandardInput = $true
          $pr = [Diagnostics.Process]::Start($psi)
          $pr.StandardInput.WriteLine($pa); $pr.StandardInput.Close(); $pr.WaitForExit()
          $pa = $null; $pb = $null
          $set = ($pr.ExitCode -eq 0)
        }
      }
      else { $set = 'kept' }
      $fp = @(& $serverExe fingerprint 2>&1 | ForEach-Object { [string]$_ })
      $words = (@($fp | Where-Object { $_ -like 'six words*' }) -replace '^six words\s+', '') -join ''
      if ($words) { Show-Banner 'The server''s six words' @($words.ToUpperInvariant(), '', 'PC2 must show exactly these words when it pairs.') 'Green' }
      $v = if (($set -eq $true -or $set -eq 'kept') -and $words) { 'PASS' } elseif ($set -eq $false) { 'FAIL' } else { 'INCONCLUSIVE' }
      $r = New-Result $v "passphrase $(if ($set -eq 'kept') { 'kept as it was' } elseif ($set) { 'set' } else { 'NOT set' }); six words $(if ($words) { 'shown' } else { 'not read' })"
      # The words go back to the kit window only (they are not written to the results).
      $r.words = $words
      return $r
    }
    'server-replace' { return (Invoke-ServerReplace $P) }
    'server-pairing-evidence' {
      $since = ([datetime]::Parse([string]$P.since)).ToUniversalTime()
      $logs = Join-Path $env:ProgramData 'HealthierYou\Server\logs'
      $pairOk = 0; $confirmedOk = 0; $devices = @{}; $errors = 0
      foreach ($f in @(Get-ChildItem -LiteralPath $logs -File -Filter 'server.log*' -ErrorAction SilentlyContinue)) {
        foreach ($line in (Read-SharedLines $f.FullName)) {
          try { $o = $line | ConvertFrom-Json } catch { continue }
          if (-not $o.timestamp -or [datetime]::Parse($o.timestamp).ToUniversalTime() -lt $since) { continue }
          if ($o.fields.event -ne 'request') { continue }
          try { $d = $o.fields.detail | ConvertFrom-Json } catch { continue }
          if ([int]$d.status -ge 500) { $errors++ }
          if ($d.endpoint -eq 'pair' -and [int]$d.status -eq 200) { $pairOk++ }
          if (@('history', 'upload', 'receipt', 'control', 'manifest') -contains [string]$d.endpoint -and [int]$d.status -eq 200) {
            $confirmedOk++
            if ($d.device) { $devices[[string]$d.device] = 1 }
          }
        }
      }
      $r = New-Result $(if ($pairOk -ge 1 -and $confirmedOk -ge 1) { 'PASS' } else { 'INCONCLUSIVE' }) "server log since the step began: $pairOk pairing(s) accepted; $confirmedOk answered request(s) only a confirmed PC may make, from $($devices.Count) PC(s); $errors 5xx"
      $r.pairings = $pairOk; $r.confirmed_devices = $devices.Count
      return $r
    }
    'soak-server' {
      $x = Invoke-StagedReport 'Test-Soak.ps1' @('-Role', 'Server', '-Hours', [string]$P.hours) 'hy-soak-server.txt'
      $v = Limit-Verdict (Get-SoakVerdict ($x.report + '.json') $x.run.code) $x.run
      $r = New-Result $v.verdict $v.detail; $r.files = $x.files
      return $r
    }
    'network-capture' {
      Show-Banner 'Network capture with the pairing' @("When HealthierYou asks for a PC name while pairing, type:   $($P.canary)", 'The script below calls it the -Canary text. Follow its five phases; PC1 confirms in phase PairedOff.') 'Cyan'
      $x = Invoke-StagedReport 'Test-NetworkCapture.ps1' @('-Server', [string]$P.server, '-Canary', [string]$P.canary, '-Out', (Get-AdminDir 'raw')) 'hy-network-capture.txt'
      $v = Limit-Verdict (Get-ExitVerdict $x.run.code ($x.report + '.json') 'Overall') $x.run
      $r = New-Result $v.verdict $v.detail; $r.files = $x.files
      return $r
    }
  }
}

# Process one waiting request (the agent's loop calls this; the self-test calls it directly).
function Invoke-AgentOnce {
  $q = Get-AdminDir 'queue'; $out = Get-AdminDir 'out'
  $job = @(Get-ChildItem -LiteralPath $q -Filter 'job-*.json' -File -ErrorAction SilentlyContinue | Where-Object { $_.Name -match '^job-\d{17}-[a-z-]{1,40}\.json$' } | Sort-Object Name | Select-Object -First 1)
  if (-not $job.Count) { return $false }
  $job = $job[0]
  $base = $job.Name.Substring(0, $job.Name.Length - 5)
  $text = $null
  if ($job.Length -le 65536) { try { $text = [IO.File]::ReadAllText($job.FullName) } catch { } }
  # The marker goes down before the request is removed, so the step always shows as queued
  # or running. The agent's PID comes first: a marker whose agent is gone is a step that
  # will never finish.
  $running = Join-Path $out "$base.running"
  Set-Content -LiteralPath $running -Value "$PID $(Get-NowText)"
  Remove-Item -LiteralPath $job.FullName -Force -ErrorAction SilentlyContinue
  $r = $null
  try {
    $j = $null; if ($text) { try { $j = ConvertTo-HashtableDeep ($text | ConvertFrom-Json) } catch { } }
    if (-not $j -or -not $j.op) { $r = New-Result 'INCONCLUSIVE' 'unreadable request' }
    else {
      $params = if ($j.params -is [hashtable]) { $j.params } else { @{} }
      Write-Log "Administrator step: $($j.op)" 'Cyan'
      $r = Invoke-AdminOp ([string]$j.op) $params
    }
  }
  catch { $r = New-Result 'INCONCLUSIVE' "the administrator step stopped: $($_.Exception.Message)" }
  Write-JsonFile (Join-Path $out "$base.result.json") $r
  Remove-Item -LiteralPath $running -Force -ErrorAction SilentlyContinue
  Write-Log "Administrator step done: $($r.verdict)" 'Cyan'
  return $true
}

function Start-AgentLoop([int]$Owner) {
  try { $Host.UI.RawUI.WindowTitle = "HealthierYou test kit - ADMINISTRATOR ($script:RoleName)" } catch { }
  Set-SafeModulePath
  Initialize-AdminArea
  $out = Get-AdminDir 'out'; $q = Get-AdminDir 'queue'
  if ($Owner -gt 0) { Set-Content -LiteralPath (Join-Path $q 'owner.pid') -Value $Owner }
  Set-Content -LiteralPath (Join-Path $out 'agent.pid') -Value $PID
  Show-Banner 'HealthierYou test kit: administrator window' @('Keep this window open. The kit sends its administrator steps here.', 'Some of them ask you something: the kit window tells you when to look here.')
  $goneSince = $null
  while ($true) {
    $did = Invoke-AgentOnce
    $ownerPid = 0; try { $ownerPid = [int](Get-Content -LiteralPath (Join-Path $q 'owner.pid') -ErrorAction Stop) } catch { }
    $alive = $ownerPid -gt 0 -and [bool](Get-Process -Id $ownerPid -ErrorAction SilentlyContinue)
    if ($alive) { $goneSince = $null } elseif (-not $goneSince) { $goneSince = Get-Date }
    if ($goneSince -and ((Get-Date) - $goneSince).TotalSeconds -gt 120 -and -not $did) { break }
    if (-not $did) { Start-Sleep -Seconds 1 }
  }
  Remove-Item -LiteralPath (Join-Path $out 'agent.pid') -ErrorAction SilentlyContinue
}

# Self-test of the bootstrap: set up as the agent does, answer the waiting requests, exit.
function Invoke-AgentRequestsOnce {
  Set-SafeModulePath
  Initialize-AdminArea
  while (Invoke-AgentOnce) { }
  if ($script:AdminReady) { return 0 } else { return 4 }
}

function Test-AgentAlive {
  if ($script:AgentInline) { return $true }
  $f = Join-Path (Get-AdminDir 'out') 'agent.pid'
  if (-not (Test-Path -LiteralPath $f)) { return $false }
  try { $id = [int](Get-Content -LiteralPath $f) } catch { return $false }
  $p = Get-Process -Id $id -ErrorAction SilentlyContinue
  return [bool]($p -and $p.ProcessName -match '^(powershell|pwsh)$')
}

function Initialize-InlineAdmin {
  if ($script:InlineAdminDone) { return }
  Set-SafeModulePath
  try { $script:PinnedDigests = Get-StagePins } catch { $script:PinnedDigests = $null; Write-Log "Administrator steps refused: $($_.Exception.Message)" 'Red' }
  Initialize-AdminArea
  $script:InlineAdminDone = $true
}

# The elevated entry: a small bootstrap passed with -EncodedCommand (no -File on any path the
# user can write). The kit window hashes this script's bytes when it starts. Before the UAC
# prompt it embeds as checked literals: that SHA-256, the role, the paths, and the pinned
# SHA-256 of every file the agent will stage (Get-StagePins: hashed from the files and
# checked against kit-manifest.json). Elevated, the bootstrap reads the user's copy of the
# script once into memory and refuses (exit 3, writing nothing) unless those bytes have the
# embedded hash. Then it makes <admin>\boot (Administrators and SYSTEM only; a folder or
# link a user made earlier is moved aside), writes those same bytes there, checks the copy's
# hash and runs that copy as the agent. The pins reach the agent only as its -StagePins
# argument, from this bootstrap, never from a file the user can write; the agent refuses any
# file to stage without a pin or with a different SHA-256. The module path is system
# folders only from the first line, before any module can load.
$AgentBootstrapTemplate = @'
$ErrorActionPreference = 'Stop'
$env:PSModulePath = (@([IO.Path]::Combine($PSHOME, 'Modules'), [IO.Path]::Combine([Environment]::GetFolderPath('ProgramFiles'), 'WindowsPowerShell\Modules'), [IO.Path]::Combine([Environment]::SystemDirectory, 'WindowsPowerShell\v1.0\Modules')) -join [IO.Path]::PathSeparator)
function D([string]$s) { [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($s)) }
$role = '@ROLE@'; $retest = '@RETEST@'; $want = '@SHA@'; $src = D '@SRC@'; $root = D '@ROOT@'; $acl = @ACL@; $pause = @PAUSE@
$run = @{ Agent = $true; Role = $role; StateDir = (D '@STATE@'); InstallersDir = (D '@INST@'); ScriptsDir = (D '@SCRIPTS@'); OwnerPid = @OWNER@; StagePins = '@PINS@' }
if ($root) { $run.AdminRoot = $root }; if (@ONCE@) { $run.AgentOnce = $true }; if ($retest) { $run.Retest = $retest }
function Fail([int]$Code, [string]$Msg) { Write-Host $Msg -ForegroundColor Red; if ($pause) { try { $null = Read-Host 'Press Enter to close this window' } catch { } }; exit $Code }
function Hash([byte[]]$b) { $h = [Security.Cryptography.SHA256]::Create(); try { ([BitConverter]::ToString($h.ComputeHash($b))) -replace '-', '' } finally { $h.Dispose() } }
function Safe([string]$p, [bool]$fresh) {
  $i = Get-Item -LiteralPath $p -Force -ErrorAction SilentlyContinue
  if ($i) {
    $bad = [bool]($i.Attributes -band [IO.FileAttributes]::ReparsePoint) -or -not $i.PSIsContainer
    if (-not $bad -and $acl) { $bad = @('S-1-5-32-544', 'S-1-5-18') -notcontains (Get-Acl -LiteralPath $p).GetOwner([Security.Principal.SecurityIdentifier]).Value }
    if ($bad) { Move-Item -LiteralPath $p -Destination ("$p.untrusted-" + [guid]::NewGuid().ToString('N').Substring(0, 8)) -Force }
    elseif ($fresh) { Remove-Item -LiteralPath $p -Recurse -Force }
  }
  if (-not (Get-Item -LiteralPath $p -Force -ErrorAction SilentlyContinue)) { $null = New-Item -ItemType Directory -Path $p }
  if ($acl) {
    $ic = [IO.Path]::Combine([Environment]::SystemDirectory, 'icacls.exe')
    & $ic $p /setowner '*S-1-5-32-544' /Q | Out-Null; if ($LASTEXITCODE -ne 0) { throw "icacls /setowner failed ($LASTEXITCODE)" }
    & $ic $p /inheritance:r /grant:r '*S-1-5-18:(OI)(CI)F' '*S-1-5-32-544:(OI)(CI)F' '*S-1-5-32-545:(OI)(CI)RX' /Q | Out-Null; if ($LASTEXITCODE -ne 0) { throw "icacls failed ($LASTEXITCODE)" }
  }
}
try {
  $bytes = [IO.File]::ReadAllBytes($src)
  if ((Hash $bytes) -ne $want) { Fail 3 'Refused: the kit script changed after the kit window started. Nothing was run as administrator. Close the kit and start it again from a fresh copy.' }
  if (-not $root) { $top = [IO.Path]::Combine([Environment]::GetFolderPath('CommonApplicationData'), 'HealthierYou-TestKit-Admin'); Safe $top $false; $sub = $role; if ($retest) { $sub = "$role-Retest" }; $root = [IO.Path]::Combine($top, $sub) }
  Safe $root $false
  $boot = [IO.Path]::Combine($root, 'boot')
  Safe $boot $true
  $staged = [IO.Path]::Combine($boot, 'Invoke-OfficeTestKit.ps1')
  [IO.File]::WriteAllBytes($staged, $bytes)
  if ((Hash ([IO.File]::ReadAllBytes($staged))) -ne $want) { Fail 3 'Refused: the protected copy of the kit script does not match.' }
} catch { Fail 2 "The administrator window could not start: $($_.Exception.Message)" }
$global:LASTEXITCODE = 0
& $staged @run
exit $LASTEXITCODE
'@

function Get-FileSha256([string]$Path) {
  $h = [Security.Cryptography.SHA256]::Create()
  try { return (([BitConverter]::ToString($h.ComputeHash([IO.File]::ReadAllBytes($Path)))) -replace '-', '') } finally { $h.Dispose() }
}

$PinPattern = '^[A-Za-z0-9._-]{1,120}=[0-9A-F]{64}(;[A-Za-z0-9._-]{1,120}=[0-9A-F]{64}){0,15}$'

# The kit window, before the UAC prompt: the SHA-256 of each file the agent will stage, from
# the files themselves, each required to match kit-manifest.json.
function Get-StagePins {
  $pins = [ordered]@{}
  foreach ($f in (Get-StageList)) {
    if (-not (Test-Path -LiteralPath $f.src)) { throw "the kit is incomplete: $($f.name) is missing" }
    $h = Get-FileSha256 $f.src
    $m = if ($script:Manifest -and $script:Manifest.sha256) { [string]$script:Manifest.sha256[$f.name] } else { '' }
    if (-not $m) { throw "$($f.name) has no SHA-256 in kit-manifest.json" }
    if ($m.ToUpperInvariant() -ne $h) { throw "$($f.name) does not match kit-manifest.json: it was changed after the kit was built" }
    $pins[$f.name] = $h
  }
  return $pins
}
function ConvertTo-PinText($Pins) { return (@($Pins.Keys | ForEach-Object { "$_=$($Pins[$_])" }) -join ';') }
function ConvertFrom-PinText([string]$Text) {
  $pins = [ordered]@{}
  if ($Text -cmatch $PinPattern) { foreach ($p in $Text.Split(';')) { $kv = $p.Split('='); $pins[$kv[0]] = $kv[1] } }
  return $pins
}

# The bootstrap text with its literals checked first (a bad value is never embedded).
function New-AgentBootstrap([hashtable]$B) {
  if ([string]$B.role -cnotmatch '^(Server|Camera)$') { throw 'refused: bad role for the administrator window' }
  if ([string]$B.retest -cnotmatch '^(Pairing)?$') { throw 'refused: bad retest for the administrator window' }
  if ([string]$B.sha256 -cnotmatch '^[0-9A-F]{64}$') { throw 'refused: bad SHA-256 for the administrator window' }
  if ([string]$B.ownerPid -notmatch '^[0-9]{1,10}$') { throw 'refused: bad owner process id' }
  if (-not $B.src) { throw 'refused: no kit script path' }
  if ([string]$B.pins -cnotmatch $PinPattern) { throw 'refused: bad or missing SHA-256 pins for the files to stage' }
  $t = $AgentBootstrapTemplate
  $lit = @{ '@ROLE@' = [string]$B.role; '@RETEST@' = [string]$B.retest; '@SHA@' = [string]$B.sha256; '@OWNER@' = [string][int64]$B.ownerPid
    '@ACL@' = $(if ($B.acl) { '$true' } else { '$false' }); '@PAUSE@' = $(if ($B.pause) { '$true' } else { '$false' })
    '@ONCE@' = $(if ($B.once) { '$true' } else { '$false' }); '@PINS@' = [string]$B.pins }
  foreach ($k in 'src', 'root', 'state', 'inst', 'scripts') {
    $v = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes([string]$B[$k]))
    if ($v -cnotmatch '^[A-Za-z0-9+/=]*$') { throw "refused: bad $k" }
    $lit["@$($k.ToUpperInvariant())@"] = $v
  }
  foreach ($k in $lit.Keys) { $t = $t.Replace($k, $lit[$k]) }
  return $t
}

function ConvertTo-EncodedCommand([string]$Text) { [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($Text)) }

# The command line of the elevated window: the bootstrap only, never a script file.
function New-AgentLaunchBootstrap([string]$Root = '', [bool]$Acl = $true, [bool]$Pause = $true, [bool]$Once = $false) {
  return (New-AgentBootstrap @{ role = $script:RoleName; retest = [string]$script:RetestName; sha256 = $script:KitSha256; src = $script:KitScriptPath; root = $Root; acl = $Acl; pause = $Pause; once = $Once
    pins = (ConvertTo-PinText (Get-StagePins)); state = $script:StateDirResolved; inst = $script:InstallersDirResolved; scripts = $script:ScriptsDirResolved; ownerPid = $PID })
}
function Get-AgentLaunchArgs {
  $text = New-AgentLaunchBootstrap
  $line = '-NoProfile -ExecutionPolicy Bypass -EncodedCommand ' + (ConvertTo-EncodedCommand $text)
  if ($line.Length -gt 30000) { throw 'the administrator command line is too long' }
  return $line
}

# Start the administrator window once (one UAC prompt), or reuse one still open.
function Start-AdminAgent([string[]]$Why) {
  if ($script:AgentInline -or $script:AgentStarted) { return $true }
  if ((Test-Elevated) -or $script:Unattended) { $script:AgentInline = $true; return $true }
  if (Test-AgentAlive) {
    try { Set-Content -LiteralPath (Join-Path (Get-AdminDir 'queue') 'owner.pid') -Value $PID } catch { }
    $script:AgentStarted = $true; Write-Log 'Using the administrator window that is still open.'; return $true
  }
  Show-Banner 'One administrator prompt' (@('Some steps need an administrator:') + @($Why | ForEach-Object { "  - $_" }) + @('', 'Windows will now ask once: "Do you want to allow this app to make changes?"', 'Click Yes (or type an administrator password if Windows asks for one).', 'A second window, "HealthierYou test kit - ADMINISTRATOR", opens: keep it open.', 'Every other step runs as you, without administrator rights.'))
  while ($true) {
    $launch = $null
    try { $launch = Get-AgentLaunchArgs }
    catch { Write-Log "Administrator steps refused: $($_.Exception.Message). Download the kit again." 'Red'; return $false }
    try { Start-Process -FilePath (Get-HostExe) -ArgumentList $launch -Verb RunAs | Out-Null }
    catch {
      Write-Log "The administrator window was not started: $($_.Exception.Message)" 'Red'
      if (Read-YesNo 'Try the administrator prompt again?' $true) { continue }
      return $false
    }
    for ($i = 0; $i -lt 60 -and -not (Test-AgentAlive); $i++) { Start-Sleep -Seconds 1 }
    if (Test-AgentAlive) { $script:AgentStarted = $true; return $true }
    if (-not (Read-YesNo 'The administrator window did not open. Try again?' $true)) { return $false }
  }
}

# Copy the reports an administrator step wrote (in <admin>\out\results, readable) into the
# kit's results, fresh only: a report from before this step is never used.
function Copy-AdminReports($Result, [datetime]$Since) {
  if (-not ($Result -is [hashtable]) -or -not $Result.files) { return $Result }
  $missing = @()
  foreach ($n in @($Result.files)) {
    $dest = Get-ResultPath ([string]$n)
    Remove-Item -LiteralPath $dest -Force -ErrorAction SilentlyContinue
    $src = Join-Path (Get-AdminResultsDir) ([IO.Path]::GetFileName([string]$n))
    if ((Test-Path -LiteralPath $src) -and (Get-Item -LiteralPath $src).LastWriteTime -ge $Since) { Copy-Item -LiteralPath $src -Destination $dest -Force }
    else { $missing += [string]$n }
  }
  if ($missing.Count -and $Result.verdict -eq 'PASS') { $Result.verdict = 'INCONCLUSIVE'; $Result.detail = "$($Result.detail); report not copied: $($missing -join ', ')" }
  $Result.Remove('files')
  return $Result
}

# Hand an operation to the administrator window and wait for it (or, with -NoWait, return
# the job's name). A job still pending from an earlier run under the same key is waited
# for instead of being sent again (for example the 8-hour server soak after the kit window
# was closed).
# Whether the step `$Name` is still being worked on: waiting in the queue for the agent, or
# running in the agent that is open now. A running marker left by an administrator window
# that was closed mid-step (or written by an older kit, without the agent's PID) is not.
function Test-AdminJobLive([string]$Name) {
  if (Test-Path -LiteralPath (Join-Path (Get-AdminDir 'queue') "$Name.json")) { return $true }
  $marker = Join-Path (Get-AdminDir 'out') "$Name.running"
  if (-not (Test-Path -LiteralPath $marker)) { return $false }
  $owner = 0; try { $owner = [int](([string](Get-Content -LiteralPath $marker -TotalCount 1)) -split ' ')[0] } catch { }
  $agent = 0; try { $agent = [int](Get-Content -LiteralPath (Join-Path (Get-AdminDir 'out') 'agent.pid')) } catch { }
  return ($owner -gt 0 -and $owner -eq $agent -and (Test-AgentAlive))
}

function Invoke-AdminJob([string]$Op, [hashtable]$Params, [string]$Key, [switch]$NoWait) {
  $since = (Get-Date).AddSeconds(-2)
  if ($script:AgentInline) {
    Initialize-InlineAdmin
    $r = Copy-AdminReports (Invoke-AdminOp $Op $Params) $since
    if ($NoWait) { $script:InlineResults[$Key] = $r; return "inline:$Key" }
    return $r
  }
  if (-not (Test-AgentAlive)) { return New-Result 'INCONCLUSIVE' 'the administrator window is not open (the UAC prompt was declined or the window was closed)' }
  $q = Get-AdminDir 'queue'; $out = Get-AdminDir 'out'
  $jobs = $script:State.data['jobs']; if (-not ($jobs -is [hashtable])) { $jobs = @{}; $script:State.data['jobs'] = $jobs }
  $name = $null
  if ($Key -and $jobs[$Key]) {
    $old = [string]$jobs[$Key].name
    if ($old -and ((Test-Path -LiteralPath (Join-Path $out "$old.result.json")) -or (Test-AdminJobLive $old))) { $name = $old; Write-Log "Waiting for the administrator step started earlier ($Op)." }
    elseif ($old) { Write-Log "The administrator step started earlier ($Op) was interrupted (its window was closed): starting it again." }
  }
  if (-not $name) {
    $name = 'job-{0}-{1}' -f (Get-Date).ToString('yyyyMMddHHmmssfff'), $Op
    Write-JsonFile (Join-Path $q "$name.json") @{ op = $Op; params = $Params }
    if ($Key) { $jobs[$Key] = @{ name = $name; since = $since.ToString('o') }; Save-State }
  }
  if ($NoWait) { return $name }
  return (Wait-AdminJob $name $Key)
}

function Wait-AdminJob([string]$Name, [string]$Key) {
  if ($Name -like 'inline:*') { return $script:InlineResults[$Name.Substring(7)] }
  $res = Join-Path (Get-AdminDir 'out') "$Name.result.json"
  while (-not (Test-Path -LiteralPath $res)) {
    if ((Test-AgentAlive) -and -not (Test-AdminJobLive $Name)) {
      Start-Sleep -Seconds 2
      if (Test-Path -LiteralPath $res) { break }
      return New-Result 'INCONCLUSIVE' 'the administrator step was interrupted before it finished (its window was closed): run this step again'
    }
    if (-not (Test-AgentAlive)) {
      Start-Sleep -Seconds 2
      if (Test-Path -LiteralPath $res) { break }
      return New-Result 'INCONCLUSIVE' 'the administrator window was closed before the step finished'
    }
    Start-Sleep -Seconds 2
  }
  $r = Read-JsonFile $res
  $since = (Get-Date).AddDays(-2)
  if ($Key -and $script:State.data['jobs'] -is [hashtable]) {
    $j = $script:State.data['jobs'][$Key]
    if ($j -is [hashtable] -and $j.since) { $since = [datetime]::Parse([string]$j.since) }
    $script:State.data['jobs'].Remove($Key); Save-State
  }
  if (-not $r) { return New-Result 'INCONCLUSIVE' 'the administrator step left no readable result' }
  return (Copy-AdminReports $r $since)
}

# ===================================================================== zip and redaction

function Get-RedactionTokens {
  $t = New-Object System.Collections.Generic.List[string]
  foreach ($v in @($env:USERNAME, $env:USER, $env:COMPUTERNAME, $env:HOSTNAME, $env:USERDOMAIN, $env:USERDNSDOMAIN, $env:LOGONSERVER)) { if ($v) { $t.Add(($v -replace '^\\\\', '')) } }
  try { if ($env:USERPROFILE) { $t.Add((Split-Path -Leaf $env:USERPROFILE)) } } catch { }
  try { if ($env:HOME) { $t.Add((Split-Path -Leaf $env:HOME)) } } catch { }
  if ($script:IsWin -and -not $script:Sim) {
    try { $u = [string](Get-CimInstance Win32_ComputerSystem).UserName; if ($u) { $t.Add($u); $t.AddRange([string[]]($u -split '\\')) } } catch { }
    try { $t.Add([Net.Dns]::GetHostName()) } catch { }
  }
  if ($script:State -and $script:State.data) {
    foreach ($k in 'server_name', 'canary') { if ($script:State.data[$k]) { $t.Add([string]$script:State.data[$k]); $t.Add((([string]$script:State.data[$k]) -split '\.')[0]) } }
  }
  foreach ($x in $script:ExtraTokens) { if ($x) { $t.Add($x) } }
  return @($t | Where-Object { $_ -and $_.Trim().Length -ge 3 } | ForEach-Object { $_.Trim() } | Sort-Object -Unique | Sort-Object Length -Descending)
}

$script:RedactPatterns = @(
  @('(?i)\b[A-Z]:(?:\\\\|\\|/)[^\s"''<>|,;)\]]*', '<path>'),
  @('(?<![A-Za-z0-9])\\{2,4}[A-Za-z0-9._$-]+(?:\\{1,2}[^\s"''<>|,;)\]]*)?', '<path>'),
  @('(?<![\w.])/(?:home|Users|root|tmp|var|mnt|private)/[^\s"''<>|,;)\]]*', '<path>'),
  @('(?i)[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}', '<email>'),
  @('(?i)\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b', '<id>'),
  @('(?i)\b(?:[0-9a-f]{2}[:-]){5}[0-9a-f]{2}\b', '<mac>'),
  @('(?<![\d.])(?:(?:25[0-5]|2[0-4]\d|1?\d?\d)\.){3}(?:25[0-5]|2[0-4]\d|1?\d?\d)(?![\d.])', '<ip>'),
  @('(?i)\b(?:[0-9a-f]{1,4}:){5,7}[0-9a-f]{1,4}\b', '<ip>'),
  @('(?i)(?<![\w:])(?=[0-9a-f:]*::)[0-9a-f:]{3,39}(?![\w:])', '<ip>')
)

function Protect-Text([string]$Text, [string[]]$Tokens) {
  $s = $Text
  foreach ($p in $script:RedactPatterns) { $s = [regex]::Replace($s, $p[0], $p[1]) }
  foreach ($tok in $Tokens) { $s = [regex]::Replace($s, '(?i)(?<![A-Za-z0-9])' + [regex]::Escape($tok) + '(?![A-Za-z0-9])', '<redacted>') }
  return $s
}
function Find-Leftover([string]$Text, [string[]]$Tokens) {
  foreach ($p in $script:RedactPatterns) { if ([regex]::IsMatch($Text, $p[0])) { return "pattern $($p[1])" } }
  foreach ($tok in $Tokens) { if ([regex]::IsMatch($Text, '(?i)(?<![A-Za-z0-9])' + [regex]::Escape($tok) + '(?![A-Za-z0-9])')) { return 'a name' } }
  return $null
}

function Test-BinaryOrImage([byte[]]$Bytes) {
  if ($Bytes.Length -ge 4) {
    $h = ($Bytes[0..3] | ForEach-Object { $_.ToString('X2') }) -join ''
    foreach ($m in '89504E47', 'FFD8FFE0', 'FFD8FFE1', 'FFD8FFDB', 'FFD8FFEE', '47494638', '52494646', '49492A00', '4D4D002A', '00000100') { if ($h -eq $m) { return $true } }
    if ($h.StartsWith('424D')) { return $true }
  }
  $n = [Math]::Min($Bytes.Length, 65536)
  if ($n -eq 0) { return $false }
  return ([Array]::IndexOf($Bytes, [byte]0, 0, $n) -ge 0)
}

# Pack the results folder into the zip: allowed text reports only, each redacted and
# checked again. Returns @{ zip; included; excluded }.
function New-ResultsZip([string]$ResultsDir, [string]$ZipPath, [string]$TopFolder) {
  $tokens = Get-RedactionTokens
  $allowed = @('.txt', '.json', '.csv')
  $included = New-Object System.Collections.Generic.List[string]
  $excluded = New-Object System.Collections.Generic.List[string]
  $stage = Join-Path ([IO.Path]::GetTempPath()) ('hykit-zip-' + [guid]::NewGuid().ToString('N'))
  New-Item -ItemType Directory -Force -Path $stage | Out-Null
  try {
    $rootFull = [IO.Path]::GetFullPath($ResultsDir).TrimEnd('\', '/')
    foreach ($f in @(Get-ChildItem -LiteralPath $ResultsDir -Recurse -File -ErrorAction SilentlyContinue | Sort-Object FullName)) {
      $rel = $f.FullName.Substring($rootFull.Length + 1) -replace '\\', '/'
      $relSafe = Protect-Text $rel $tokens
      if ($relSafe -ne $rel) { $excluded.Add("$relSafe (its name held a name or path)"); continue }
      $ext = [IO.Path]::GetExtension($f.Name).ToLowerInvariant()
      if ($allowed -notcontains $ext) { $excluded.Add("$rel (not a text report: $ext)"); continue }
      if ($f.Length -gt 64MB) { $excluded.Add("$rel (over 64 MB)"); continue }
      $bytes = [IO.File]::ReadAllBytes($f.FullName)
      $isUtf16 = $bytes.Length -ge 2 -and (($bytes[0] -eq 0xFF -and $bytes[1] -eq 0xFE) -or ($bytes[0] -eq 0xFE -and $bytes[1] -eq 0xFF))
      if (-not $isUtf16 -and (Test-BinaryOrImage $bytes)) { $excluded.Add("$rel (binary or image content)"); continue }
      $text = if ($isUtf16) { if ($bytes[0] -eq 0xFF) { [Text.Encoding]::Unicode.GetString($bytes, 2, $bytes.Length - 2) } else { [Text.Encoding]::BigEndianUnicode.GetString($bytes, 2, $bytes.Length - 2) } } else { [Text.Encoding]::UTF8.GetString($bytes) }
      if ($text.Length -gt 0 -and $text[0] -eq [char]0xFEFF) { $text = $text.Substring(1) }
      $clean = Protect-Text $text $tokens
      $left = Find-Leftover $clean $tokens
      if ($left) { $excluded.Add("$rel (still held $left after redaction)"); continue }
      if ($ext -eq '.json' -and $clean.Trim()) {
        $valid = $false; try { $null = $clean | ConvertFrom-Json; $valid = $true } catch { }
        if (-not $valid) { $excluded.Add("$rel (no longer valid JSON after redaction)"); continue }
      }
      $dest = Join-Path $stage ($rel -replace '/', [IO.Path]::DirectorySeparatorChar)
      New-Item -ItemType Directory -Force -Path (Split-Path -Parent $dest) | Out-Null
      [IO.File]::WriteAllText($dest, $clean, (New-Object Text.UTF8Encoding($false)))
      $included.Add($rel)
    }
    if (Test-Path -LiteralPath $ZipPath) { Remove-Item -LiteralPath $ZipPath -Force }
    Add-Type -AssemblyName System.IO.Compression -ErrorAction SilentlyContinue
    Add-Type -AssemblyName System.IO.Compression.FileSystem -ErrorAction SilentlyContinue
    $zip = [IO.Compression.ZipFile]::Open($ZipPath, [IO.Compression.ZipArchiveMode]::Create)
    try {
      foreach ($rel in $included) {
        $src = Join-Path $stage ($rel -replace '/', [IO.Path]::DirectorySeparatorChar)
        $null = [IO.Compression.ZipFileExtensions]::CreateEntryFromFile($zip, $src, "$TopFolder/$rel")
      }
    }
    finally { $zip.Dispose() }
  }
  finally { Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue }
  return @{ zip = $ZipPath; included = $included.ToArray(); excluded = $excluded.ToArray() }
}

# ===================================================================== summary

function Write-Summary([object[]]$Steps) {
  $res = Join-Path $script:StateDirResolved 'results'
  New-Item -ItemType Directory -Force -Path $res | Out-Null
  $rows = New-Object System.Collections.Generic.List[object]
  $lines = New-Object System.Collections.Generic.List[string]
  $lines.Add("HealthierYou office test kit $KitVersion - $($script:PcLabel) ($script:RoleName)$(if ($script:RetestName) { " - $($script:RetestName.ToLowerInvariant()) retest" }) - summary written $(Get-NowText)")
  if ($script:Manifest) { $lines.Add("Kit build: commit $($script:Manifest.commit), CI run $($script:Manifest.run_id); installers are CI test builds (update keys made for that run only; no Authenticode signature)") }
  $e = $script:State.env
  if ($e.Count) { $lines.Add(("This PC: {0} build {1}; {2} GB memory; {3} logical processors; laptop {4}; webcams found {5}; pktmon {6}; account in Administrators {7}; kit window elevated {8}; PowerShell {9}" -f $e.windows, $e.build, $e.ram_gb, $e.cores, $e.laptop, $e.webcams, $e.pktmon, $e.admin_member, $e.elevated, $e.powershell)) }
  foreach ($n in @($script:State.data['row_notes'])) { if ($n) { $lines.Add("Note: $n") } }
  $lines.Add('')
  $lines.Add(('{0,-6} {1,-13} {2,-18} {3,-19} {4}' -f 'Step', 'Verdict', 'Rows', 'Finished', 'What / evidence'))
  $lines.Add(('-' * 100))
  foreach ($st in $Steps) {
    $s = $script:State.steps[$st.Id]
    $v = if ($s -and $s.status -eq 'done') { $s.verdict } else { 'NOT RUN' }
    $r = if ($s -and $s.rows) { (@($s.rows) -join ',') } else { '-' }
    $lines.Add(('{0,-6} {1,-13} {2,-18} {3,-19} {4}: {5}' -f $st.Id, $v, $r, $(if ($s -and $s.ended) { $s.ended } else { '' }), $st.Name, $(if ($s) { $s.detail } else { '' })))
    foreach ($sub in @($(if ($s) { $s.sub } else { @() }))) { if ($sub) { $lines.Add(('{0,-6} {1,-13} {2,-18} {3,-19}   {4}' -f '', $sub.verdict, $sub.row, '', $sub.detail)) } }
    $rows.Add(@{ id = $st.Id; name = $st.Name; verdict = $v; rows = $(if ($s) { @($s.rows) } else { @() }); detail = $(if ($s) { $s.detail } else { '' }); sub = $(if ($s) { @($s.sub) } else { @() }); started = $(if ($s) { $s.started } else { '' }); ended = $(if ($s) { $s.ended } else { '' }); attempts = $(if ($s) { $s.attempts } else { 0 }); needs_person = $st.Human; admin = [bool]$st.Admin })
  }
  $lines.Add('')
  $lines.Add('Verdicts come from each test script''s own report: PASS only with its evidence; INCONCLUSIVE means evidence is missing (the detail says what); SKIPPED means not run.')
  $lines.Add('Rows are the docs/V1_Acceptance.md ids this run covers. Filing them there is a separate step: nothing is marked passed by the kit.')
  $lines | Set-Content -LiteralPath (Join-Path $res 'kit-summary.txt') -Encoding UTF8
  Write-JsonFile (Join-Path $res 'kit-summary.json') @{ kit = $KitVersion; pc = $script:PcLabel; role = $script:RoleName; retest = [string]$script:RetestName; manifest = $script:Manifest; env = $script:State.env; row_notes = @($script:State.data['row_notes']); steps = $rows.ToArray() }
  return $lines
}

# ===================================================================== the steps

function Get-ResultsDir { Join-Path $script:StateDirResolved 'results' }
function Get-ResultPath([string]$Name) { Join-Path (Get-ResultsDir) $Name }

function Invoke-CheckStep([hashtable]$Ctx) {
  $e = Get-EnvInfo
  $script:State.env = $e
  $rows = Select-Rows $e $script:RoleName
  $script:State.data['rows'] = $rows
  $script:State.data['row_notes'] = @($rows.notes)
  $notes = @("Windows $(if ($e.windows11) { '11' } else { '10' }) build $($e.build), $($e.ram_gb) GB, webcams $($e.webcams)")
  $verdicts = @()
  $need = if ($script:RoleName -eq 'Server') { @('server', 'companion') } else { @('companion') }
  foreach ($k in $need) {
    $p = Get-Installer $k
    if (-not $p) { $notes += "$k installer missing from the kit"; $verdicts += 'INCONCLUSIVE'; continue }
    $h = Test-InstallerHash $p
    $notes += $h.detail
    $verdicts += $(if ($h.ok -eq $true) { 'PASS' } elseif ($h.ok -eq $false) { 'FAIL' } else { 'INCONCLUSIVE' })
  }
  if ($script:RetestName) { foreach ($k in $need) { if (-not (Get-KitBuildHash $k)) { $notes += "kit-manifest.json records no SHA-256 for the $k program, so the retest stops before replacing an installed one (it refuses to uninstall without the hash)" } } }
  if ($script:RoleName -eq 'Camera' -and -not $e.pktmon) { $notes += 'pktmon is missing (Windows 10 2004 or later is needed for the capture)' }
  Show-Banner 'This PC' @("Windows: $($e.windows) build $($e.build)", "Memory: $($e.ram_gb) GB; webcams found: $($e.webcams); laptop: $($e.laptop)", "Rows chosen: $((@($rows.capture) + @($rows.camera_row) + @($rows.soak) | Where-Object { $_ }) -join ', ')") 'Cyan'
  return New-Result (Join-Verdicts $verdicts) ($notes -join '; ')
}

# ---------------------------------------------------------------- PC1 (server)

# The mirror folder for a fresh server install: a second drive, else a USB stick or a share
# the person gives. '' when they skip.
function Select-MirrorFolder {
  while ($true) {
    $sys = if ($env:SystemDrive) { $env:SystemDrive.TrimEnd(':').ToUpperInvariant() } else { 'C' }
    $drive = $null
    if ($script:Sim) { $drive = 'D:' }
    else { $drive = @(Get-CimInstance Win32_LogicalDisk -ErrorAction SilentlyContinue | Where-Object { @(2, 3) -contains [int]$_.DriveType -and $_.DeviceID.TrimEnd(':').ToUpperInvariant() -ne $sys -and [double]$_.FreeSpace -gt 1GB } | Sort-Object DriveType -Descending | Select-Object -First 1 | ForEach-Object { $_.DeviceID }) }
    if ($drive) { Write-Log 'Mirror folder chosen on a second drive or share.'; return "$drive\HealthierYou-Mirror" }
    Show-Banner 'The server needs a second drive' @('The server keeps a copy of its control log (the mirror) on ANOTHER drive than C:.', 'No second drive was found. Plug in a USB stick (leave it plugged in from now on),', 'then press Enter. Or type a network share path (\\server\share\folder), or s to skip.')
    $a = Read-Answer '  Enter = look again, a \\share\path, or s' ''
    if ($a -match '^[sS]$') { return '' }
    if ($a -like '\\*') { Write-Log 'Mirror folder chosen on a second drive or share.'; return $a }
  }
}

function Invoke-ServerInstallStep([hashtable]$Ctx) {
  $mirror = $null
  $already = $false
  if (-not $script:Sim) { $already = [bool](Get-Service -Name 'HealthierYouServer' -ErrorAction SilentlyContinue) }
  if (-not $already) {
    $mirror = Select-MirrorFolder
    if (-not $mirror) { return New-Result 'SKIPPED' 'no second drive for the mirror' @('SV1') }
  }
  $r = Invoke-AdminJob 'server-install' @{ mirror = $mirror } 'server-install'
  $r.rows = @('SV1')
  $r.detail = "$($r.detail). SmartScreen is not seen in a silent install: SV1's warning check stays for a double-click install."
  # SV2 needs a Private or Domain network: the firewall rule does not open Public.
  if (-not $script:Sim -and $script:IsWin) {
    $pub = @(Get-NetConnectionProfile -ErrorAction SilentlyContinue | Where-Object { [string]$_.NetworkCategory -eq 'Public' })
    if ($pub.Count) {
      Show-Banner 'This network is set to Public' @('The server''s firewall rule opens Private and Domain networks only (by design), so PC2 cannot reach it.', 'Ask office IT, or set it in Settings > Network & internet > (your network) > Private network.')
      Wait-Enter 'Press Enter when the network is Private (or to carry on anyway)'
    }
  }
  return $r
}

function Invoke-PassphraseStep([hashtable]$Ctx) {
  if (-not $script:AgentInline) { Show-Banner 'Look at the ADMINISTRATOR window' @('It asks for the admin passphrase for the server and then shows the server''s six words.') 'Cyan' }
  $r = Invoke-AdminJob 'server-passphrase' @{} 'server-passphrase'
  if ($r.words) { $script:State.data['six_words'] = [string]$r.words }
  $r.Remove('words')
  return $r
}

function Invoke-CompanionSilentStep([hashtable]$Ctx) {
  $exe = Get-CompanionExe
  if (-not $script:Sim -and (Test-Path -LiteralPath $exe)) {
    $ok = Start-CompanionIfNeeded
    return New-Result $(if ($ok) { 'PASS' } else { 'FAIL' }) "already installed for this user; $(if ($ok) { 'running' } else { 'did not start' })"
  }
  $inst = Get-Installer 'companion'
  if (-not $inst) { return New-Result 'INCONCLUSIVE' 'the companion installer is not in the kit' }
  Write-Log 'Installing HealthierYou for this user (silent, no administrator)...'
  Write-Host '  If Windows asks "Do you want to run this file?", click Run.' -ForegroundColor Yellow
  if ($script:Sim) { return New-Result 'PASS' 'installed (simulated)' }
  $p = Start-Process -FilePath $inst -ArgumentList '/S' -PassThru
  $null = $p.Handle; $p.WaitForExit()
  if ($p.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $exe)) { return New-Result 'FAIL' "the installer did not install the companion (exit code $($p.ExitCode))" }
  $ok = Start-CompanionIfNeeded
  return New-Result $(if ($ok) { 'PASS' } else { 'FAIL' }) "installed per user (exit code 0); $(if ($ok) { 'running' } else { 'did not start' })"
}

function Get-RemoteServerPeers {
  # Remote ends of connections to TCP 8443 that are not this PC: counted, never written.
  $local = @{}
  try { foreach ($a in @(Get-NetIPAddress -ErrorAction Stop)) { $local[[string]$a.IPAddress] = 1 } } catch { }
  $peers = @()
  try { $peers = @(Get-NetTCPConnection -LocalPort 8443 -ErrorAction Stop | Where-Object { $_.State -ne 'Listen' -and $_.RemoteAddress -notmatch '^(127\.|::1$|0\.0\.0\.0|::$)' -and -not $local.ContainsKey([string]$_.RemoteAddress) } | ForEach-Object { [string]$_.RemoteAddress }) } catch { }
  return $peers
}

function Invoke-PairingStep([hashtable]$Ctx) {
  $since = (Get-Date).ToUniversalTime().ToString('o')
  $name = $env:COMPUTERNAME
  $problem = Get-ServerNameProblem $name
  if ($problem) {
    Show-Banner 'Pairing cannot start on this PC' @($problem) 'Red'
    return New-Result 'INCONCLUSIVE' $problem @('SV2', 'LAN-3')
  }
  $words = [string]$script:State.data['six_words']
  if (-not $script:Sim) { try { Start-Process 'http://127.0.0.1:8444' | Out-Null } catch { } }
  Show-Banner 'Pair PC2 with the server (you, on both PCs)' @(
    "SERVER NAME - PC2 asks for it:   $name",
    "SIX WORDS   - PC2 must show:     $(if ($words) { $words.ToUpperInvariant() } else { '(see the ADMINISTRATOR window, or run: hy-server fingerprint)' })",
    '',
    '1. In the browser page that just opened (http://127.0.0.1:8444), sign in with the passphrase.',
    '2. Add a user named  kit-pc2  (never a real name). Click New slip. Leave the slip on screen.',
    "3. Go to PC2 and start $(Get-Launcher 'Camera') there. Its network capture step asks, in order,",
    '   to pair (phase "Pending": paste the slip there; send it by internal chat or a USB stick, or',
    '   type it) and then to have it confirmed (phase "PairedOff"):',
    '4. To confirm: on PC2 read the six words and the check code; compare them with this admin',
    '   page (the words above, the code on the pending row); if both match, click Confirm.',
    '5. Optional, for the soak: pair THIS PC''s HealthierYou too (user kit-pc1): tray icon >',
    '   Settings > Sharing & server > Pair with the office server; then Confirm it here.',
    '6. When PC2''s kit says the capture step is finished, come back here and press Enter.')
  $seen = @{}
  if ($script:Unattended) { Wait-Enter 'Press Enter when PC2''s capture step is finished' }
  else {
    Write-Host '  Waiting. Press Enter when PC2''s capture step is finished.' -ForegroundColor Yellow
    $keyOk = $true
    while ($true) {
      foreach ($p in (Get-RemoteServerPeers)) { $seen[$p] = 1 }
      try { if ([Console]::KeyAvailable) { $k = [Console]::ReadKey($true); if ($k.Key -eq 'Enter') { break } } } catch { $keyOk = $false }
      if (-not $keyOk) { $null = Read-Host '  Press Enter'; break }
      Start-Sleep -Milliseconds 1500
    }
  }
  $ev = Invoke-AdminJob 'server-pairing-evidence' @{ since = $since } 'pairing-evidence'
  $remote = $seen.Count
  $prof = 'not read'
  if (-not $script:Sim -and $script:IsWin) { try { $prof = (@(Get-NetConnectionProfile -ErrorAction Stop | ForEach-Object { [string]$_.NetworkCategory }) | Sort-Object -Unique) -join ',' } catch { } }
  $okProfile = ($script:Sim -or ($prof -ne 'not read' -and $prof -notmatch 'Public'))
  $v = if ($ev.verdict -eq 'PASS' -and ($remote -ge 1 -or $script:Sim) -and $okProfile) { 'PASS' } else { 'INCONCLUSIVE' }
  $d = "$($ev.detail); connections from another PC to TCP 8443 seen while waiting: $(if ($script:Sim) { 'simulated' } else { $remote }); network profile: $prof"
  if ($v -ne 'PASS') { $d += '. Missing for PASS: a pairing and a confirmed request in the server log, a connection from PC2, and a Private or Domain network.' }
  return New-Result $v $d @('SV2', 'LAN-3')
}

function Invoke-ServerSoakStep([hashtable]$Ctx) {
  $hours = if ($script:Unattended) { '0.01' } else { '8' }
  $running = $script:Sim -or [bool](Get-Service -Name 'HealthierYouServer' -ErrorAction SilentlyContinue | Where-Object { $_.Status -eq 'Running' })
  if (-not $running) { return New-Result 'INCONCLUSIVE' 'the HealthierYouServer service is not running, so the soaks did not start' @('S2', 'S1') }
  Show-Banner '8-hour soaks: server (S2) and this PC''s companion (S1)' @('Nothing to do: leave this PC on, this window and the ADMINISTRATOR window open.', 'This PC is kept awake while they run. Locking it is fine. Work on PC2 meanwhile.', 'They need 8 hours of awake time (at most about 16.5 hours in all).')
  if (-not (Read-YesNo 'Start the 8-hour soaks now?' $true)) { return New-Result 'SKIPPED' 'not started (run the kit again to start them)' @('S2', 'S1') }
  $companionReport = Get-ResultPath 'hy-soak-companion.txt'
  Start-KeepAwake
  try {
    $job = Invoke-AdminJob 'soak-server' @{ hours = $hours } 'soak-server' -NoWait
    if ($job -is [hashtable]) { $s2 = $job; $job = $null }
    $run = Invoke-ReportedScript 'Test-Soak.ps1' @('-Role', 'Companion', '-Hours', $hours, '-Report', $companionReport) @($companionReport, "$companionReport.json")
    $s1 = Limit-Verdict (Get-SoakVerdict ($companionReport + '.json') $run.code) $run
    if ($job) { Write-Log 'Companion soak finished; waiting for the server soak.'; $s2 = Wait-AdminJob $job 'soak-server' }
  }
  finally { Stop-KeepAwake }
  $sub = @(@{ row = 'S2'; verdict = $s2.verdict; detail = "server soak: $($s2.detail)" }, @{ row = 'S1'; verdict = $s1.verdict; detail = "companion soak on the server PC: $($s1.detail)" })
  return New-Result (Join-Verdicts @($s2.verdict, $s1.verdict)) 'reports hy-soak-server.txt and hy-soak-companion.txt' @('S2', 'S1') $sub
}

# ---------------------------------------------------------------- PC2 (camera)

function Invoke-GuidedInstallStep([hashtable]$Ctx) {
  $inst = Get-Installer 'companion'
  if (-not $inst) { return New-Result 'INCONCLUSIVE' 'the companion installer is not in the kit' @('I1', 'I2', 'I3', 'I4', 'A16') }
  $report = Get-ResultPath 'hy-office-install.txt'
  if (-not $script:Sim -and (Test-Path -LiteralPath (Get-CompanionExe))) {
    Show-Banner 'HealthierYou is already installed for this user' @('The guided install checks need a first install. Uninstall it from Settings > Apps first', 'to run them, or carry on without them.')
    if (-not (Read-YesNo 'Skip the guided install checks and carry on?' $true)) { return New-Result 'INCONCLUSIVE' 'uninstall it, then run the kit again' @('I1') }
    return New-Result 'SKIPPED' 'already installed: the install checks did not run' @('I1', 'I2', 'I3', 'I4', 'I6')
  }
  Show-Banner 'Install HealthierYou (guided, about 20 minutes)' @('A File Explorer window opens with the installer selected. The script below tells you', 'when to double-click it and asks yes/no questions about what Windows shows.', 'These are CI test builds without a code signature: Windows may warn "Unknown publisher".', 'That warning is what this step records: click More info > Run anyway, never turn a protection off.', 'It also asks you to change the display scale (Settings > System > Display) a few times.')
  if (-not $script:Sim) { try { Start-Process explorer.exe -ArgumentList "/select,`"$inst`"" | Out-Null } catch { } }
  $a = @('-Phase', 'Install', '-Installer', $inst, '-Report', $report)
  $newer = Get-Installer 'companion-newer'
  if ($newer) { $a += @('-NewerInstaller', $newer) }
  $run = Invoke-ReportedScript 'Test-HealthierYou-OfficePC.ps1' $a @($report)
  $v = Limit-Verdict (Get-GuidedVerdict $report) $run
  $null = Start-CompanionIfNeeded
  $rows = @('I1', 'I2', 'I3', 'I4', 'A16'); if ($newer) { $rows += 'I6' }
  return New-Result $v.verdict "$($v.detail) (exit code $($run.code))" $rows
}

function Invoke-SignInStep([hashtable]$Ctx) {
  $d = $script:State.data
  if (-not $d['signout_asked']) {
    Show-Banner 'Sign out and sign in again (start at sign-in, I5)' @('1. Save your work. Sign out of Windows: Start > your name > Sign out.', '2. Sign in again. Wait for the pet to appear.', '3. This kit starts again by itself (or double-click Run-PC2-Camera.cmd).', 'Type s to skip this check.')
    $a = Read-Answer '  Press Enter, then sign out (or s to skip)' ''
    if ($a -match '^[sS]$') { return New-Result 'SKIPPED' 'skipped' @('I5') }
    $d['signout_asked'] = $true
    Set-AutoResume $true
    return @{ verdict = 'PAUSE'; detail = 'Sign out of Windows now. The kit carries on after you sign in again.' }
  }
  $report = Get-ResultPath 'hy-office-aftersignin.txt'
  $run = Invoke-ReportedScript 'Test-HealthierYou-OfficePC.ps1' @('-Phase', 'AfterSignIn', '-Report', $report) @($report)
  $v = Limit-Verdict (Get-GuidedVerdict $report) $run
  $d.Remove('signout_asked')
  return New-Result $v.verdict "$($v.detail) (exit code $($run.code))" @('I5')
}

function Invoke-CaptureStep([hashtable]$Ctx) {
  $rows = @($script:State.data['rows'].capture)
  if (-not $rows.Count) { $rows = @('S3') }
  $e = $script:State.env
  if (-not $e.pktmon -and -not $script:Sim) { return New-Result 'INCONCLUSIVE' 'pktmon is not on this PC (Windows 10 version 2004 or later is needed)' $rows }
  $d = $script:State.data
  if (-not $d['canary']) {
    $chars = 'ABCDEFGHJKMNPQRSTVWXYZ23456789'.ToCharArray()
    $rnd = New-Object Random
    $d['canary'] = 'KITPC2-' + (-join (1..4 | ForEach-Object { $chars[$rnd.Next($chars.Length)] }))
    Save-State
  }
  # A retest takes the server name the first run was given, when it is still usable.
  if ($script:RetestName -and -not $d['server_name']) {
    $first = Read-JsonFile (Join-Path (Join-Path (Split-Path -Parent $script:StateDirResolved) $script:RoleName) 'state.json')
    $n = if ($first -and $first.data) { [string]$first.data['server_name'] } else { '' }
    if ($n -and (Test-ServerName $n)) {
      $ok = $script:Sim
      if (-not $ok) { try { $ok = @([Net.Dns]::GetHostAddresses($n)).Count -gt 0 } catch { $ok = $false } }
      if ($ok) { $d['server_name'] = $n; Save-State; Write-Log 'Using the server name given in the first run.' }
    }
  }
  while (-not $d['server_name']) {
    Show-Banner 'The server''s name' @('PC1''s kit shows "SERVER NAME" on its pairing screen. Type it here.')
    $n = Read-Answer '  Server name (s to skip the capture)' $(if ($script:Unattended) { 'kit-server' } else { '' })
    if ($n -match '^[sS]$') { return New-Result 'SKIPPED' 'skipped' $rows }
    if (-not (Test-ServerName $n)) { Write-Host "  $($script:ServerNameRule)" -ForegroundColor Red; continue }
    $resolves = $script:Sim
    if (-not $resolves) { try { $resolves = @([Net.Dns]::GetHostAddresses($n)).Count -gt 0 } catch { $resolves = $false } }
    if (-not $resolves) { Write-Host "  '$n' is not found on the network from this PC. Check the name (and that both PCs are on the office network)." -ForegroundColor Red; continue }
    $d['server_name'] = $n; Save-State
  }
  if (-not (Start-CompanionIfNeeded)) { return New-Result 'INCONCLUSIVE' 'HealthierYou is not installed or did not start' $rows }
  Show-Banner 'Network capture with the pairing (about 30 minutes, with PC1)' @(
    'This runs in the ADMINISTRATOR window. Look there: it guides five phases.',
    "When HealthierYou asks for a PC name while pairing, type:   $($d['canary'])",
    'Phase "Pending": paste the slip from PC1 (Settings > Sharing & server > Pair), do NOT confirm yet.',
    'Phase "PairedOff": confirm on PC1 after comparing the six words and the check code.',
    'When it says "Report written", come back here.')
  $r = Invoke-AdminJob 'network-capture' @{ server = $d['server_name']; canary = $d['canary'] } 'network-capture'
  Show-Banner 'Capture step finished' @('Tell PC1: press Enter on its pairing screen now.', 'The raw capture files stay in an administrators-only folder on this PC and are never put in the zip.') 'Green'
  $r.rows = $rows
  return $r
}

function Invoke-CameraSoakStep([hashtable]$Ctx, [string]$Kind, [string[]]$Rows, [string]$ReportName) {
  if (-not $Kind) { return New-Result 'SKIPPED' 'no webcam found on this PC' $Rows }
  $report = Get-ResultPath $ReportName
  Show-Banner "2-hour camera soak ($($Rows -join ', '); webcam: $Kind)" @(
    'You sit at this desk and work as usual for about 2 to 2.5 hours, in view of the webcam.',
    '1. The script asks you to quit HealthierYou from its tray icon (right-click > Quit); it starts it again.',
    '2. Turn the posture check on when asked (pet menu > Posture check > Turn on camera; allow it).',
    '3. Lock the PC at least once for a minute (Windows+L), and if you can, have one call or slide show.',
    '4. At the end it asks you to turn the camera off and whether the webcam light went out.',
    'This PC is kept awake while it runs.')
  if (-not (Read-YesNo 'Start the camera soak now?' $true)) { return New-Result 'SKIPPED' 'not started (run the kit again to do it)' $Rows }
  Start-KeepAwake
  try {
    $hours = if ($script:Unattended) { '0.01' } else { '2' }
    $run = Invoke-ReportedScript 'Test-CameraSoak.ps1' @('-Hours', $hours, '-Webcam', $Kind, '-Report', $report) @($report, "$report.json")
  }
  finally { Stop-KeepAwake }
  $v = Limit-Verdict (Get-ExitVerdict $run.code ($report + '.json') 'verdict') $run
  return New-Result $v.verdict "$($v.detail); report $ReportName" $Rows
}

function Invoke-FirstCameraSoakStep([hashtable]$Ctx) {
  $rows = $script:State.data['rows']
  $e = $script:State.env
  if ([int]$e.webcams -le 0 -and -not $script:Sim) {
    Show-Banner 'No webcam found' @('Plug in a USB webcam now if you have one, then press Enter. Type s to skip the camera tests.')
    $a = Read-Answer '  Enter = look again, s = skip' ''
    if ($a -notmatch '^[sS]$') {
      $e.webcams = Get-WebcamCount
      $rows = Select-Rows $e 'Camera'
      $script:State.data['rows'] = $rows; $script:State.data['row_notes'] = @($rows.notes); Save-State
    }
  }
  $name = switch ($rows.camera_kind) { 'usb' { 'hy-camera-soak-usb.txt' } default { if ($e.windows11) { 'hy-camera-soak-w11.txt' } else { 'hy-camera-soak-w10.txt' } } }
  return Invoke-CameraSoakStep $Ctx ([string]$rows.camera_kind) @($rows.camera_row) $name
}

function Invoke-SecondCameraSoakStep([hashtable]$Ctx) {
  $rows = $script:State.data['rows']
  if (-not $rows.second_webcam) { return New-Result 'SKIPPED' 'only one webcam found (CS3 needs a USB webcam as the one in use)' @('CS3') }
  Show-Banner 'A second webcam was found: CS3 (2 more hours)' @('HealthierYou uses Windows'' default camera. To make the USB webcam the one in use,', 'disable the built-in one for now (Device Manager > Cameras > right-click > Disable device),', 'or close the laptop lid if you use an external screen. Turn it back on afterwards.')
  if (-not (Read-YesNo 'Run the 2-hour soak with the USB webcam now?' $false)) { return New-Result 'SKIPPED' 'not run now (offered)' @('CS3') }
  return Invoke-CameraSoakStep $Ctx 'usb' @('CS3') 'hy-camera-soak-usb.txt'
}

# One desk session. It counts as done only with this run's own evidence: exit code 0, a
# fresh report whose session line passed, and the fresh raw-count JSON HealthierYou wrote.
function Invoke-DeskSession([string]$DeskDir, [string]$SetupNo, [string]$SessionNo) {
  $base = Join-Path $DeskDir "hy-desk-$SetupNo-$SessionNo"
  $txt = "$base.txt"; $json = "$base.json"
  foreach ($old in "$base.csv", "$base.labels.csv", "$base.session.txt") { Remove-Item -LiteralPath $old -Force -ErrorAction SilentlyContinue }
  $run = Invoke-ReportedScript 'Test-HealthierYou-Camera.ps1' @('-Phase', 'Desk', '-Setup', $SetupNo, '-Session', $SessionNo, '-Report', $txt) @($txt, $json)
  $why = $null
  if ($run.code -ne 0) { $why = "the session script exited with code $($run.code)" }
  elseif (-not $run.fresh) { $why = "this run wrote no $(@($run.missing) -join ' or ')" }
  elseif (-not @(Get-Content -LiteralPath $txt | Where-Object { $_ -like '`[PASS`] session-report*' }).Count) { $why = 'HealthierYou gave no session counts' }
  if ($why) {
    # A failed session's files must not reach the gate.
    foreach ($old in $json, "$base.session.txt") { Remove-Item -LiteralPath $old -Force -ErrorAction SilentlyContinue }
    return @{ ok = $false; why = $why }
  }
  return @{ ok = $true; why = '' }
}

function Invoke-DeskStep([hashtable]$Ctx) {
  $rows = $script:State.data['rows']
  if (-not $rows.camera_kind) { return New-Result 'SKIPPED' 'no webcam found on this PC' @('V2-3') }
  $d = $script:State.data
  $done = @($d['desk_done'] | Where-Object { $_ })
  $deskDir = Join-Path (Get-ResultsDir) 'desk'
  New-Item -ItemType Directory -Force -Path $deskDir | Out-Null
  $all = @(); foreach ($s in 1..5) { foreach ($n in 1..2) { $all += "$s-$n" } }
  $deskFails = 0
  while ($true) {
    $next = @($all | Where-Object { $done -notcontains $_ } | Select-Object -First 1)
    if (-not $next.Count) { break }
    $parts = $next[0] -split '-'
    $lines = @("Done so far: $($done.Count) of 10 sessions (5 setups x 2). Each session is about 45 minutes.", 'The script tells you what to do and beeps at the end of each step (sit, read, type, lean, step away...).')
    if ($parts[1] -eq '1' -and $done.Count) { $lines += "Setup $($parts[0]) is a NEW setup: change something (camera position, light, glasses, desk, another PC's webcam)." }
    $lines += 'y = start the next session now; l = later (the kit stops; it carries on here next time); d = done with desk sessions (make the report)'
    Show-Banner "Desk session: setup $($parts[0]), session $($parts[1])" $lines
    $a = Read-Answer '  y / l / d' $(if ($script:Unattended -and ($done.Count -ge 2 -or $deskFails -ge 3)) { 'd' } else { 'y' })
    if ($a -match '^[lL]') { return @{ verdict = 'PAUSE'; detail = "desk sessions paused after $($done.Count) of 10" } }
    if ($a -match '^[dD]') { break }
    if (-not (Start-CompanionIfNeeded)) { return New-Result 'INCONCLUSIVE' 'HealthierYou did not start' @('V2-3') }
    $ok = Invoke-DeskSession $deskDir $parts[0] $parts[1]
    if ($ok.ok) {
      $done += $next[0]
      $d['desk_done'] = @($done)
      Save-State
    }
    else {
      Show-Banner "Session $($next[0]) was not recorded" @($ok.why, 'It stays to do: you can run it again now (y), later (l), or stop the desk sessions (d).') 'Red'
      Write-Log "Desk session $($next[0]) not recorded: $($ok.why)" 'Red'
      $deskFails++
    }
  }
  if (-not $done.Count) { return New-Result 'SKIPPED' 'no desk session run' @('V2-3') }
  $gate = Join-Path $deskDir 'hy-desk-gate.txt'
  $gateJson = Join-Path $deskDir 'hy-desk-gate.json'
  $run = Invoke-ReportedScript 'Test-HealthierYou-Camera.ps1' @('-Phase', 'DeskReport', '-Report', $gate) @($gate, $gateJson)
  $v = Limit-Verdict (Get-GateVerdict $gateJson) $run
  return New-Result $v.verdict "$($done.Count) session(s) recorded; $($v.detail) (exit code $($run.code))" @('V2-3')
}

function Invoke-OvernightSoakStep([hashtable]$Ctx) {
  $rows = $script:State.data['rows']
  if (-not $rows.companion_soak) { return New-Result 'SKIPPED' 'S1 runs on PC1 (this PC is not the Windows 10 or small-memory PC the soak prefers)' @('S1') }
  Show-Banner 'Overnight companion soak (S1), 8 hours' @('This is the PC docs/V1_Acceptance.md prefers for S1 (Windows 10 or about 4 GB).', 'Leave it on overnight with this window open; nothing to do. It is kept awake.', 'Open your usual office apps first if you can (the soak should see a normal PC).')
  if (-not (Read-YesNo 'Start the 8-hour companion soak now?' $true)) { return New-Result 'SKIPPED' 'not started' @('S1') }
  $report = Get-ResultPath 'hy-soak-companion-pc2.txt'
  Start-KeepAwake
  try { $run = Invoke-ReportedScript 'Test-Soak.ps1' @('-Role', 'Companion', '-Hours', $(if ($script:Unattended) { '0.01' } else { '8' }), '-Report', $report) @($report, "$report.json") }
  finally { Stop-KeepAwake }
  $v = Limit-Verdict (Get-SoakVerdict ($report + '.json') $run.code) $run
  return New-Result $v.verdict $v.detail @('S1', 'A7')
}

function Test-CacheLeft { if ($script:Sim) { return [bool]$script:Sim.cache_left }; return (Test-Path -LiteralPath (Join-Path $env:LOCALAPPDATA 'app.healthieryou.companion')) }

function Invoke-UninstallStep([hashtable]$Ctx) {
  $d = $script:State.data
  if ($d['i9b_recheck']) {
    $left = Test-CacheLeft
    $d.Remove('i9b_recheck')
    $prev = [string]$d['i9b_detail']
    return New-Result 'FAIL' "$prev; after a sign-out and sign-in the folder was $(if ($left) { 'STILL there' } else { 'gone' }) (file it against I9a)" @('I7', 'I9b')
  }
  Show-Banner 'Uninstall check (I7, I9b), about 10 minutes' @('First use HealthierYou a little if you have not today (open Settings from the tray, answer a reminder).', 'The script then asks you to uninstall it from Windows Settings > Apps > Installed apps,', 'leaving "Also delete my data" UNTICKED. Do it signed in as you, not as an administrator.')
  if (-not (Read-YesNo 'Do the uninstall check now?' $true)) { return New-Result 'SKIPPED' 'not run' @('I7', 'I9b') }
  $report = Get-ResultPath 'hy-office-uninstall.txt'
  $run = Invoke-ReportedScript 'Test-HealthierYou-OfficePC.ps1' @('-Phase', 'Uninstall', '-Report', $report) @($report)
  $code = $run.code
  $i7 = Limit-Verdict (Get-GuidedVerdict $report) $run
  # I9b: wait for HealthierYou's WebView2 processes to end, then look for the cache folder.
  $waited = 0
  if (-not $script:Sim) {
    $root = Join-Path $env:LOCALAPPDATA 'HealthierYou'
    while ($waited -lt 120) {
      $wv = @(Get-CimInstance Win32_Process -Filter "Name = 'msedgewebview2.exe'" -ErrorAction SilentlyContinue | Where-Object { ($_.ExecutablePath -and $_.ExecutablePath.StartsWith($root, [StringComparison]::OrdinalIgnoreCase)) -or ($_.CommandLine -match 'app\.healthieryou\.companion') })
      if (-not $wv.Count) { break }
      Start-Sleep -Seconds 2; $waited += 2
    }
  }
  $left = $true
  for ($i = 0; $i -lt 30; $i++) { $left = Test-CacheLeft; if (-not $left) { break }; if (-not $script:Sim) { Start-Sleep -Seconds 2 } }
  $dataKept = $script:Sim -or (Test-Path -LiteralPath (Join-Path (Join-Path $env:LOCALAPPDATA 'HealthierYou') 'data'))
  $uac = Read-YesNo 'Did Windows ask for an administrator password or show a UAC "Yes" prompt during the uninstall?' $false
  $member = $script:State.env.admin_member
  $detail = "I7: $($i7.detail); WebView2 processes gone after $waited s; cache folder app.healthieryou.companion $(if ($left) { 'LEFT' } else { 'gone' }); data folder $(if ($dataKept) { 'kept' } else { 'missing' }); UAC prompt: $(if ($uac) { 'yes' } else { 'no' }); account in Administrators: $member"
  $i9 = if ($left -or $uac -or -not $dataKept) { 'FAIL' } elseif ($member -eq $false) { 'PASS' } else { 'INCONCLUSIVE' }
  if ($i9 -eq 'INCONCLUSIVE') { $detail += ' (I9b asks for a standard, non-administrator account)' }
  if ($left -and -not $uac) {
    $d['i9b_recheck'] = $true; $d['i9b_detail'] = $detail
    Set-AutoResume $true
    Show-Banner 'The WebView2 cache folder was left' @('I9b then asks whether it goes after a sign-out and sign-in.', 'Sign out of Windows now and sign in again: the kit carries on by itself.')
    Save-State
    return @{ verdict = 'PAUSE'; detail = 'sign out and in for the I9b re-check' }
  }
  $sub = @(@{ row = 'I7'; verdict = $i7.verdict; detail = $i7.detail }, @{ row = 'I9b'; verdict = $i9; detail = 'cache folder, data folder, UAC and account type as above' })
  return New-Result (Join-Verdicts @($i7.verdict, $i9)) "$detail (exit code $code)" @('I7', 'I9b') $sub
}

# ---------------------------------------------------------------- pairing retest

# R1-02 (PC1): the office server replaced by the kit's build when it is not that build, in
# the administrator window (Invoke-ServerReplace). The six words come back to this window.
function Invoke-ServerReplaceStep([hashtable]$Ctx) {
  $kit = Get-KitBuildHash 'server'
  if (-not $kit) { return New-Result 'INCONCLUSIVE' 'not replaced: the kit records no valid hy-server.exe SHA-256 (kit-manifest.json), so the server was not touched. Download the kit again.' @('SV1') }
  $installed = if ($script:Sim) { [bool]($script:SimServer -and $script:SimServer.exe) } else { [bool](Get-Service -Name 'HealthierYouServer' -ErrorAction SilentlyContinue) }
  $p = @{}
  if (-not $installed) {
    # Only a fresh install needs a mirror; kept data brings its own.
    $m = Select-MirrorFolder
    if (-not $m) { return New-Result 'SKIPPED' 'no server installed and no second drive for the mirror' @('SV1') }
    $p.mirror = $m
  }
  $p.exe_sha256 = $kit
  $r = Invoke-AdminJob 'server-replace' $p 'server-replace'
  if ($r.words) { $script:State.data['six_words'] = [string]$r.words }
  $script:State.data['passphrase_needed'] = ($r.passphrase_needed -eq $true)
  foreach ($k in 'words', 'passphrase_needed', 'data_kept') { $r.Remove($k) }
  return $r
}

# R1-03 (PC1): the admin passphrase, only when the server's data did not survive (or when
# run again with -Redo R1-03, for a forgotten passphrase).
function Invoke-RetestPassphraseStep([hashtable]$Ctx) {
  if (-not $script:State.data['passphrase_needed'] -and -not $Ctx.redo) {
    return New-Result 'SKIPPED' "not needed: the server's data (its keys, and the database that holds the admin passphrase) was kept, so the passphrase from the first run still works. Forgotten? Run $(Get-Launcher 'Server') -Redo R1-03"
  }
  return (Invoke-PassphraseStep $Ctx)
}

function Test-CompanionData {
  if ($script:Sim) { return [bool]$script:Sim.companion_data }
  return (Test-Path -LiteralPath (Join-Path (Join-Path $env:LOCALAPPDATA 'HealthierYou') 'data'))
}
function Get-InstalledCompanionHash {
  if ($script:Sim) { return [string]$script:Sim.companion_exe }
  return (Get-FileSha256OrEmpty (Get-CompanionExe))
}
function Stop-CompanionForReplace {
  if ($script:Sim) { return }
  Get-Process -Name 'hy-companion', 'hy-update-helper' -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
  # The WebView2 processes of the bundled runtime end shortly after the app.
  $root = Join-Path $env:LOCALAPPDATA 'HealthierYou'
  $deadline = (Get-Date).AddSeconds(30)
  while ((Get-Date) -lt $deadline) {
    $left = @(Get-CimInstance Win32_Process -Filter "Name = 'msedgewebview2.exe'" -ErrorAction SilentlyContinue | Where-Object { $_.ExecutablePath -and $_.ExecutablePath.StartsWith($root, [StringComparison]::OrdinalIgnoreCase) })
    if (-not $left.Count) { return }
    Start-Sleep -Milliseconds 500
  }
}
# HealthierYou's own uninstaller, silent and without /DELETEDATA: the data folder stays
# (installer-hooks.nsh). It runs from a temporary copy; wait for it and the program to go.
function Invoke-CompanionUninstall {
  if ($script:Sim) {
    $script:Sim.calls += 'companion-uninstall'
    $script:Sim.companion_exe = ''
    if ($script:Sim.companion_uninstall_loses_data) { $script:Sim.companion_data = $false }
    return $null
  }
  $root = Join-Path $env:LOCALAPPDATA 'HealthierYou'
  $u = Join-Path $root 'uninstall.exe'
  if (-not (Test-Path -LiteralPath $u)) { return 'the installed HealthierYou has no uninstall.exe' }
  Write-Log 'Removing the installed HealthierYou, keeping its data (silent)...'
  $p = Start-Process -FilePath $u -ArgumentList '/S' -PassThru
  $null = $p.Handle
  if (-not $p.WaitForExit(600000)) { return 'the uninstaller did not finish within 10 minutes' }
  $deadline = (Get-Date).AddMinutes(3)
  while ((Get-Date) -lt $deadline) {
    if (-not (Test-Path -LiteralPath (Get-CompanionExe)) -and -not @(Get-Process -Name 'Au_' -ErrorAction SilentlyContinue).Count) { return $null }
    Start-Sleep -Milliseconds 500
  }
  return 'the old HealthierYou was not removed within 3 minutes'
}
function Invoke-CompanionSetup([string]$Installer) {
  if ($script:Sim) { $script:Sim.calls += 'companion-setup'; $script:Sim.companion_exe = [string]$script:Sim.companion_kit_exe; return 0 }
  Write-Log 'Installing HealthierYou for this user (silent, no administrator)...'
  $p = Start-Process -FilePath $Installer -ArgumentList '/S' -PassThru
  $null = $p.Handle; $p.WaitForExit()
  return [int]$p.ExitCode
}

# R1-04 (PC1) and R2-02 (PC2): HealthierYou for this user is the kit's build: installed
# when absent, kept when byte for byte the kit's, otherwise uninstalled keeping its data and
# installed from the kit (a same-version silent install over the old one is not relied on).
function Invoke-CompanionReplaceStep([hashtable]$Ctx) {
  $inst = Get-Installer 'companion'
  if (-not $inst) { return New-Result 'INCONCLUSIVE' 'the companion installer is not in the kit' }
  $hc = Test-InstallerHash $inst
  if ($hc.ok -ne $true) { return New-Result 'INCONCLUSIVE' "not installed: $($hc.detail)" }
  $kit = Get-KitBuildHash 'companion'
  if (-not $kit) { return New-Result 'INCONCLUSIVE' 'not replaced: the kit records no valid hy-companion.exe SHA-256 (kit-manifest.json), so HealthierYou was not touched. Download the kit again.' }
  $decision = Get-ReplaceDecision (Get-InstalledCompanionHash) $kit
  $notes = @()
  if ($decision -eq 'keep') { $notes += 'HealthierYou is already the kit''s build (hy-companion.exe SHA-256 matches); not installed again' }
  else {
    $busy = @(Get-RunningKitTests)
    if ($busy.Count) { return New-Result 'INCONCLUSIVE' "not replaced: $($busy -join ', ') is still running (a first-run soak or session). Start the retest again when it has finished." }
    Write-Host '  If Windows asks "Do you want to run this file?", click Run.' -ForegroundColor Yellow
    $dataBefore = Test-CompanionData
    if ($decision -eq 'replace') {
      $notes += "the installed HealthierYou is not the kit's build ($(if ($kit) { 'its hy-companion.exe SHA-256 differs' } else { 'the kit records no program SHA-256' })): uninstalled keeping its data, then installed from the kit"
      Stop-CompanionForReplace
      $why = Invoke-CompanionUninstall
      if ($why) { return New-Result 'FAIL' (($notes + "uninstall: $why") -join '; ') }
      if ($dataBefore -and -not (Test-CompanionData)) { return New-Result 'FAIL' (($notes + 'its data folder did not survive the uninstall, so nothing was installed') -join '; ') }
    }
    else { $notes += 'HealthierYou was not installed: installed from the kit (per user, silent)' }
    $code = Invoke-CompanionSetup $inst
    $notes += "setup exit code $code"
    if ($code -ne 0) { return New-Result 'FAIL' ($notes -join '; ') }
    if ($dataBefore) { $notes += "its data folder $(if (Test-CompanionData) { 'kept' } else { 'MISSING' })" }
  }
  $after = Get-InstalledCompanionHash
  $isKit = [bool]($kit -and $after -and ($after -ceq $kit))
  $notes += "installed hy-companion.exe $(if ($isKit) { 'is the kit''s build' } elseif (-not $after) { 'is MISSING' } elseif ($kit) { 'is NOT the kit''s build' } else { 'not compared (the kit records no program SHA-256)' })"
  $ok = Start-CompanionIfNeeded
  $notes += $(if ($ok) { 'running' } else { 'did not start' })
  $v = if (-not $after -or ($kit -and -not $isKit) -or -not $ok) { 'FAIL' } elseif (-not $kit) { 'INCONCLUSIVE' } else { 'PASS' }
  return New-Result $v ($notes -join '; ')
}

function Get-RetestSteps([string]$RoleName) {
  if ($RoleName -eq 'Server') {
    return @(
      @{ Id = 'R1-01'; Name = 'Check this PC and the kit'; Minutes = '1'; Human = 'no'; Admin = $false; Run = { param($c) Invoke-CheckStep $c } },
      @{ Id = 'R1-02'; Name = 'Office server: the kit''s build (replaced if needed, data kept)'; Minutes = '3'; Human = 'no'; Admin = $true; Why = 'replace the office server with this kit''s build, keeping its data'; Run = { param($c) Invoke-ServerReplaceStep $c } },
      @{ Id = 'R1-03'; Name = 'Admin passphrase (only if the server data was not kept)'; Minutes = '0-2'; Human = 'only if asked: type a passphrase'; Admin = $true; Why = 'set the admin passphrase, only if the server data was not kept'; Run = { param($c) Invoke-RetestPassphraseStep $c } },
      @{ Id = 'R1-04'; Name = 'HealthierYou on this PC: the kit''s build (per user, data kept)'; Minutes = '2'; Human = 'no'; Admin = $false; Run = { param($c) Invoke-CompanionReplaceStep $c } },
      @{ Id = 'R1-05'; Name = 'Pair PC2 (and this PC) with the server'; Minutes = '30-45 (with PC2)'; Human = 'yes: admin page, slip, compare six words, Confirm'; Admin = $true; Why = 'read the server log for pairing evidence'; Run = { param($c) Invoke-PairingStep $c } }
    )
  }
  return @(
    @{ Id = 'R2-01'; Name = 'Check this PC and the kit'; Minutes = '1'; Human = 'no'; Admin = $false; Run = { param($c) Invoke-CheckStep $c } },
    @{ Id = 'R2-02'; Name = 'HealthierYou: the kit''s build (installed or replaced, data kept)'; Minutes = '2'; Human = 'no'; Admin = $false; Run = { param($c) Invoke-CompanionReplaceStep $c } },
    @{ Id = 'R2-03'; Name = 'Network capture with the pairing'; Minutes = '30-40 (with PC1)'; Human = 'yes: paste the slip, read six words to the admin'; Admin = $true; Why = 'the network capture (pktmon and the connection audit)'; Run = { param($c) Invoke-CaptureStep $c } }
  )
}

function Get-Steps([string]$RoleName) {
  if ($script:RetestName) { return (Get-RetestSteps $RoleName) }
  if ($RoleName -eq 'Server') {
    return @(
      @{ Id = 'P1-01'; Name = 'Check this PC and the kit'; Minutes = '1'; Human = 'no'; Admin = $false; Run = { param($c) Invoke-CheckStep $c } },
      @{ Id = 'P1-02'; Name = 'Install the office server as a Windows service'; Minutes = '3'; Human = 'only if no second drive (plug in a USB stick)'; Admin = $true; Why = 'install the office server as a Windows service'; Run = { param($c) Invoke-ServerInstallStep $c } },
      @{ Id = 'P1-03'; Name = 'Admin passphrase and the server''s six words'; Minutes = '2'; Human = 'type a passphrase'; Admin = $true; Why = 'set the admin passphrase'; Run = { param($c) Invoke-PassphraseStep $c } },
      @{ Id = 'P1-04'; Name = 'Install HealthierYou on this PC (silent, per user)'; Minutes = '2'; Human = 'no'; Admin = $false; Run = { param($c) Invoke-CompanionSilentStep $c } },
      @{ Id = 'P1-05'; Name = 'Pair PC2 (and this PC) with the server'; Minutes = '30-45 (with PC2)'; Human = 'yes: admin page, slip, compare six words, Confirm'; Admin = $true; Why = 'read the server log for pairing evidence'; Run = { param($c) Invoke-PairingStep $c } },
      @{ Id = 'P1-06'; Name = '8-hour soaks: server (S2) and companion (S1)'; Minutes = '480-990 (unattended)'; Human = 'no (leave on)'; Admin = $true; Why = 'the 8-hour server soak'; Run = { param($c) Invoke-ServerSoakStep $c } }
    )
  }
  return @(
    @{ Id = 'P2-01'; Name = 'Check this PC (Windows, memory, webcams) and pick the rows'; Minutes = '1'; Human = 'no'; Admin = $false; Run = { param($c) Invoke-CheckStep $c } },
    @{ Id = 'P2-02'; Name = 'Install HealthierYou with the guided install checks'; Minutes = '20-30'; Human = 'yes: double-click, answer what Windows shows, change display scale'; Admin = $false; Run = { param($c) Invoke-GuidedInstallStep $c } },
    @{ Id = 'P2-03'; Name = 'Network capture with the pairing'; Minutes = '30-40 (with PC1)'; Human = 'yes: paste the slip, read six words to the admin'; Admin = $true; Why = 'the network capture (pktmon and the connection audit)'; Run = { param($c) Invoke-CaptureStep $c } },
    @{ Id = 'P2-04'; Name = 'Start at sign-in after a real sign-out and sign-in'; Minutes = '5'; Human = 'yes: sign out and in, sleep and wake'; Admin = $false; Run = { param($c) Invoke-SignInStep $c } },
    @{ Id = 'P2-05'; Name = '2-hour camera soak'; Minutes = '125-150'; Human = 'yes: sit and work at the desk, allow the camera, lock once'; Admin = $false; Run = { param($c) Invoke-FirstCameraSoakStep $c } },
    @{ Id = 'P2-06'; Name = '2-hour camera soak with the USB webcam (if found)'; Minutes = '0 or 125-150'; Human = 'yes, if run'; Admin = $false; Run = { param($c) Invoke-SecondCameraSoakStep $c } },
    @{ Id = 'P2-07'; Name = 'Desk sessions (posture quality gate)'; Minutes = '45 per session, up to 10'; Human = 'yes: follow the on-screen cues'; Admin = $false; Run = { param($c) Invoke-DeskStep $c } },
    @{ Id = 'P2-08'; Name = 'Overnight companion soak (Windows 10 / small-memory PC only)'; Minutes = '0 or 480+'; Human = 'no (leave on)'; Admin = $false; Run = { param($c) Invoke-OvernightSoakStep $c } },
    @{ Id = 'P2-09'; Name = 'Uninstall from Windows Settings (I7, I9b)'; Minutes = '10'; Human = 'yes: uninstall from Settings'; Admin = $false; Run = { param($c) Invoke-UninstallStep $c } }
  )
}

# ===================================================================== the run

# Run the steps in order. Done steps are skipped; a step left "running" (window closed,
# reboot) runs again. PAUSE keeps the step pending and stops the run.
# The answer to "run a step again": "Redo P1-06" and "-Redo P1-06" mean the same as "P1-06".
function ConvertFrom-StepAnswer([string]$Answer) { ($Answer.Trim() -replace '^(?i)-?redo\s+', '').ToUpperInvariant() }

function Invoke-Steps([object[]]$Steps, [string]$RedoId) {
  $n = $Steps.Count; $i = 0
  foreach ($st in $Steps) {
    $i++
    $s = $script:State.steps[$st.Id]
    if (-not ($s -is [hashtable])) { $s = @{ status = 'pending'; attempts = 0 }; $script:State.steps[$st.Id] = $s }
    if ($s.status -eq 'done' -and $RedoId -ne $st.Id) { continue }
    if ($s.status -eq 'running') { Write-Log "Step $($st.Id) did not finish last time (window closed, sign-out or restart): running it again." 'Yellow' }
    $s.status = 'running'; $s.started = Get-NowText; $s.attempts = [int]$s.attempts + 1
    Save-State
    Show-Banner "Step $i of $n ($($st.Id)): $($st.Name)" @("About $($st.Minutes) min. Needs you: $($st.Human)") 'Cyan'
    $r = $null
    $script:Ctx.redo = ($RedoId -eq $st.Id)
    try { $r = & $st.Run $script:Ctx }
    catch {
      if ($_.Exception.Message -like 'KIT-SIMULATED-CLOSE*') { throw }
      $r = New-Result 'INCONCLUSIVE' "the step stopped with an error: $($_.Exception.Message)"
    }
    if ($r -is [array]) { $r = @($r | Where-Object { $_ -is [hashtable] -and $_.ContainsKey('verdict') })[-1] }
    if (-not ($r -is [hashtable])) { $r = New-Result 'INCONCLUSIVE' 'the step returned no result' }
    if ($r.verdict -eq 'PAUSE') {
      $s.status = 'pending'; $s.detail = $r.detail; Save-State
      Show-Banner 'Paused' @($r.detail, 'Start the kit again to carry on (it starts by itself at your next sign-in).') 'Yellow'
      return 'paused'
    }
    $s.status = 'done'; $s.verdict = $r.verdict; $s.detail = [string]$r.detail; $s.rows = @($r.rows); $s.sub = @($r.sub); $s.ended = Get-NowText
    Save-State
    $color = switch ($r.verdict) { 'PASS' { 'Green' } 'FAIL' { 'Red' } default { 'Yellow' } }
    Write-Log "$($st.Id) $($r.verdict): $($r.detail)" $color
    if ($RedoId -eq $st.Id) { $RedoId = '' }
  }
  return 'complete'
}

# The zip's name: a retest's never takes a first run's.
function Get-ResultsZipBase {
  $b = "HealthierYou-results-$($script:PcLabel)"
  if ($script:RetestName) { $b += "-$($script:RetestName.ToLowerInvariant())-retest" }
  return $b
}
# A retest keeps its own state, results and log, apart from the first run's.
function Get-StateDirName([string]$RoleName) { if ($script:RetestName) { return "$RoleName-retest-$($script:RetestName.ToLowerInvariant())" } return $RoleName }
function Get-StateFileName { if ($script:RetestName) { return "state-retest-$($script:RetestName.ToLowerInvariant()).json" } return 'state.json' }
function Get-Launcher([string]$RoleName) {
  $pc = if ($RoleName -eq 'Server') { 'PC1' } else { 'PC2' }
  if ($script:RetestName) { return "Run-$pc-$($script:RetestName)-Retest.cmd" }
  if ($RoleName -eq 'Server') { return 'Run-PC1-Server.cmd' } return 'Run-PC2-Camera.cmd'
}

function Complete-Run([object[]]$Steps) {
  $lines = Write-Summary $Steps
  Write-Host ''
  foreach ($l in $lines) {
    $c = if ($l -match '^\S+\s+PASS\b') { 'Green' } elseif ($l -match '^\S+\s+FAIL\b') { 'Red' } elseif ($l -match '^\S+\s+(INCONCLUSIVE|NOT RUN)\b') { 'Yellow' } else { 'Gray' }
    Write-Host $l -ForegroundColor $c
  }
  $res = Get-ResultsDir
  try { Copy-Item -LiteralPath $script:LogFile -Destination (Join-Path $res 'kit-log.txt') -Force } catch { }
  $dir = $script:OutDirResolved
  if (-not $dir) { $dir = [Environment]::GetFolderPath('Desktop') }
  if (-not $dir) { $dir = $script:StateDirResolved }
  $name = "$(Get-ResultsZipBase)-$((Get-Date).ToString('yyyy-MM-dd'))"
  $zip = Join-Path $dir "$name.zip"
  $z = New-ResultsZip $res $zip $name
  Show-Banner 'Results zip' (@("File: $zip", "Reports in it: $($z.included.Count)") + @($z.excluded | ForEach-Object { "Left out: $_" }) + @('', 'UPLOAD THIS FILE IN THE PROJECT THREAD.')) 'Green'
  return $z
}

# ===================================================================== self-test

function Invoke-SelfTest {
  $script:Fails = 0
  $script:RetestName = ''
  function Check([string]$Name, [bool]$Ok) { if ($Ok) { Write-Host "[PASS] $Name" -ForegroundColor Green } else { Write-Host "[FAIL] $Name" -ForegroundColor Red; $script:Fails++ } }
  $tmp = Join-Path ([IO.Path]::GetTempPath()) ('hykit-selftest-' + [guid]::NewGuid().ToString('N'))
  New-Item -ItemType Directory -Force -Path $tmp | Out-Null
  $script:Unattended = $true
  $script:Sim = @{ windows = 'Windows 11 Pro'; build = 22631; windows11 = $true; ram_gb = 8; cores = 4; laptop = $true; webcams = 1; pktmon = $true; elevated = $false; admin_member = $false; powershell = 'test' }
  try {
    # --- the server log is read while the running server holds it open for writing
    $logFile = Join-Path $tmp 'server.log'
    $writer = [IO.FileStream]::new($logFile, [IO.FileMode]::Create, [IO.FileAccess]::Write, [IO.FileShare]'ReadWrite, Delete')
    try {
      $bytes = [Text.Encoding]::UTF8.GetBytes("one`ntwo`n"); $writer.Write($bytes, 0, $bytes.Length); $writer.Flush()
      if ($env:OS -eq 'Windows_NT') {
        $refused = $false; try { $null = @([IO.File]::ReadLines($logFile)) } catch { $refused = $true }
        Check 'Windows refuses a plain read of a log a writer holds (the field failure)' $refused
      }
      $got = @(Read-SharedLines $logFile)
      Check 'the server log is read while a writer holds it' ($got.Count -eq 2 -and $got[1] -eq 'two')
    } finally { $writer.Dispose() }
    # --- verdict rules
    $j = Join-Path $tmp 'v.json'
    Write-JsonFile $j @{ verdict = 'PASS' }
    Check 'exit 0 with a PASS report is PASS' ((Get-ExitVerdict 0 $j 'verdict').verdict -eq 'PASS')
    Write-JsonFile $j @{ verdict = 'INCONCLUSIVE' }
    Check 'exit 0 with an INCONCLUSIVE report is not PASS' ((Get-ExitVerdict 0 $j 'verdict').verdict -eq 'INCONCLUSIVE')
    Check 'exit 1 is FAIL' ((Get-ExitVerdict 1 $j 'verdict').verdict -eq 'FAIL')
    Check 'no report is never PASS (exit 0)' ((Get-ExitVerdict 0 (Join-Path $tmp 'none.json') 'verdict').verdict -eq 'INCONCLUSIVE')
    Write-JsonFile $j @{ ok = $true; checks = @(@{ Name = 'a'; Pass = $true; Required = $true }) }
    Check 'soak ok is PASS' ((Get-SoakVerdict $j 0).verdict -eq 'PASS')
    Write-JsonFile $j @{ ok = $false; checks = @(@{ Name = 'awake time covered'; Pass = $false; Required = $true }) }
    $sv = Get-SoakVerdict $j 1
    Check 'soak not ok is FAIL and names the check' ($sv.verdict -eq 'FAIL' -and $sv.detail -like '*awake time covered*')
    Check 'soak with no report is INCONCLUSIVE' ((Get-SoakVerdict (Join-Path $tmp 'none.json') 1).verdict -eq 'INCONCLUSIVE')
    $g = Join-Path $tmp 'g.txt'
    @('[PASS] a', '[PASS] b') | Set-Content $g
    Check 'guided: all answered and passed is PASS' ((Get-GuidedVerdict $g).verdict -eq 'PASS')
    @('[PASS] a', '[SKIP] b') | Set-Content $g
    Check 'guided: a skipped question is INCONCLUSIVE' ((Get-GuidedVerdict $g).verdict -eq 'INCONCLUSIVE')
    @('[PASS] a', '[FAIL] b - seen') | Set-Content $g
    Check 'guided: a failed question is FAIL' ((Get-GuidedVerdict $g).verdict -eq 'FAIL')
    @('[INFO] a') | Set-Content $g
    Check 'guided: no answer at all is INCONCLUSIVE' ((Get-GuidedVerdict $g).verdict -eq 'INCONCLUSIVE')
    Write-JsonFile $j @{ pass = $false; setups = @(@{ setup = 1; sessions = 2; pass = $true }) }
    Check 'gate: incomplete is INCONCLUSIVE' ((Get-GateVerdict $j).verdict -eq 'INCONCLUSIVE')
    Write-JsonFile $j @{ pass = $false; setups = @(@{ setup = 1; sessions = 2; pass = $false }, @{ setup = 2; sessions = 1; pass = $false }) }
    Check 'gate: a complete setup failing is FAIL' ((Get-GateVerdict $j).verdict -eq 'FAIL')
    Write-JsonFile $j @{ pass = $true; setups = @(1..5 | ForEach-Object { @{ setup = $_; sessions = 2; pass = $true } }) }
    Check 'gate: passed is PASS' ((Get-GateVerdict $j).verdict -eq 'PASS')
    Check 'join: PASS and INCONCLUSIVE is INCONCLUSIVE' ((Join-Verdicts @('PASS', 'INCONCLUSIVE')) -eq 'INCONCLUSIVE')
    Check 'join: SKIPPED only is SKIPPED' ((Join-Verdicts @('SKIPPED')) -eq 'SKIPPED')
    Check 'join: FAIL wins' ((Join-Verdicts @('PASS', 'FAIL', 'INCONCLUSIVE')) -eq 'FAIL')

    # --- rows from the PC
    $r = Select-Rows @{ windows11 = $true; webcams = 1; laptop = $true; ram_gb = 8 } 'Camera'
    Check 'Windows 11 laptop: CS1, CS4, S3; no CS3; no S1' (($r.camera_row -join ',') -eq 'CS1,CS4' -and ($r.capture -join ',') -eq 'S3' -and -not $r.second_webcam -and -not $r.companion_soak)
    $r = Select-Rows @{ windows11 = $false; webcams = 2; laptop = $true; ram_gb = 4 } 'Camera'
    Check 'Windows 10 4 GB laptop with 2 webcams: CS2, CS3 offered, S3+S4, S1 offered' (($r.camera_row -join ',') -eq 'CS2,CS4' -and $r.second_webcam -and ($r.capture -join ',') -eq 'S3,S4' -and $r.companion_soak)
    $r = Select-Rows @{ windows11 = $true; webcams = 1; laptop = $false; ram_gb = 16 } 'Camera'
    Check 'desktop with one webcam: CS3 as usb' ($r.camera_kind -eq 'usb' -and ($r.camera_row -join ',') -eq 'CS3,CS4')
    $r = Select-Rows @{ windows11 = $true; webcams = 0; laptop = $true; ram_gb = 8 } 'Camera'
    Check 'no webcam: no camera rows, a note' (-not $r.camera_kind -and @($r.camera_row).Count -eq 0 -and @($r.notes).Count -ge 1)

    # --- sequencing and resume with stub steps
    $script:StateDirResolved = Join-Path $tmp 'state'
    $script:StatePath = Join-Path $script:StateDirResolved 'state.json'
    $script:LogFile = Join-Path $script:StateDirResolved 'kit-log.txt'
    New-Item -ItemType Directory -Force -Path (Join-Path $script:StateDirResolved 'results') | Out-Null
    $script:RoleName = 'Camera'; $script:PcLabel = 'PC2'
    $script:State = Read-State
    $script:Calls = New-Object System.Collections.Generic.List[string]
    $script:CloseAt = 'B'
    $stubs = @(
      @{ Id = 'A'; Name = 'first'; Minutes = '1'; Human = 'no'; Run = { param($c) $script:Calls.Add('A'); New-Result 'PASS' 'ok' @('X1') } },
      @{ Id = 'B'; Name = 'second'; Minutes = '1'; Human = 'no'; Run = { param($c) $script:Calls.Add('B'); if ($script:CloseAt -eq 'B') { $script:CloseAt = ''; throw 'KIT-SIMULATED-CLOSE' }; New-Result 'FAIL' 'seen' } },
      @{ Id = 'C'; Name = 'third'; Minutes = '1'; Human = 'yes'; Run = { param($c) $script:Calls.Add('C'); if (-not $script:State.data['c_flag']) { $script:State.data['c_flag'] = $true; return @{ verdict = 'PAUSE'; detail = 'sign out' } }; New-Result 'INCONCLUSIVE' 'missing x' } },
      @{ Id = 'D'; Name = 'fourth'; Minutes = '1'; Human = 'no'; Run = { param($c) $script:Calls.Add('D'); throw 'boom' } }
    )
    $closed = $false
    try { $null = Invoke-Steps $stubs '' } catch { $closed = $_.Exception.Message -like 'KIT-SIMULATED-CLOSE*' }
    Check 'a closed window stops the run' $closed
    $script:State = Read-State
    Check 'state: A done, B left running' ($script:State.steps['A'].status -eq 'done' -and $script:State.steps['B'].status -eq 'running')
    $o1 = Invoke-Steps $stubs ''
    Check 'resume: A not run again, B run again, C paused' ($o1 -eq 'paused' -and (($script:Calls) -join '') -eq 'ABBC')
    $script:State = Read-State
    Check 'state after pause: B done FAIL, C pending' ($script:State.steps['B'].verdict -eq 'FAIL' -and $script:State.steps['C'].status -eq 'pending' -and $script:State.steps['B'].attempts -eq 2)
    $o2 = Invoke-Steps $stubs ''
    Check 'after the pause: C finishes, D runs' ($o2 -eq 'complete' -and (($script:Calls) -join '') -eq 'ABBCCD')
    $script:State = Read-State
    Check 'an error is INCONCLUSIVE, never PASS' ($script:State.steps['D'].verdict -eq 'INCONCLUSIVE' -and $script:State.steps['D'].detail -like '*boom*')
    $o3 = Invoke-Steps $stubs ''
    Check 'a finished run runs nothing again' ($o3 -eq 'complete' -and $script:Calls.Count -eq 6)
    $null = Invoke-Steps $stubs 'B'
    Check '-Redo runs that one step again' ((($script:Calls) -join '') -eq 'ABBCCDB')

    # --- administrator window: protected staging, requests, tampering
    $script:AgentInline = $false
    $kitScripts = Join-Path $tmp 'kit-scripts'
    $kitInst = Join-Path $tmp 'kit-installers'
    New-Item -ItemType Directory -Force -Path $kitScripts, $kitInst | Out-Null
    Set-Content -LiteralPath (Join-Path $kitScripts 'Test-Soak.ps1') -Value '# soak v1'
    Set-Content -LiteralPath (Join-Path $kitScripts 'Test-NetworkCapture.ps1') -Value '# capture v1'
    Set-Content -LiteralPath (Join-Path $kitInst 'HealthierYou-Server_0.1.0_x64-setup.exe') -Value 'server v1'
    $saveScripts = $script:ScriptsDirResolved; $saveInst = $script:InstallersDirResolved; $saveManifest = $script:Manifest
    $script:ScriptsDirResolved = $kitScripts; $script:InstallersDirResolved = $kitInst
    $script:Manifest = @{ sha256 = @{ 'Test-Soak.ps1' = (Get-FileHash -LiteralPath (Join-Path $kitScripts 'Test-Soak.ps1') -Algorithm SHA256).Hash; 'HealthierYou-Server_0.1.0_x64-setup.exe' = (Get-FileHash -LiteralPath (Join-Path $kitInst 'HealthierYou-Server_0.1.0_x64-setup.exe') -Algorithm SHA256).Hash } }
    $script:RoleName = 'Server'
    $script:AdminRootOverride = Join-Path $tmp 'admin'
    $script:PinnedDigests = Get-StagePins
    Check 'pins: the kit window pins the role''s script and the server installer from the manifest' ($script:PinnedDigests.Count -eq 2 -and $script:PinnedDigests['Test-Soak.ps1'] -eq $script:Manifest.sha256['Test-Soak.ps1'])
    Initialize-AdminArea
    Check 'staging: the role''s script and the server installer are staged' ($script:AdminReady -and $script:StageHashes.ContainsKey('Test-Soak.ps1') -and $script:StagedInstaller -eq 'HealthierYou-Server_0.1.0_x64-setup.exe')
    $q = Get-AdminDir 'queue'; $out = Get-AdminDir 'out'
    Write-JsonFile (Join-Path $q 'job-20260101000000001-echo.json') @{ op = 'echo'; params = @{ text = 'hi' } }
    Write-JsonFile (Join-Path $q 'job-20260101000000002-format-disk.json') @{ op = 'format-disk'; params = @{} }
    Write-JsonFile (Join-Path $q 'job-20260101000000003-network-capture.json') @{ op = 'network-capture'; params = @{ server = 'srv; rm -rf'; canary = 'KITPC2-AB12' } }
    $null = Invoke-AgentOnce; $null = Invoke-AgentOnce; $null = Invoke-AgentOnce
    $r1 = Read-JsonFile (Join-Path $out 'job-20260101000000001-echo.result.json')
    $r2 = Read-JsonFile (Join-Path $out 'job-20260101000000002-format-disk.result.json')
    $r3 = Read-JsonFile (Join-Path $out 'job-20260101000000003-network-capture.result.json')
    Check 'agent runs a listed step' ($r1.verdict -eq 'PASS' -and $r1.detail -eq 'echo hi')
    Check 'agent refuses an unlisted step' ($r2.verdict -eq 'INCONCLUSIVE' -and $r2.detail -like '*refused*')
    Check 'agent refuses bad parameters' ($r3.verdict -eq 'INCONCLUSIVE' -and $r3.detail -like 'refused*')
    Check 'server name: a computer name with an underscore is accepted' ((Test-ServerName 'FIRSTNAME_LASTNAME') -and (Test-ServerName 'pc_01.corp.local') -and (Test-ServerName '192.168.1.20') -and (Test-ServerName 'kit-server'))
    Check 'server name: a space or other characters are refused' (-not (Test-ServerName 'FIRSTNAME LASTNAME') -and -not (Test-ServerName 'pc;rm') -and -not (Test-ServerName '') -and -not (Test-ServerName '-pc') -and -not (Test-ServerName 'pc.') -and -not (Test-ServerName "pc`n") -and -not (Test-ServerName ('a' * 254)))
    Check 'server name: the capture step''s parameters take an underscore name and refuse a space' ($null -eq (Test-AdminParams 'network-capture' @{ server = 'FIRSTNAME_LASTNAME'; canary = 'KITPC2-AB12' }) -and $null -ne (Test-AdminParams 'network-capture' @{ server = 'FIRSTNAME LASTNAME'; canary = 'KITPC2-AB12' }))
    Check 'server name: an underscore computer name can pair; a name a slip refuses stops pairing with the reason' ((-not (Get-ServerNameProblem 'FIRSTNAME_LASTNAME')) -and (Get-ServerNameProblem 'PC$1') -match 'Rename the PC' -and (Get-ServerNameProblem 'MY PC') -match '-Redo P1-05' -and (Get-ServerNameProblem ''))
    Check 'agent leaves no request behind' (@(Get-ChildItem -LiteralPath $q -Filter 'job-*').Count -eq 0 -and @(Get-ChildItem -LiteralPath $out -Filter '*.running').Count -eq 0)
    # --- an administrator step left half-started by a closed window (office PC1, 29 Oct)
    Set-Content -LiteralPath (Join-Path $out 'agent.pid') -Value $PID
    # This process stands for the open administrator window (its name differs by host).
    $saveAlive = ${function:Test-AgentAlive}; ${function:script:Test-AgentAlive} = { $true }
    $script:State.data['jobs'] = @{ 'soak-server' = @{ name = 'job-20260101000000010-soak-server'; since = (Get-Date).ToString('o') } }
    Set-Content -LiteralPath (Join-Path $out 'job-20260101000000010-soak-server.running') -Value '2026-10-29 22:10:00'
    Check 'a running marker without a live agent PID (an older kit, or a closed window) is not live' (-not (Test-AdminJobLive 'job-20260101000000010-soak-server'))
    $fresh = Invoke-AdminJob 'echo' @{ text = 'again' } 'soak-server' -NoWait
    Check 'an interrupted administrator step is started again, not waited for' ($fresh -ne 'job-20260101000000010-soak-server' -and (Test-Path -LiteralPath (Join-Path $q "$fresh.json")) -and $script:State.data['jobs']['soak-server'].name -eq $fresh)
    $null = Invoke-AgentOnce
    Check 'the fresh step runs and finishes' ((Wait-AdminJob $fresh 'soak-server').detail -eq 'echo again')
    Set-Content -LiteralPath (Join-Path $out 'job-20260101000000011-soak-server.running') -Value "$PID 2026-10-30 06:00:00"
    Check 'a running marker of the open agent is live' (Test-AdminJobLive 'job-20260101000000011-soak-server')
    $script:State.data['jobs'] = @{ 'soak-server' = @{ name = 'job-20260101000000011-soak-server'; since = (Get-Date).ToString('o') } }
    Check 'a step the open agent is running is waited for, not sent again' ((Invoke-AdminJob 'echo' @{ text = 'x' } 'soak-server' -NoWait) -eq 'job-20260101000000011-soak-server')
    Set-Content -LiteralPath (Join-Path $out 'job-20260101000000011-soak-server.running') -Value '1 2026-10-30 06:00:00'
    $lost = Wait-AdminJob 'job-20260101000000011-soak-server' 'soak-server'
    Check 'waiting on a step nobody is running ends INCONCLUSIVE instead of waiting forever' ($lost.verdict -eq 'INCONCLUSIVE' -and $lost.detail -like '*interrupted*')
    Remove-Item -LiteralPath (Join-Path $out 'job-20260101000000010-soak-server.running'), (Join-Path $out 'job-20260101000000011-soak-server.running'), (Join-Path $out 'agent.pid') -Force -ErrorAction SilentlyContinue
    $script:State.data['jobs'] = @{}
    ${function:script:Test-AgentAlive} = $saveAlive
    Check 'the step prompt takes "Redo P1-06", "-redo p1-06" and "P1-06" alike' (((ConvertFrom-StepAnswer 'Redo P1-06') -eq 'P1-06') -and ((ConvertFrom-StepAnswer ' -redo p1-06 ') -eq 'P1-06') -and ((ConvertFrom-StepAnswer 'P1-06') -eq 'P1-06') -and ((ConvertFrom-StepAnswer 'redo') -eq 'REDO'))

    # Tampering: after staging, the user's copy is replaced and a request names its own report path.
    Set-Content -LiteralPath (Join-Path $kitScripts 'Test-Soak.ps1') -Value '# EVIL'
    $script:Seen = @{}
    $script:ScriptStub = {
      param($Name, $A, $Path)
      $script:Seen.path = $Path; $script:Seen.content = (Get-Content -LiteralPath $Path -Raw).Trim()
      $rep = $null; for ($i = 0; $i -lt $A.Count - 1; $i++) { if ($A[$i] -eq '-Report') { $rep = $A[$i + 1] } }
      $script:Seen.report = $rep
      Set-Content $rep 'Overall: PASS'; Write-JsonFile ($rep + '.json') @{ ok = $true; checks = @() }
      return 0
    }
    $evilReport = Join-Path $tmp 'elsewhere.txt'
    $rs = Invoke-AdminOp 'soak-server' @{ hours = '0.01'; report = $evilReport }
    $stageFull = [IO.Path]::GetFullPath((Get-AdminDir 'stage'))
    Check 'tamper: the elevated side runs the staged copy, not the user''s replaced file' ($script:Seen.path -and [IO.Path]::GetFullPath($script:Seen.path).StartsWith($stageFull) -and $script:Seen.content -eq '# soak v1')
    Check 'tamper: the report path comes from the agent, never the request' ($script:Seen.report -and [IO.Path]::GetFullPath($script:Seen.report).StartsWith([IO.Path]::GetFullPath((Get-AdminResultsDir))) -and -not (Test-Path -LiteralPath $evilReport))
    Check 'tamper: the staged run gives its verdict and names its reports' ($rs.verdict -eq 'PASS' -and @($rs.files) -contains 'hy-soak-server.txt')
    Set-Content -LiteralPath (Join-Path (Get-AdminDir 'stage') 'Test-Soak.ps1') -Value '# EVIL'
    $script:Seen = @{}
    $rt = $null; try { $rt = Invoke-AdminOp 'soak-server' @{ hours = '0.01' } } catch { $rt = New-Result 'INCONCLUSIVE' $_.Exception.Message }
    Check 'tamper: a changed staged copy is refused, nothing runs' ($rt.verdict -ne 'PASS' -and $rt.detail -like '*changed after it was staged*' -and -not $script:Seen.path)
    # A user's copy changed after the kit window pinned it is never staged.
    Initialize-AdminArea
    Check 'tamper: a script changed after it was pinned is not staged' (-not $script:AdminReady -and $script:AdminSetupError -like '*Test-Soak.ps1 changed after the kit window checked it*' -and $script:StageHashes.Count -eq 0)
    $rn = Invoke-AdminOp 'soak-server' @{ hours = '0.01' }
    Check 'tamper: with staging refused, every administrator step is refused' ($rn.verdict -eq 'INCONCLUSIVE' -and $rn.detail -like '*not ready*')
    $pinErr = ''; try { $null = Get-StagePins } catch { $pinErr = $_.Exception.Message }
    Check 'pins: a script that does not match kit-manifest.json is refused before the UAC prompt' ($pinErr -like '*Test-Soak.ps1 does not match kit-manifest.json*')
    Set-Content -LiteralPath (Join-Path $kitScripts 'Test-Soak.ps1') -Value '# soak v1'
    # Missing digests are a hard refusal, never a skipped check.
    $goodPins = $script:PinnedDigests
    $script:PinnedDigests = [ordered]@{ 'Test-Soak.ps1' = $goodPins['Test-Soak.ps1'] }
    Initialize-AdminArea
    Check 'pins: a staged file without a pinned digest is refused' (-not $script:AdminReady -and $script:AdminSetupError -like '*HealthierYou-Server_0.1.0_x64-setup.exe has no SHA-256 pinned*')
    $script:PinnedDigests = $null
    Initialize-AdminArea
    Check 'pins: no pins at all is refused' (-not $script:AdminReady -and $script:AdminSetupError -like '*no SHA-256 was pinned*')
    $script:PinnedDigests = $goodPins
    $planted = Join-Path $kitInst 'HealthierYou-Server_0.0.1_x64-setup.exe'
    Set-Content -LiteralPath $planted -Value 'planted'
    Initialize-AdminArea
    Check 'pins: an installer added after the kit window checked the kit is refused' (-not $script:AdminReady -and $script:AdminSetupError -like '*HealthierYou-Server_0.*refused')
    Remove-Item -LiteralPath $planted -Force
    $saveM = $script:Manifest; $script:Manifest = @{ sha256 = @{ 'Test-Soak.ps1' = $saveM.sha256['Test-Soak.ps1'] } }
    $pinErr = ''; try { $null = Get-StagePins } catch { $pinErr = $_.Exception.Message }
    Check 'pins: a file with no SHA-256 in kit-manifest.json is refused before the UAC prompt' ($pinErr -like '*HealthierYou-Server_0.1.0_x64-setup.exe has no SHA-256 in kit-manifest.json*')
    $script:Manifest = $null
    $pinErr = ''; try { $null = Get-StagePins } catch { $pinErr = $_.Exception.Message }
    Check 'pins: no kit-manifest.json at all is refused before the UAC prompt' ($pinErr -like '*no SHA-256 in kit-manifest.json*')
    $script:Manifest = $saveM
    # Results are copied back only when written by this step.
    Initialize-AdminArea
    $old = Join-Path (Get-AdminResultsDir) 'stale.txt'; Set-Content $old 'old PASS'
    (Get-Item -LiteralPath $old).LastWriteTime = (Get-Date).AddHours(-3)
    $cp = Copy-AdminReports (New-Result 'PASS' 'x') (Get-Date).AddMinutes(-1)
    $cp2 = @{ verdict = 'PASS'; detail = 'x'; files = @('stale.txt') }
    $cp2 = Copy-AdminReports $cp2 (Get-Date).AddMinutes(-1)
    Check 'a stale administrator report is not copied, and the step is not PASS' ($cp2.verdict -eq 'INCONCLUSIVE' -and -not (Test-Path -LiteralPath (Get-ResultPath 'stale.txt')))
    # Folder rights on Windows (an elevated run only: CI's Windows runner).
    if ($script:IsWin -and (Test-Elevated)) {
      $script:TestAcl = $true
      $script:AdminRootOverride = Join-Path $tmp 'admin-acl'
      Initialize-AdminArea
      $writeBits = [Security.AccessControl.FileSystemRights]'WriteData, AppendData, WriteExtendedAttributes, WriteAttributes, Delete, DeleteSubdirectoriesAndFiles, ChangePermissions, TakeOwnership'
      function Get-UserWrite([string]$Path) {
        @((Get-Acl -LiteralPath $Path).Access | Where-Object { $_.AccessControlType -eq 'Allow' -and @('S-1-5-32-545', 'S-1-1-0', 'S-1-5-11') -contains $_.IdentityReference.Translate([Security.Principal.SecurityIdentifier]).Value -and ($_.FileSystemRights -band $writeBits) })
      }
      Check 'rights: users cannot write the staged files' (@(Get-UserWrite (Get-AdminDir 'stage')).Count -eq 0 -and @(Get-UserWrite (Join-Path (Get-AdminDir 'stage') 'Test-Soak.ps1')).Count -eq 0)
      Check 'rights: users cannot write results' (@(Get-UserWrite (Get-AdminResultsDir)).Count -eq 0)
      Check 'rights: users may write requests' (@(Get-UserWrite (Get-AdminDir 'queue')).Count -ge 1)
      $rawUsers = @((Get-Acl -LiteralPath (Get-AdminDir 'raw')).Access | Where-Object { @('S-1-5-32-545', 'S-1-1-0', 'S-1-5-11') -contains $_.IdentityReference.Translate([Security.Principal.SecurityIdentifier]).Value })
      Check 'rights: raw captures are administrators only' ($rawUsers.Count -eq 0)
      Check 'rights: the folders belong to Administrators' ((Get-Acl -LiteralPath (Get-AdminDir 'stage')).GetOwner([Security.Principal.SecurityIdentifier]).Value -eq 'S-1-5-32-544')
      $script:TestAcl = $false
    }
    else { Write-Host '[SKIP] folder rights (needs an elevated Windows run; CI runs it)' }

    # --- the elevated entry: the bootstrap runs only the bytes the kit window started with
    $bt = Join-Path $tmp 'boot-test'
    $btKit = Join-Path $bt 'kit folder'; $btState = Join-Path $bt 'state'
    New-Item -ItemType Directory -Force -Path $btKit, $btState | Out-Null
    $btSrc = Join-Path $btKit 'Invoke-OfficeTestKit.ps1'
    $fakeV1 = "param([switch]`$Agent, [string]`$Role, [string]`$StateDir, [string]`$InstallersDir, [string]`$ScriptsDir, [int]`$OwnerPid, [string]`$StagePins, [string]`$AdminRoot, [switch]`$AgentOnce)`n# kit v1`nSet-Content -LiteralPath (Join-Path `$StateDir 'ran-v1.txt') -Value @(`$PSCommandPath, `$Role, `$OwnerPid, [string]`$Agent, `$env:PSModulePath)`n"
    $fakeEvil = "param([switch]`$Agent, [string]`$Role, [string]`$StateDir, [string]`$InstallersDir, [string]`$ScriptsDir, [int]`$OwnerPid, [string]`$StagePins, [string]`$AdminRoot, [switch]`$AgentOnce)`nSet-Content -LiteralPath (Join-Path `$StateDir 'EVIL-RAN.txt') -Value 'evil'`n"
    [IO.File]::WriteAllText($btSrc, $fakeV1)
    $v1Hash = Get-FileSha256 $btSrc
    $btRoot = Join-Path $bt 'admin'
    $bootArgs = @{ role = 'Camera'; pins = ('Test-NetworkCapture.ps1=' + ('A' * 64)); sha256 = $v1Hash; src = $btSrc; root = $btRoot; acl = $false; pause = $false; state = $btState; inst = (Join-Path $btKit 'installers'); scripts = $btKit; ownerPid = 4242 }
    function Invoke-BootForTest([hashtable]$A) {
      $ErrorActionPreference = 'Continue'
      $o = & (Get-HostExe) -NoProfile -NonInteractive -ExecutionPolicy Bypass -EncodedCommand (ConvertTo-EncodedCommand (New-AgentBootstrap $A)) 2>&1 | Out-String
      return @{ code = $LASTEXITCODE; out = $o }
    }
    $btBoot = Join-Path $btRoot 'boot'
    # (a) the user's copy is replaced between the launch (hash taken) and the elevated run.
    [IO.File]::WriteAllText($btSrc, $fakeEvil)
    $ra = Invoke-BootForTest $bootArgs
    Check 'elevated entry: a kit script changed after launch is refused (exit 3)' ($ra.code -eq 3 -and $ra.out -like '*Refused*')
    Check 'elevated entry: the replacement never runs and nothing is written' (-not (Test-Path -LiteralPath (Join-Path $btState 'EVIL-RAN.txt')) -and -not (Test-Path -LiteralPath $btRoot))
    # (b) unchanged: the protected copy runs, never the user's file; later changes have no effect.
    [IO.File]::WriteAllText($btSrc, $fakeV1)
    $rb = Invoke-BootForTest $bootArgs
    $ran = @(); if (Test-Path -LiteralPath (Join-Path $btState 'ran-v1.txt')) { $ran = @(Get-Content -LiteralPath (Join-Path $btState 'ran-v1.txt')) }
    $btStaged = Join-Path $btBoot 'Invoke-OfficeTestKit.ps1'
    Check 'elevated entry: unchanged, the protected copy runs as the agent with its role' ($rb.code -eq 0 -and $ran.Count -ge 4 -and [IO.Path]::GetFullPath($ran[0]) -eq [IO.Path]::GetFullPath($btStaged) -and $ran[1] -eq 'Camera' -and $ran[2] -eq '4242' -and $ran[3] -eq 'True')
    $userMods = @([Environment]::GetFolderPath('MyDocuments'), [Environment]::GetFolderPath('UserProfile'), [Environment]::GetFolderPath('LocalApplicationData')) | Where-Object { $_ }
    Check 'elevated entry: the agent''s module path holds no per-user folder' ($ran.Count -ge 5 -and @($userMods | Where-Object { $ran[4].ToLowerInvariant().Contains($_.ToLowerInvariant()) }).Count -eq 0)
    [IO.File]::WriteAllText($btSrc, $fakeEvil)
    Check 'elevated entry: changing the user''s copy afterwards leaves the protected copy as launched' ((Get-FileSha256 $btStaged) -eq $v1Hash -and -not (Test-Path -LiteralPath (Join-Path $btState 'EVIL-RAN.txt')))
    # A link a user put where the protected folder goes is moved aside, never written through.
    [IO.File]::WriteAllText($btSrc, $fakeV1)
    Remove-Item -LiteralPath $btBoot -Recurse -Force
    $lure = Join-Path $bt 'lure'; New-Item -ItemType Directory -Force -Path $lure | Out-Null
    $linkType = if ($script:IsWin) { 'Junction' } else { 'SymbolicLink' }
    $null = New-Item -ItemType $linkType -Path $btBoot -Target $lure
    $rl = Invoke-BootForTest $bootArgs
    Check 'elevated entry: a link in place of the protected folder is moved aside' ($rl.code -eq 0 -and @(Get-ChildItem -LiteralPath $lure -Force).Count -eq 0 -and @(Get-ChildItem -LiteralPath $btRoot -Filter 'boot.untrusted-*' -Force).Count -eq 1 -and (Test-Path -LiteralPath $btStaged))
    # End to end through the real bootstrap and the real kit script as the agent: a test script
    # the agent stages is changed between the launch (pins taken) and the elevated run.
    $e2eOk = (-not $script:IsWin) -or (Test-Elevated)
    if ($e2eOk) {
      $ee = Join-Path $tmp 'e2e'
      $eKit = Join-Path $ee 'kit'; $eScripts = Join-Path $eKit 'scripts'; $eInst = Join-Path $eKit 'installers'; $eState = Join-Path $ee 'state'; $eMark = Join-Path $ee 'marks'
      New-Item -ItemType Directory -Force -Path $eScripts, $eInst, $eState, $eMark | Out-Null
      $eSrc = Join-Path $eKit 'Invoke-OfficeTestKit.ps1'
      Copy-Item -LiteralPath $PSCommandPath -Destination $eSrc
      $soakTpl = "param([string]`$Role, [string]`$Hours, [string]`$Report)`nSet-Content -LiteralPath '" + (Join-Path $eMark 'MARK.txt') + "' -Value 'ran'`nSet-Content -LiteralPath `$Report -Value 'Overall: PASS'`nSet-Content -LiteralPath (`$Report + '.json') -Value '{""ok"":true,""checks"":[]}'`n"
      $eSoak = Join-Path $eScripts 'Test-Soak.ps1'
      [IO.File]::WriteAllText($eSoak, $soakTpl.Replace('MARK', 'soak-v1'))
      [IO.File]::WriteAllText((Join-Path $eInst 'HealthierYou-Server_0.1.0_x64-setup.exe'), 'server v1')
      $saveE = @{ r = $script:RoleName; s = $script:ScriptsDirResolved; i = $script:InstallersDirResolved; m = $script:Manifest; st = $script:StateDirResolved; kp = $script:KitScriptPath; kh = $script:KitSha256 }
      $script:RoleName = 'Server'; $script:ScriptsDirResolved = $eScripts; $script:InstallersDirResolved = $eInst; $script:StateDirResolved = $eState
      $script:Manifest = @{ sha256 = @{ 'Test-Soak.ps1' = (Get-FileSha256 $eSoak); 'HealthierYou-Server_0.1.0_x64-setup.exe' = (Get-FileSha256 (Join-Path $eInst 'HealthierYou-Server_0.1.0_x64-setup.exe')) } }
      $script:KitScriptPath = $eSrc; $script:KitSha256 = Get-FileSha256 $eSrc
      $eAcl = [bool]$script:IsWin
      function Invoke-E2E([string]$Root, [string]$Text) {
        New-Item -ItemType Directory -Force -Path (Join-Path $Root 'queue') | Out-Null
        Write-JsonFile (Join-Path (Join-Path $Root 'queue') 'job-20260101000000001-soak-server.json') @{ op = 'soak-server'; params = @{ hours = '0.01' } }
        # As the kit window's request would find it: the folders already belong to Administrators.
        if ($script:IsWin) { & icacls.exe $Root /setowner '*S-1-5-32-544' /T /Q | Out-Null }
        $ErrorActionPreference = 'Continue'
        $o = & (Get-HostExe) -NoProfile -NonInteractive -ExecutionPolicy Bypass -EncodedCommand (ConvertTo-EncodedCommand $Text) 2>&1 | Out-String
        return @{ code = $LASTEXITCODE; out = $o; result = (Read-JsonFile (Join-Path (Join-Path $Root 'out') 'job-20260101000000001-soak-server.result.json')) }
      }
      # Unchanged: the agent stages the pinned script and runs it.
      $eRoot1 = Join-Path $ee 'admin1'
      $t1 = New-AgentLaunchBootstrap $eRoot1 $eAcl $false $true
      $e1 = Invoke-E2E $eRoot1 $t1
      Check 'end to end: unchanged kit, the agent stages the pinned script and runs it' ($e1.code -eq 0 -and $e1.result.verdict -eq 'PASS' -and (Test-Path -LiteralPath (Join-Path $eMark 'soak-v1.txt')))
      if ($e1.code -ne 0 -or $e1.result.verdict -ne 'PASS') { Write-Host $e1.out }
      # Changed after the launch: the bootstrap text (pins) is made first, then the script is swapped.
      $eRoot2 = Join-Path $ee 'admin2'
      $t2 = New-AgentLaunchBootstrap $eRoot2 $eAcl $false $true
      [IO.File]::WriteAllText($eSoak, $soakTpl.Replace('MARK', 'EVIL-soak'))
      $e2 = Invoke-E2E $eRoot2 $t2
      Check 'end to end: a test script changed between launch and elevation is refused by the agent' ($e2.code -eq 4 -and $e2.result.verdict -eq 'INCONCLUSIVE' -and $e2.result.detail -like '*Test-Soak.ps1 changed after the kit window checked it*')
      Check 'end to end: the changed script never ran and was not left staged' (-not (Test-Path -LiteralPath (Join-Path $eMark 'EVIL-soak.txt')) -and -not (Test-Path -LiteralPath (Join-Path (Join-Path $eRoot2 'stage') 'Test-Soak.ps1')))
      if ($e2.code -ne 4) { Write-Host $e2.out }
      # A bootstrap whose pins leave out a file the agent would stage: refused, nothing runs.
      [IO.File]::WriteAllText($eSoak, $soakTpl.Replace('MARK', 'soak-v3'))
      $script:Manifest.sha256['Test-Soak.ps1'] = Get-FileSha256 $eSoak
      $eRoot3 = Join-Path $ee 'admin3'
      $t3 = New-AgentLaunchBootstrap $eRoot3 $eAcl $false $true
      $t3 = $t3.Replace(";HealthierYou-Server_0.1.0_x64-setup.exe=$($script:Manifest.sha256['HealthierYou-Server_0.1.0_x64-setup.exe'])", '').Replace("HealthierYou-Server_0.1.0_x64-setup.exe=$($script:Manifest.sha256['HealthierYou-Server_0.1.0_x64-setup.exe']);", '')
      $e3 = Invoke-E2E $eRoot3 $t3
      Check 'end to end: a missing pinned digest is refused by the agent, nothing runs' ($e3.code -eq 4 -and $e3.result.detail -like '*has no SHA-256 pinned*' -and -not (Test-Path -LiteralPath (Join-Path $eMark 'soak-v3.txt')))
      $script:RoleName = $saveE.r; $script:ScriptsDirResolved = $saveE.s; $script:InstallersDirResolved = $saveE.i; $script:Manifest = $saveE.m; $script:StateDirResolved = $saveE.st; $script:KitScriptPath = $saveE.kp; $script:KitSha256 = $saveE.kh
    }
    else { Write-Host '[SKIP] end-to-end bootstrap run (on Windows it needs an elevated run; CI runs it)' }
    # (c) the elevation command line runs no script file from the user's folder.
    $saveRole = $script:RoleName; $saveSha = $script:KitSha256; $savePath = $script:KitScriptPath
    $saveScripts2 = $script:ScriptsDirResolved; $saveInst2 = $script:InstallersDirResolved; $saveMan2 = $script:Manifest
    $script:ScriptsDirResolved = $kitScripts; $script:InstallersDirResolved = $kitInst; $script:Manifest = $saveM
    $script:RoleName = 'Server'; $script:KitSha256 = $v1Hash; $script:KitScriptPath = $btSrc
    $line = Get-AgentLaunchArgs
    $enc = ($line -split ' ')[-1]
    $decoded = [Text.Encoding]::Unicode.GetString([Convert]::FromBase64String($enc))
    Check 'elevation command line: -EncodedCommand only, no -File, no kit folder path' ($line -match '^-NoProfile -ExecutionPolicy Bypass -EncodedCommand [A-Za-z0-9+/=]+$' -and $line -notmatch '(?i)-File' -and -not $line.Contains($btKit) -and $line.Length -le 30000)
    Check 'elevation bootstrap: embeds the launch hash and runs no -File' ($decoded.Contains("'$v1Hash'") -and $decoded -notmatch '(?i)-File\b' -and -not $decoded.Contains($btKit))
    Check 'elevation bootstrap: embeds the pinned digest of every file to stage' ($decoded.Contains("StagePins = '" + (ConvertTo-PinText $goodPins) + "'") -and $goodPins.Count -eq 2)
    $script:RoleName = $saveRole; $script:KitSha256 = $saveSha; $script:KitScriptPath = $savePath
    $script:ScriptsDirResolved = $saveScripts2; $script:InstallersDirResolved = $saveInst2; $script:Manifest = $saveMan2
    # (d) bad literals are refused before anything is embedded.
    foreach ($bad in @(@{ role = "Server'; calc; '" }, @{ role = 'server' }, @{ sha256 = 'abc' }, @{ sha256 = ($v1Hash.Substring(0, 63) + "'") }, @{ sha256 = $v1Hash.ToLowerInvariant() }, @{ ownerPid = '1; calc' }, @{ pins = '' }, @{ pins = "Test-Soak.ps1=$v1Hash'; calc; '" }, @{ pins = "..\x=$v1Hash" + 'x' }, @{ retest = "Pairing'; calc; '" }, @{ retest = 'pairing' }, @{ retest = 'Soak' })) {
      $x = $bootArgs.Clone(); foreach ($k in $bad.Keys) { $x[$k] = $bad[$k] }
      $refused = $false; try { $null = New-AgentBootstrap $x } catch { $refused = $_.Exception.Message -like 'refused*' }
      Check "elevation bootstrap: bad $(@($bad.Keys)[0]) '$(@($bad.Values)[0])' refused" $refused
    }
    $q1 = $bootArgs.Clone(); $q1.state = "C:\it's `$(calc) here"
    Check 'elevation bootstrap: paths are embedded only as base64' (-not (New-AgentBootstrap $q1).Contains('calc'))
    if ($script:IsWin -and (Test-Elevated)) {
      $acl1 = $bootArgs.Clone(); $acl1.acl = $true; $acl1.root = Join-Path $bt 'admin-acl'
      $rc = Invoke-BootForTest $acl1
      $ab = Join-Path $acl1.root 'boot'
      $wb = [Security.AccessControl.FileSystemRights]'WriteData, AppendData, WriteExtendedAttributes, WriteAttributes, Delete, DeleteSubdirectoriesAndFiles, ChangePermissions, TakeOwnership'
      $uw = @((Get-Acl -LiteralPath $ab).Access | Where-Object { $_.AccessControlType -eq 'Allow' -and @('S-1-5-32-545', 'S-1-1-0', 'S-1-5-11') -contains $_.IdentityReference.Translate([Security.Principal.SecurityIdentifier]).Value -and ($_.FileSystemRights -band $wb) })
      Check 'rights: the protected kit copy belongs to Administrators and users cannot write it' ($rc.code -eq 0 -and $uw.Count -eq 0 -and (Get-Acl -LiteralPath $ab).GetOwner([Security.Principal.SecurityIdentifier]).Value -eq 'S-1-5-32-544')
    }
    else { Write-Host '[SKIP] protected kit copy rights (needs an elevated Windows run; CI runs it)' }
    $script:ScriptStub = $null
    $script:AdminRootOverride = $null
    $script:ScriptsDirResolved = $saveScripts; $script:InstallersDirResolved = $saveInst; $script:Manifest = $saveManifest

    # --- reruns never inherit an old report (guided steps) and failed desk sessions stay to do
    $script:RoleName = 'Camera'
    $script:InstallersDirResolved = $kitInst
    Set-Content -LiteralPath (Join-Path $kitInst 'HealthierYou_0.1.0_x64-setup.exe') -Value 'companion v1'
    $script:Manifest = @{ companion = 'HealthierYou_0.1.0_x64-setup.exe' }
    $ins = Get-ResultPath 'hy-office-install.txt'
    @('[PASS] a', '[PASS] b') | Set-Content $ins
    $script:StubMode = 'fail-no-report'
    $script:ScriptStub = {
      param($Name, $A, $Path)
      $rep = $null; for ($i = 0; $i -lt $A.Count - 1; $i++) { if ($A[$i] -eq '-Report') { $rep = $A[$i + 1] } }
      switch ($script:StubMode) {
        'fail-no-report' { return 1 }
        'fail-with-pass-report' { @('[PASS] a') | Set-Content $rep; return 1 }
        'desk-fail-once' {
          if ($A -contains 'Desk') {
            $script:DeskCalls++
            if ($script:DeskCalls -eq 1) { @('[FAIL] session-report - exited 3') | Set-Content $rep; return 0 }
            @('[PASS] session-report') | Set-Content $rep
            Write-JsonFile ([IO.Path]::ChangeExtension($rep, '.json')) @{ setup = 1 }
            return 0
          }
          Write-JsonFile ([IO.Path]::ChangeExtension($rep, '.json')) @{ pass = $false; setups = @() }; Set-Content $rep 'gate'; return 0
        }
      }
      return 0
    }
    $g1 = Invoke-GuidedInstallStep @{}
    Check 'redo: a failed guided install with no new report does not inherit the old PASS' ($g1.verdict -eq 'INCONCLUSIVE' -and -not (Test-Path -LiteralPath $ins))
    $script:StubMode = 'fail-with-pass-report'
    $g2 = Invoke-GuidedInstallStep @{}
    Check 'redo: a guided script that exits with an error is never PASS' ($g2.verdict -eq 'INCONCLUSIVE' -and $g2.detail -like '*exited with code 1*')
    $si = Get-ResultPath 'hy-office-aftersignin.txt'
    @('[PASS] a') | Set-Content $si
    $script:State.data['signout_asked'] = $true
    $script:StubMode = 'fail-no-report'
    $g3 = Invoke-SignInStep @{}
    Check 'redo: a failed sign-in check does not inherit the old PASS' ($g3.verdict -eq 'INCONCLUSIVE')
    $script:State.data['rows'] = @{ camera_kind = 'built-in' }
    $script:State.data['desk_done'] = @()
    $script:StubMode = 'desk-fail-once'; $script:DeskCalls = 0
    $script:Answers.Clear(); foreach ($x in 'y', 'y', 'd') { $script:Answers.Enqueue($x) }
    $dr = Invoke-DeskStep @{}
    $dd = @($script:State.data['desk_done'])
    Check 'desk: a failed session is not recorded and is offered again; the retry is recorded' ($script:DeskCalls -eq 2 -and $dd.Count -eq 1 -and $dd[0] -eq '1-1')
    Check 'desk: a failed session leaves no counts for the gate' ($dr.verdict -ne 'PASS')
    $script:ScriptStub = $null; $script:StubMode = $null
    $script:Answers.Clear()
    $script:InstallersDirResolved = $saveInst; $script:Manifest = $saveManifest

    # --- zip and redaction
    $res = Join-Path $script:StateDirResolved 'results'
    $script:ExtraTokens = @('ALICE-LAPTOP', 'alicew', 'CORPDOMAIN')
    $script:State.data['server_name'] = 'hy-server01.office.local'
    $script:State.data['canary'] = 'KITPC2-7F3A'
    $planted = "user alicew on ALICE-LAPTOP (CORPDOMAIN) path C:\Users\alicew\AppData\Local\x.txt share \\fileserver\share\a ip 192.168.1.20 v6 fe80::1c2b:3d4e mac 00-1A-2B-3C-4D-5E mail a.b@example.com id 123e4567-e89b-12d3-a456-426614174000 server hy-server01 label KITPC2-7F3A linux /home/alicew/x Windows 10.0.22631 at 12:30:45"
    Set-Content -LiteralPath (Join-Path $res 'report.txt') -Value $planted -Encoding UTF8
    Write-JsonFile (Join-Path $res 'report.txt.json') @{ path = 'C:\Users\alicew\hy.db'; ok = $true; build = '10.0.22631' }
    New-Item -ItemType Directory -Force -Path (Join-Path $res 'desk') | Out-Null
    'start_s,end_s,label' | Set-Content -LiteralPath (Join-Path (Join-Path $res 'desk') 'hy-desk-1-1.labels.csv')
    [IO.File]::WriteAllBytes((Join-Path $res 'frame.png'), [byte[]](0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A))
    [IO.File]::WriteAllBytes((Join-Path $res 'disguised.txt'), [byte[]](0xFF, 0xD8, 0xFF, 0xE0, 0, 0x10, 0x4A, 0x46))
    Set-Content -LiteralPath (Join-Path $res 'phase-PairedOff.pcapng') -Value 'raw'
    Set-Content -LiteralPath (Join-Path $res 'alicew-notes.txt') -Value 'x'
    $zipPath = Join-Path $tmp 'HealthierYou-results-PC2-test.zip'
    $z = New-ResultsZip $res $zipPath 'HealthierYou-results-PC2-test'
    Check 'the zip is written' (Test-Path -LiteralPath $zipPath)
    Add-Type -AssemblyName System.IO.Compression -ErrorAction SilentlyContinue
    Add-Type -AssemblyName System.IO.Compression.FileSystem -ErrorAction SilentlyContinue
    $za = [IO.Compression.ZipFile]::OpenRead($zipPath)
    try {
      $names = @($za.Entries | ForEach-Object { $_.FullName })
      $texts = @{}
      foreach ($en in $za.Entries) { $sr = New-Object IO.StreamReader($en.Open()); $texts[$en.FullName] = $sr.ReadToEnd(); $sr.Dispose() }
    }
    finally { $za.Dispose() }
    Check 'entries use / under one folder' (@($names | Where-Object { $_ -notlike 'HealthierYou-results-PC2-test/*' -or $_ -like '*\*' }).Count -eq 0)
    Check 'images, raw captures and binary files are left out' (-not ($names -match 'frame\.png|pcapng|disguised') -and @($z.excluded | Where-Object { $_ -like 'frame.png*' -or $_ -like 'disguised.txt*' -or $_ -like '*pcapng*' }).Count -eq 3)
    Check 'a file whose name holds a user name is left out' (-not ($names -match 'alicew') -and @($z.excluded | Where-Object { $_ -like '*name held*' }).Count -eq 1)
    $all = ($texts.Values -join "`n")
    foreach ($bad in 'alicew', 'ALICE-LAPTOP', 'CORPDOMAIN', 'C:\Users', 'fileserver', '192.168.1.20', 'fe80::1c2b', '00-1A-2B', 'example.com', '123e4567', 'hy-server01', 'KITPC2-7F3A', '/home/') {
      Check "redacted: $bad" (-not $all.ToLowerInvariant().Contains($bad.ToLowerInvariant()))
    }
    Check 'Windows build and times are kept' ($all.Contains('10.0.22631') -and $all.Contains('12:30:45'))
    $rj = $texts['HealthierYou-results-PC2-test/report.txt.json']
    $okJson = $false; try { $null = $rj | ConvertFrom-Json; $okJson = $true } catch { }
    Check 'a redacted JSON report is still valid JSON' $okJson
    Check 'the desk labels are kept' ($names -contains 'HealthierYou-results-PC2-test/desk/hy-desk-1-1.labels.csv')
    Check 'leftover scan finds nothing in redacted text' ($null -eq (Find-Leftover (Protect-Text $planted (Get-RedactionTokens)) (Get-RedactionTokens)))

    # --- pairing retest: decisions, names, the administrator steps it may take, the server
    # and companion replace with simulated installs, tampering
    Check 'retest decision: nothing installed is install' ((Get-ReplaceDecision '' ('A' * 64)) -eq 'install')
    Check 'retest decision: the same SHA-256 is keep (any letter case)' ((Get-ReplaceDecision ('ab' * 32) ('AB' * 32)) -eq 'keep')
    Check 'retest decision: another SHA-256 is replace' ((Get-ReplaceDecision ('C' * 64) ('A' * 64)) -eq 'replace')
    Check 'retest decision: no SHA-256 in the kit is replace, never keep' ((Get-ReplaceDecision ('C' * 64) '') -eq 'replace')
    $script:RoleName = 'Server'; $script:PcLabel = 'PC1'; $script:RetestName = ''
    $mainNames = @((Get-ResultsZipBase), (Get-StateFileName), (Get-StateDirName 'Server'), (Get-AdminSubName), (Get-AutoResumeName), (Get-Launcher 'Server'))
    Check 'first run: names as before' (($mainNames -join '|') -eq 'HealthierYou-results-PC1|state.json|Server|Server|HealthierYouTestKit|Run-PC1-Server.cmd')
    $script:RetestName = 'Pairing'
    $rtNames = @((Get-ResultsZipBase), (Get-StateFileName), (Get-StateDirName 'Server'), (Get-AdminSubName), (Get-AutoResumeName), (Get-Launcher 'Server'), (Get-Launcher 'Camera'))
    Check 'retest: its own zip, state file, state folder, administrator folder, resume entry and launchers' (($rtNames -join '|') -eq 'HealthierYou-results-PC1-pairing-retest|state-retest-pairing.json|Server-retest-pairing|Server-Retest|HealthierYouTestKitPairingRetest|Run-PC1-Pairing-Retest.cmd|Run-PC2-Pairing-Retest.cmd')
    Check 'retest: steps R1-01 to R1-05 on PC1, R2-01 to R2-03 on PC2' (((@(Get-Steps 'Server') | ForEach-Object { $_.Id }) -join ',') -eq 'R1-01,R1-02,R1-03,R1-04,R1-05' -and ((@(Get-Steps 'Camera') | ForEach-Object { $_.Id }) -join ',') -eq 'R2-01,R2-02,R2-03')
    Check 'retest: a slip-refused PC name points at the retest launcher' ((Get-ServerNameProblem 'MY PC') -match 'Run-PC1-Pairing-Retest\.cmd -Redo R1-05')
    $ro = Invoke-AdminOp 'soak-server' @{ hours = '1' }
    Check 'retest: the administrator window refuses a step the retest does not take' ($ro.verdict -eq 'INCONCLUSIVE' -and $ro.detail -like '*not part of this retest*')
    $script:RoleName = 'Camera'
    $ro = Invoke-AdminOp 'server-replace' @{}
    Check 'retest: PC2''s administrator window refuses the server replace' ($ro.verdict -eq 'INCONCLUSIVE' -and $ro.detail -like '*not part of this retest*')
    $script:RetestName = ''; $script:RoleName = 'Server'
    $ro = Invoke-AdminOp 'server-replace' @{}
    Check 'first run: the administrator window refuses the server replace' ($ro.verdict -eq 'INCONCLUSIVE' -and $ro.detail -like '*not part of this run*')
    Check 'first run: the administrator steps it takes are unchanged' (((Get-AllowedAdminOps) -join ',') -eq 'server-install,server-passphrase,server-pairing-evidence,soak-server,network-capture,echo')
    $script:RetestName = 'Pairing'
    Check 'server replace parameters: a good request passes' ($null -eq (Test-AdminParams 'server-replace' @{ exe_sha256 = ('A' * 64); mirror = 'D:\HealthierYou-Mirror' }))
    Check 'server replace parameters: a bad SHA-256 is refused' ($null -ne (Test-AdminParams 'server-replace' @{ exe_sha256 = 'abc' }) -and $null -ne (Test-AdminParams 'server-replace' @{ exe_sha256 = ('a' * 64) }) -and $null -ne (Test-AdminParams 'server-replace' @{ exe_sha256 = (('A' * 64) + "`n") }))
    Check 'server replace parameters: a mirror with spaces or other characters is refused' ($null -ne (Test-AdminParams 'server-replace' @{ mirror = 'D:\a b' }) -and $null -ne (Test-AdminParams 'server-replace' @{ mirror = 'D:\x;calc' }))
    Check 'server replace parameters: an unexpected parameter is refused' ($null -ne (Test-AdminParams 'server-replace' @{ installer = 'C:\evil.exe' }))
    $rq = Join-Path $tmp 'rq'; New-Item -ItemType Directory -Force -Path $rq | Out-Null
    $saveR = @{ s = $script:ScriptsDirResolved; i = $script:InstallersDirResolved; m = $script:Manifest; ao = $script:AdminRootOverride; pd = $script:PinnedDigests }
    $script:ScriptsDirResolved = $kitScripts; $script:InstallersDirResolved = $kitInst
    $srvName = 'HealthierYou-Server_0.1.0_x64-setup.exe'; $cmpName = 'HealthierYou_0.1.0_x64-setup.exe'
    $script:Manifest = @{ companion = $cmpName; server_exe_sha256 = ('A' * 64); companion_exe_sha256 = ('B' * 64); sha256 = @{ 'Test-Soak.ps1' = (Get-FileSha256 (Join-Path $kitScripts 'Test-Soak.ps1')); $srvName = (Get-FileSha256 (Join-Path $kitInst $srvName)); $cmpName = (Get-FileSha256 (Join-Path $kitInst $cmpName)) } }
    $script:AdminRootOverride = Join-Path $tmp 'admin-retest'
    $script:PinnedDigests = Get-StagePins
    Check 'retest pins: only the server installer is staged on PC1 (no soak script)' ($script:PinnedDigests.Count -eq 1 -and $script:PinnedDigests.Contains($srvName))
    Initialize-AdminArea
    Check 'retest staging: the server installer is staged' ($script:AdminReady -and $script:StagedInstaller -eq $srvName -and -not $script:StageHashes.ContainsKey('Test-Soak.ps1'))
    function New-SimServer([hashtable]$Over = @{}) {
      $h = @{ exe = ('D' * 64); kit_exe = ('A' * 64); cert = 'CERT1'; db = $true; mirror_known = $true; healthy = $true; words = 'alpha bravo charlie delta echo foxtrot'; setup_code = 0; calls = @() }
      foreach ($k in $Over.Keys) { $h[$k] = $Over[$k] }
      return $h
    }
    $good = @{ exe_sha256 = ('A' * 64) }
    $script:SimServer = New-SimServer
    $x = Invoke-AdminOp 'server-replace' $good
    Check 'server replace: another build is uninstalled keeping its data, then the staged installer runs (no /MIRROR: the data''s own)' ((($script:SimServer.calls) -join '|') -eq 'server-uninstall|server-setup /S' -and $x.verdict -eq 'PASS')
    Check 'server replace: the data is kept, so no passphrase is asked for, and the six words come back' ($x.data_kept -eq $true -and $x.passphrase_needed -eq $false -and $x.words -eq 'alpha bravo charlie delta echo foxtrot' -and $x.detail -like '*kept*')
    $script:SimServer = New-SimServer @{ exe = ('A' * 64) }
    $x = Invoke-AdminOp 'server-replace' $good
    Check 'server replace: the kit''s build already installed is kept, nothing runs' ($script:SimServer.calls.Count -eq 0 -and $x.verdict -eq 'PASS' -and $x.passphrase_needed -eq $false -and $x.detail -like '*already the kit*')
    $script:SimServer = New-SimServer @{ exe = ''; cert = ''; db = $false; mirror_known = $false }
    $x = Invoke-AdminOp 'server-replace' @{ exe_sha256 = ('A' * 64); mirror = 'D:\HealthierYou-Mirror' }
    Check 'server replace: no server installed is a fresh install with the mirror, and the passphrase is asked for' ((($script:SimServer.calls) -join '|') -eq 'server-setup /S /MIRROR=D:\HealthierYou-Mirror' -and $x.verdict -eq 'PASS' -and $x.passphrase_needed -eq $true)
    $script:SimServer = New-SimServer @{ mirror_known = $false; mirror = 'E:\HY-Mirror' }
    $x = Invoke-AdminOp 'server-replace' $good
    Check 'server replace: data without its remembered mirror reinstalls on the installed server''s mirror' ((($script:SimServer.calls) -join '|') -eq 'server-uninstall|server-setup /S /MIRROR=E:\HY-Mirror' -and $x.verdict -eq 'PASS')
    $script:SimServer = New-SimServer @{ mirror_known = $false; mirror = '' }
    $x = Invoke-AdminOp 'server-replace' $good
    Check 'server replace: no known mirror is refused before the old server is removed' ($x.verdict -eq 'INCONCLUSIVE' -and $x.detail -like '*mirror folder is not known*' -and $script:SimServer.calls.Count -eq 0)
    $script:SimServer = New-SimServer @{ uninstall_loses_data = $true }
    $x = Invoke-AdminOp 'server-replace' $good
    Check 'server replace: data lost in the uninstall is FAIL and nothing is installed' ($x.verdict -eq 'FAIL' -and (($script:SimServer.calls) -join '|') -eq 'server-uninstall')
    $script:SimServer = New-SimServer @{ uninstall_fails = $true }
    $x = Invoke-AdminOp 'server-replace' $good
    Check 'server replace: an uninstall that does not finish is FAIL and nothing is installed' ($x.verdict -eq 'FAIL' -and (($script:SimServer.calls) -join '|') -eq 'server-uninstall')
    $script:SimServer = New-SimServer @{ kit_exe = ('E' * 64) }
    $x = Invoke-AdminOp 'server-replace' $good
    Check 'server replace: a result that is not the kit''s build is FAIL' ($x.verdict -eq 'FAIL' -and $x.detail -like '*NOT the kit*')
    $script:SimServer = New-SimServer @{ setup_code = 3 }
    $x = Invoke-AdminOp 'server-replace' $good
    Check 'server replace: a setup error is FAIL with its meaning' ($x.verdict -eq 'FAIL' -and $x.detail -like '*exit code 3 (refused*')
    $script:SimServer = New-SimServer
    $x = Invoke-AdminOp 'server-replace' @{}
    Check 'server replace: a request without the kit''s SHA-256 is refused, no uninstall' ($x.verdict -eq 'INCONCLUSIVE' -and $script:SimServer.calls.Count -eq 0)
    Check 'server replace parameters: a missing SHA-256 is refused' ($null -ne (Test-AdminParams 'server-replace' @{}) -and $null -ne (Test-AdminParams 'server-replace' @{ mirror = 'D:\HealthierYou-Mirror' }))
    $script:SimServer = New-SimServer
    $x = Invoke-ServerReplace @{}
    Check 'server replace: the elevated operation itself refuses a missing or malformed SHA-256, no uninstall' ($x.verdict -eq 'INCONCLUSIVE' -and $script:SimServer.calls.Count -eq 0 -and (Invoke-ServerReplace @{ exe_sha256 = 'abc' }).verdict -eq 'INCONCLUSIVE' -and $script:SimServer.calls.Count -eq 0)
    $savedKit = $script:Manifest['server_exe_sha256']
    foreach ($badKit in @($null, '', 'not-a-hash')) {
      $script:Manifest['server_exe_sha256'] = $badKit
      $script:SimServer = New-SimServer
      $x = Invoke-ServerReplaceStep @{}
      Check "server replace step: kit SHA-256 '$badKit' stops before any request or uninstall" ($x.verdict -eq 'INCONCLUSIVE' -and $x.detail -like '*no valid hy-server.exe SHA-256*' -and $script:SimServer.calls.Count -eq 0)
    }
    $script:Manifest['server_exe_sha256'] = $savedKit
    $script:Sim.running_tests = @('Test-Soak.ps1')
    $script:SimServer = New-SimServer
    $x = Invoke-AdminOp 'server-replace' $good
    Check 'server replace: refused while a first-run soak runs, nothing touched' ($x.verdict -eq 'INCONCLUSIVE' -and $x.detail -like '*Test-Soak.ps1 is still running*' -and $script:SimServer.calls.Count -eq 0)
    $script:Sim.running_tests = @()
    Write-JsonFile (Join-Path (Get-AdminDir 'queue') 'job-20260101000000009-server-replace.json') @{ op = 'server-replace'; params = @{ exe_sha256 = ('A' * 64); installer = 'C:\evil.exe' } }
    $script:SimServer = New-SimServer
    $null = Invoke-AgentOnce
    $xq = Read-JsonFile (Join-Path (Get-AdminDir 'out') 'job-20260101000000009-server-replace.result.json')
    Check 'server replace: a request with a parameter of its own is refused, nothing touched' ($xq.verdict -eq 'INCONCLUSIVE' -and $xq.detail -like 'refused*' -and $script:SimServer.calls.Count -eq 0)
    Set-Content -LiteralPath (Join-Path (Get-AdminDir 'stage') $srvName) -Value 'EVIL installer'
    $script:SimServer = New-SimServer
    $x = $null; try { $x = Invoke-AdminOp 'server-replace' $good } catch { $x = New-Result 'INCONCLUSIVE' $_.Exception.Message }
    Check 'tamper: a changed staged server installer is refused before the old server is removed' ($x.verdict -ne 'PASS' -and $x.detail -like '*changed after it was staged*' -and $script:SimServer.calls.Count -eq 0)
    Set-Content -LiteralPath (Join-Path $kitInst $srvName) -Value 'EVIL installer'
    $pinErr = ''; try { $null = Get-StagePins } catch { $pinErr = $_.Exception.Message }
    Check 'tamper: a server installer that does not match kit-manifest.json is refused before the UAC prompt (retest)' ($pinErr -like "*$srvName does not match kit-manifest.json*")
    Set-Content -LiteralPath (Join-Path $kitInst $srvName) -Value 'server v1'
    $script:SimServer = $null
    # The kit window's steps: the companion replace (per user), the passphrase only when needed.
    $script:Sim.calls = @(); $script:Sim.companion_kit_exe = ('B' * 64); $script:Sim.companion_data = $true
    $script:Sim.companion_exe = ('C' * 64)
    $c1 = Invoke-CompanionReplaceStep @{}
    Check 'companion replace: another build is uninstalled keeping its data, then installed from the kit' ((($script:Sim.calls) -join '|') -eq 'companion-uninstall|companion-setup' -and $c1.verdict -eq 'PASS' -and $c1.detail -like '*data folder kept*')
    $script:Sim.calls = @()
    $c2 = Invoke-CompanionReplaceStep @{}
    Check 'companion replace: the kit''s build already installed is kept, nothing runs' ($script:Sim.calls.Count -eq 0 -and $c2.verdict -eq 'PASS' -and $c2.detail -like '*already the kit*')
    $script:Sim.companion_exe = ''
    $c3 = Invoke-CompanionReplaceStep @{}
    Check 'companion replace: not installed (PC2 after the uninstall check) is installed from the kit' ((($script:Sim.calls) -join '|') -eq 'companion-setup' -and $c3.verdict -eq 'PASS')
    $script:Sim.calls = @(); $script:Sim.companion_exe = ('C' * 64); $script:Sim.companion_uninstall_loses_data = $true
    $c4 = Invoke-CompanionReplaceStep @{}
    Check 'companion replace: data lost in the uninstall is FAIL and nothing is installed' ($c4.verdict -eq 'FAIL' -and (($script:Sim.calls) -join '|') -eq 'companion-uninstall')
    $script:Sim.companion_uninstall_loses_data = $false
    $script:Sim.calls = @(); $script:Sim.companion_exe = ('C' * 64); $script:Sim.running_tests = @('Test-CameraSoak.ps1')
    $c5 = Invoke-CompanionReplaceStep @{}
    Check 'companion replace: refused while a first-run camera soak runs, nothing touched' ($c5.verdict -eq 'INCONCLUSIVE' -and $script:Sim.calls.Count -eq 0)
    $script:Sim.running_tests = @()
    Set-Content -LiteralPath (Join-Path $kitInst $cmpName) -Value 'EVIL companion'
    $c6 = Invoke-CompanionReplaceStep @{}
    Check 'companion replace: an installer that does not match kit-manifest.json is never run' ($c6.verdict -eq 'INCONCLUSIVE' -and $c6.detail -like '*does NOT match*' -and $script:Sim.calls.Count -eq 0)
    Set-Content -LiteralPath (Join-Path $kitInst $cmpName) -Value 'companion v1'
    $script:Sim.calls = @(); $script:Sim.companion_exe = ('C' * 64); $script:Sim.companion_kit_exe = ('F' * 64)
    $c7 = Invoke-CompanionReplaceStep @{}
    Check 'companion replace: a result that is not the kit''s build is FAIL' ($c7.verdict -eq 'FAIL' -and $c7.detail -like '*NOT the kit*')
    $savedKit = $script:Manifest['companion_exe_sha256']
    foreach ($badKit in @($null, '', 'not-a-hash')) {
      $script:Manifest['companion_exe_sha256'] = $badKit
      $script:Sim.calls = @(); $script:Sim.companion_exe = ('C' * 64)
      $c8 = Invoke-CompanionReplaceStep @{}
      Check "companion replace: kit SHA-256 '$badKit' stops before any uninstall" ($c8.verdict -eq 'INCONCLUSIVE' -and $c8.detail -like '*no valid hy-companion.exe SHA-256*' -and $script:Sim.calls.Count -eq 0)
    }
    $script:Manifest['companion_exe_sha256'] = $savedKit
    $script:Sim.companion_exe = $null; $script:Sim.companion_kit_exe = $null; $script:Sim.calls = $null; $script:Sim.companion_data = $null
    $script:State.data['passphrase_needed'] = $false
    $pp = Invoke-RetestPassphraseStep @{}
    Check 'retest passphrase: not asked for when the server data was kept' ($pp.verdict -eq 'SKIPPED' -and $pp.detail -like '*-Redo R1-03*')
    $saveInline = @{ a = $script:AgentInline; d = $script:InlineAdminDone; s = $script:AdminStub }
    $script:AgentInline = $true; $script:InlineAdminDone = $true
    $script:AdminStub = { param($Op, $P) if ($Op -eq 'server-passphrase') { $r = New-Result 'PASS' 'stub passphrase'; $r.words = 'w1 w2 w3 w4 w5 w6'; return $r } }
    $script:State.data['passphrase_needed'] = $true
    $pp = Invoke-RetestPassphraseStep @{}
    Check 'retest passphrase: asked for when the server data was not kept' ($pp.verdict -eq 'PASS' -and $script:State.data['six_words'] -eq 'w1 w2 w3 w4 w5 w6')
    $script:State.data['passphrase_needed'] = $false; $script:State.data['six_words'] = $null
    $pp = Invoke-RetestPassphraseStep @{ redo = $true }
    Check 'retest passphrase: -Redo R1-03 asks for it (a forgotten passphrase)' ($pp.verdict -eq 'PASS' -and $script:State.data['six_words'] -eq 'w1 w2 w3 w4 w5 w6')
    $script:AgentInline = $saveInline.a; $script:InlineAdminDone = $saveInline.d; $script:AdminStub = $saveInline.s
    # The elevated entry carries the retest to the agent; a bad value is refused.
    $rtBoot = $bootArgs.Clone(); $rtBoot.retest = 'Pairing'
    Check 'elevation bootstrap: the retest is embedded as a checked literal' ((New-AgentBootstrap $rtBoot).Contains("`$retest = 'Pairing'") -and (New-AgentBootstrap $bootArgs).Contains("`$retest = ''"))
    $fakeRt = "param([switch]`$Agent, [string]`$Role, [string]`$Retest, [string]`$StateDir, [string]`$InstallersDir, [string]`$ScriptsDir, [int]`$OwnerPid, [string]`$StagePins, [string]`$AdminRoot, [switch]`$AgentOnce)`nSet-Content -LiteralPath (Join-Path `$StateDir 'ran-rt.txt') -Value @(`$Role, `$Retest)`n"
    [IO.File]::WriteAllText($btSrc, $fakeRt)
    $rtBoot.sha256 = Get-FileSha256 $btSrc; $rtBoot.root = Join-Path $bt 'admin-rt'
    $rr = Invoke-BootForTest $rtBoot
    $ranRt = @(); if (Test-Path -LiteralPath (Join-Path $btState 'ran-rt.txt')) { $ranRt = @(Get-Content -LiteralPath (Join-Path $btState 'ran-rt.txt')) }
    Check 'elevation bootstrap: the agent starts as the retest' ($rr.code -eq 0 -and $ranRt.Count -ge 2 -and $ranRt[0] -eq 'Camera' -and $ranRt[1] -eq 'Pairing')
    [IO.File]::WriteAllText($btSrc, $fakeV1)
    $script:RetestName = ''; $script:RoleName = 'Camera'; $script:PcLabel = 'PC2'
    $script:ScriptsDirResolved = $saveR.s; $script:InstallersDirResolved = $saveR.i; $script:Manifest = $saveR.m; $script:AdminRootOverride = $saveR.ao; $script:PinnedDigests = $saveR.pd

    # --- the summary and the dry runs of both roles
    $script:Manifest = @{ commit = 'abc1234'; run_id = '1' }
    $sum = Write-Summary $stubs
    Check 'summary lists every step with its verdict' ((@($sum | Where-Object { $_ -match '^(A|B|C|D)\s+(PASS|FAIL|INCONCLUSIVE)' }).Count -eq 4))
    foreach ($role in 'Server', 'Camera') {
      $d = Invoke-DryRun $role (Join-Path $tmp "dry-$role")
      Check "dry run ($role): every step finished, none PASS without its stubbed evidence" ($d.complete -and $d.notRun -eq 0)
      Check "dry run ($role): the results zip is made" ($d.zipOk)
      Check "dry run ($role): the kit check verifies the installers against the manifest" ($d.check -eq 'PASS')
      Check "dry run ($role): the first run's state file and zip keep their names" ((Split-Path -Leaf $d.statePath) -eq 'state.json' -and (Split-Path -Leaf $d.zip) -like "HealthierYou-results-PC?-20*.zip")
    }
    $script:RetestName = 'Pairing'
    # The first run's PC2 state, beside the retest's own folder: its server name is reused.
    Write-JsonFile (Join-Path (Join-Path $tmp 'Camera') 'state.json') @{ steps = @{}; data = @{ server_name = 'FIRST_RUN-PC' } }
    foreach ($role in 'Server', 'Camera') {
      $pc = if ($role -eq 'Server') { 'PC1' } else { 'PC2' }
      $d = Invoke-DryRun $role (Join-Path $tmp "dry-retest-$role")
      Check "retest dry run ($role): every step finished, the zip is made" ($d.complete -and $d.notRun -eq 0 -and $d.zipOk -and $d.check -eq 'PASS')
      Check "retest dry run ($role): its own state file and zip name" ((Split-Path -Leaf $d.statePath) -eq 'state-retest-pairing.json' -and (Split-Path -Leaf $d.zip) -like "HealthierYou-results-$pc-pairing-retest-20*.zip" -and -not (Test-Path -LiteralPath (Join-Path (Join-Path $tmp "dry-retest-$role") 'state.json')))
      if ($role -eq 'Server') {
        Check 'retest dry run (Server): the server and the companion both replaced with data kept, no passphrase asked, then the pairing' ((($d.calls) -join '|') -eq 'companion-uninstall|companion-setup|server-uninstall|server-setup /S' -and (($d.ids) -join ',') -eq 'R1-01,R1-02,R1-03,R1-04,R1-05' -and $d.verdicts['R1-02'] -eq 'PASS' -and $d.verdicts['R1-03'] -eq 'SKIPPED' -and $d.verdicts['R1-04'] -eq 'PASS')
      }
      else {
        Check 'retest dry run (Camera): the companion replaced, then the capture with the pairing passes on its stubbed evidence' ((($d.calls) -join '|') -eq 'companion-uninstall|companion-setup' -and (($d.ids) -join ',') -eq 'R2-01,R2-02,R2-03' -and $d.verdicts['R2-02'] -eq 'PASS' -and $d.verdicts['R2-03'] -eq 'PASS')
        Check 'retest dry run (Camera): the server name given in the first run is used, not asked again' ($script:State.data['server_name'] -eq 'FIRST_RUN-PC')
        Check 'retest dry run (Camera): the first run''s state is left as it was' (((Read-JsonFile (Join-Path (Join-Path $tmp 'Camera') 'state.json')).steps.Count -eq 0))
      }
    }
    $script:RetestName = ''
  }
  finally {
    $script:Sim = $null
    Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
  }
  if ($script:Fails) { Write-Host "$script:Fails self-test check(s) failed" -ForegroundColor Red; exit 1 }
  Write-Host 'Self-test passed.' -ForegroundColor Green
}

# A whole role with stubbed scripts and administrator steps (no PC is changed).
function Invoke-DryRun([string]$RoleName, [string]$Dir) {
  $script:Unattended = $true
  if (-not $script:Sim) { $script:Sim = @{ windows = 'Windows 10 Pro'; build = 19045; windows11 = $false; ram_gb = 4; cores = 4; laptop = $true; webcams = 2; pktmon = $true; elevated = $false; admin_member = $false; powershell = 'dry-run' } }
  $script:RoleName = $RoleName
  $script:PcLabel = if ($RoleName -eq 'Server') { 'PC1' } else { 'PC2' }
  $script:StateDirResolved = $Dir
  $script:StatePath = Join-Path $Dir (Get-StateFileName)
  $script:LogFile = Join-Path $Dir 'kit-log.txt'
  $script:OutDirResolved = $Dir
  $script:AgentInline = $true
  $script:InlineAdminDone = $false
  $script:AdminRootOverride = Join-Path $Dir 'admin'
  $script:InlineResults = @{}
  $script:State = $null
  New-Item -ItemType Directory -Force -Path (Join-Path $Dir 'results') | Out-Null
  # Stand-in installers with a matching manifest, so the kit checks run as on a real kit.
  $inst = Join-Path $Dir 'installers'
  New-Item -ItemType Directory -Force -Path $inst | Out-Null
  $sha = @{}
  foreach ($f in 'HealthierYou_0.1.0_x64-setup.exe', 'HealthierYou_0.1.1_x64-setup.exe', 'HealthierYou-Server_0.1.0_x64-setup.exe') {
    Set-Content -LiteralPath (Join-Path $inst $f) -Value "stand-in $f"
    $sha[$f] = (Get-FileHash -LiteralPath (Join-Path $inst $f) -Algorithm SHA256).Hash
  }
  foreach ($f in 'Test-Soak.ps1', 'Test-NetworkCapture.ps1') {
    $p = Join-Path $script:ScriptsDirResolved $f
    if (Test-Path -LiteralPath $p) { $sha[$f] = Get-FileSha256 $p }
  }
  $script:InstallersDirResolved = $inst
  $kitServerExe = 'A' * 64; $kitCompanionExe = 'B' * 64
  $script:Manifest = @{ commit = 'dry-run'; run_id = '0'; companion = 'HealthierYou_0.1.0_x64-setup.exe'; companion_newer = 'HealthierYou_0.1.1_x64-setup.exe'; server = 'HealthierYou-Server_0.1.0_x64-setup.exe'; sha256 = $sha; server_exe_sha256 = $kitServerExe; companion_exe_sha256 = $kitCompanionExe }
  # A pairing retest starts from a first run's builds: same version, other bytes.
  if ($script:RetestName) {
    $script:Sim.companion_exe = 'C' * 64; $script:Sim.companion_kit_exe = $kitCompanionExe; $script:Sim.companion_data = $true; $script:Sim.calls = @()
    $script:SimServer = @{ exe = ('D' * 64); kit_exe = $kitServerExe; cert = 'CERT1'; db = $true; mirror_known = $true; healthy = $true; words = 'alpha bravo charlie delta echo foxtrot'; setup_code = 0; calls = @() }
  }
  $script:State = Read-State
  $script:Answers.Clear()
  # Each stub writes the report its script would write, with a verdict.
  $script:ScriptStub = {
    param($Name, $A)
    $rep = $null; for ($i = 0; $i -lt $A.Count - 1; $i++) { if ($A[$i] -eq '-Report') { $rep = $A[$i + 1] } }
    if ($rep) { $pd = Split-Path -Parent $rep; if ($pd) { New-Item -ItemType Directory -Force -Path $pd | Out-Null } }
    switch ($Name) {
      'Test-Soak.ps1' { Set-Content $rep 'Overall: PASS'; Write-JsonFile ($rep + '.json') @{ ok = $true; checks = @(@{ Name = 'x'; Pass = $true; Required = $true }) }; return 0 }
      'Test-CameraSoak.ps1' { Set-Content $rep 'Overall: INCONCLUSIVE'; Write-JsonFile ($rep + '.json') @{ verdict = 'INCONCLUSIVE' }; return 2 }
      'Test-NetworkCapture.ps1' { Set-Content $rep 'Overall: PASS'; Write-JsonFile ($rep + '.json') @{ Overall = 'PASS' }; return 0 }
      'Test-HealthierYou-OfficePC.ps1' { @('[PASS] a', '[SKIP] b') | Set-Content $rep; return 0 }
      'Test-HealthierYou-Camera.ps1' {
        if ($A -contains 'DeskReport') { Set-Content $rep 'gate'; Write-JsonFile ([IO.Path]::ChangeExtension($rep, '.json')) @{ pass = $false; setups = @(@{ setup = 1; sessions = 2; pass = $true }) } }
        else { @('[PASS] session-report') | Set-Content $rep; Write-JsonFile ([IO.Path]::ChangeExtension($rep, '.json')) @{ setup = 1 } }
        return 0
      }
    }
    return 0
  }
  $script:AdminStub = {
    param($Op, $P)
    switch ($Op) {
      # The soak and the capture go through the real staged path (with the script stub).
      'soak-server' { return $null }
      'network-capture' { return $null }
      'server-replace' { return $null }
      'server-passphrase' { $r = New-Result 'PASS' 'stub'; $r.words = 'alpha bravo charlie delta echo foxtrot'; return $r }
      'server-pairing-evidence' { return (New-Result 'PASS' 'stub evidence') }
      default { return (New-Result 'PASS' "stub $Op") }
    }
  }
  $steps = Get-Steps $RoleName
  $outcome = 'paused'; $guard = 0
  while ($outcome -eq 'paused' -and $guard -lt 10) { $outcome = Invoke-Steps $steps ''; $guard++ }
  $z = Complete-Run $steps
  $notRun = @($steps | Where-Object { $script:State.steps[$_.Id].status -ne 'done' }).Count
  $calls = @(@($script:Sim.calls) + @($(if ($script:SimServer) { $script:SimServer.calls } else { @() })) | Where-Object { $_ })
  $verdicts = @{}; foreach ($st in $steps) { $verdicts[$st.Id] = [string]$script:State.steps[$st.Id].verdict }
  $script:ScriptStub = $null; $script:AdminStub = $null; $script:AdminRootOverride = $null; $script:SimServer = $null
  return @{ complete = ($outcome -eq 'complete'); notRun = $notRun; zipOk = (Test-Path -LiteralPath $z.zip) -and $z.included.Count -ge 2; check = $script:State.steps[$steps[0].Id].verdict; zip = $z.zip; statePath = $script:StatePath; ids = @($steps | ForEach-Object { $_.Id }); verdicts = $verdicts; calls = $calls }
}

# ===================================================================== entry

$script:InlineResults = @{}
$script:ExtraTokens = @()
$script:Ctx = @{}
$script:Sim = $null
$script:SimServer = $null
$script:RetestName = [string]$Retest

if ($SelfTest) { Initialize-KitPaths; Invoke-SelfTest; return }

# What the person started: the elevated window runs only these exact bytes.
$script:KitScriptPath = $PSCommandPath
$script:KitSha256 = Get-FileSha256 $PSCommandPath

if (-not $Role) { throw 'Give -Role Server or -Role Camera (or -SelfTest), and -Retest Pairing for the pairing retest.' }
$script:RoleName = $Role
$script:PcLabel = if ($Role -eq 'Server') { 'PC1' } else { 'PC2' }
Initialize-KitPaths

if ($DryRun) {
  $dir = if ($StateDir) { $StateDir } else { Join-Path ([IO.Path]::GetTempPath()) ('hykit-dryrun-' + [guid]::NewGuid().ToString('N')) }
  $d = Invoke-DryRun $Role $dir
  Write-Host "Dry run$(if ($Retest) { " ($($Retest.ToLowerInvariant()) retest)" }) finished: complete $($d.complete); zip made $($d.zipOk) ($(Split-Path -Leaf $d.zip)). Steps: $(@($d.ids | ForEach-Object { "$_ $($d.verdicts[$_])" }) -join ', '). Folder: $dir"
  if (-not ($d.complete -and $d.zipOk)) { exit 1 }
  return
}

if (-not $script:IsWin -and -not ($Agent -and $AgentOnce)) { throw 'The office test kit runs on Windows (use -DryRun or -SelfTest elsewhere).' }
$script:StateDirResolved = if ($StateDir) { $StateDir } else { Join-Path (Join-Path $env:LOCALAPPDATA 'HealthierYou-TestKit') (Get-StateDirName $Role) }
New-Item -ItemType Directory -Force -Path (Join-Path $script:StateDirResolved 'results') | Out-Null
$script:StatePath = Join-Path $script:StateDirResolved (Get-StateFileName)
$script:LogFile = Join-Path $script:StateDirResolved 'kit-log.txt'
$script:OutDirResolved = $OutDir

if ($Agent) {
  if ($AdminRoot) { $script:AdminRootOverride = $AdminRoot }
  $script:PinnedDigests = ConvertFrom-PinText $StagePins
  if ($AgentOnce) { exit (Invoke-AgentRequestsOnce) }
  Start-AgentLoop $OwnerPid; return
}

try { $Host.UI.RawUI.WindowTitle = "HealthierYou test kit - $script:PcLabel ($Role)$(if ($Retest) { " - $($Retest.ToLowerInvariant()) retest" })" } catch { }
$script:State = Read-State
$steps = Get-Steps $Role
if (Test-Elevated) {
  Show-Banner 'This window runs as administrator' @('The kit is meant to be started by double-clicking, not with "Run as administrator":', 'steps that must run as you would run elevated instead.')
  if (-not (Read-YesNo 'Carry on anyway (the report records it)?' $false)) { return }
  $script:AgentInline = $true
}
$todo = @($steps | Where-Object { $s = $script:State.steps[$_.Id]; -not ($s -is [hashtable] -and $s.status -eq 'done') -or $Redo -eq $_.Id })
Show-Banner "HealthierYou office test kit - $script:PcLabel$(if ($Retest) { " - $($Retest.ToLowerInvariant()) retest" })" (@("Started $(Get-NowText). Steps to do: $($todo.Count) of $($steps.Count).", 'You can close this window at any time: start it again and it carries on where it stopped.', '') + @($steps | ForEach-Object { $s = $script:State.steps[$_.Id]; '{0}  {1,-13} {2}' -f $_.Id, $(if ($s -is [hashtable] -and $s.status -eq 'done') { $s.verdict } else { 'to do' }), $_.Name })) 'Cyan'
Set-AutoResume $true
$adminTodo = @($todo | Where-Object { $_.Admin })
if ($adminTodo.Count) { $null = Start-AdminAgent @($adminTodo | ForEach-Object { $_.Why }) }

$outcome = Invoke-Steps $steps $Redo
if ($outcome -eq 'paused') { return }
while ($true) {
  $null = Complete-Run $steps
  $again = Read-Answer "To run a step again type its id (for example $($steps[-1].Id)); press Enter to finish" ''
  if (-not $again) { break }
  $again = ConvertFrom-StepAnswer $again
  $match = @($steps | Where-Object { $_.Id -eq $again })
  if (-not $match.Count) { Write-Host "  No step $again." -ForegroundColor Red; continue }
  if ($match[0].Admin -and -not (Test-AgentAlive)) { $script:AgentStarted = $false; $null = Start-AdminAgent @($match[0].Why) }
  $outcome = Invoke-Steps $steps $match[0].Id
  if ($outcome -eq 'paused') { return }
}
Set-AutoResume $false
Show-Banner 'All done' @('Upload the zip named above in the project thread.', 'HealthierYou and (on PC1) the server stay installed: see README.md to remove them.') 'Green'
if ($Retest) { Write-Host '  The first run''s state and results are unchanged: its own launcher carries on with it as before.' }
