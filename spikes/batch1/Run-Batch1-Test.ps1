<#
.SYNOPSIS
  Automated feasibility test for "HY Spike Batch1" (Tauri 2 + bundled WebView2).
  Runs as a standard user. Works on Windows PowerShell 5.1 and PowerShell 7.

.DESCRIPTION
  Installs the app silently, measures start-up, idle memory/CPU, focus behaviour,
  click response, camera load, camera-denied behaviour and uninstall, then writes
  results.csv / results.md and zips everything onto the Desktop.

.PARAMETER IdleMinutes    Idle phase length (default 30).
.PARAMETER CameraMinutes  Camera phase length (default 10).
.PARAMETER Quick          Dry run: idle 3 min, camera 2 min (results not valid for a verdict).
.PARAMETER SkipCamera     Skip C1-C4.
.PARAMETER SkipFocus      Skip P3/P4.
#>
[CmdletBinding()]
param(
    [int]$IdleMinutes = 30,
    [int]$CameraMinutes = 10,
    [switch]$Quick,
    [switch]$SkipCamera,
    [switch]$SkipFocus
)

Set-StrictMode -Off
$ErrorActionPreference = 'Continue'
if ($Quick) { $IdleMinutes = 3; $CameraMinutes = 2 }

$ScriptDir = $PSScriptRoot
if (-not $ScriptDir) { $ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path }
$Inv = [Globalization.CultureInfo]::InvariantCulture

# ---------------------------------------------------------------- native helpers
$nativeSrc = @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class HYNative {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left; public int Top; public int Right; public int Bottom; }
  [StructLayout(LayoutKind.Sequential)] public struct MOUSEINPUT { public int dx; public int dy; public uint mouseData; public uint dwFlags; public uint time; public IntPtr dwExtraInfo; }
  [StructLayout(LayoutKind.Sequential)] public struct INPUT { public uint type; public MOUSEINPUT mi; }
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindow(string cls, string title);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
  [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr h);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint a, uint b, bool attach);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder sb, int max);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern IntPtr GetDC(IntPtr h);
  [DllImport("user32.dll")] public static extern int ReleaseDC(IntPtr h, IntPtr dc);
  [DllImport("gdi32.dll")] public static extern int GetDeviceCaps(IntPtr dc, int idx);
  [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
  [DllImport("kernel32.dll")] public static extern uint SetThreadExecutionState(uint flags);
  [DllImport("user32.dll", SetLastError=true)] static extern uint SendInput(uint n, INPUT[] inputs, int size);
  static void Mouse(uint flag) {
    INPUT[] a = new INPUT[1];
    a[0].type = 0; a[0].mi.dwFlags = flag;
    SendInput(1, a, Marshal.SizeOf(typeof(INPUT)));
  }
  public static void MouseDown() { Mouse(0x0002); }
  public static void MouseUp()   { Mouse(0x0004); }
  public static int Dpi() {
    IntPtr dc = GetDC(IntPtr.Zero); int d = GetDeviceCaps(dc, 88); ReleaseDC(IntPtr.Zero, dc); return d;
  }
  public static string Title(IntPtr h) { StringBuilder sb = new StringBuilder(256); GetWindowText(h, sb, 256); return sb.ToString(); }
}
'@
if (-not ('HYNative' -as [type])) { Add-Type -TypeDefinition $nativeSrc -Language CSharp }
Add-Type -AssemblyName System.Windows.Forms
try { [void][HYNative]::SetProcessDPIAware() } catch {}
# keep the PC awake during the test (system + display)
try { [void][HYNative]::SetThreadExecutionState([uint32]2147483651) } catch {}

# ---------------------------------------------------------------- paths and state
$Desktop = [Environment]::GetFolderPath('Desktop')
if (-not $Desktop -or -not (Test-Path $Desktop)) { $Desktop = Join-Path $env:USERPROFILE 'Desktop' }
if (-not (Test-Path $Desktop)) { $Desktop = $env:USERPROFILE }
$Stamp = Get-Date -Format 'yyyyMMdd-HHmm'
$ResultsDir = Join-Path $Desktop ("HY-Batch1-Results-" + $Stamp)
New-Item -ItemType Directory -Path $ResultsDir -Force | Out-Null
$LogDir = Join-Path $env:LOCALAPPDATA 'HY-Spike\logs'
$RunLog = Join-Path $ResultsDir 'run-log.txt'
$SampleCsv = Join-Path $ResultsDir 'process-samples.csv'

$script:Results = New-Object System.Collections.ArrayList
$script:Failures = New-Object System.Collections.ArrayList
$script:PhaseTimes = New-Object System.Collections.ArrayList
$script:Samples = New-Object System.Collections.ArrayList
$script:Exe = $null
$script:InstallDir = $null
$script:Run = $null
$script:IdleRun = $null
$script:IdleApps = $null
$script:Sys = @{}
$script:OnlineSkipped = $false
$script:CameraUnavailable = $false
$script:PhaseNo = 0

# ---------------------------------------------------------------- generic helpers
function Clean([string]$s) {
    # strip the Windows user name from anything written to results
    if ($null -eq $s) { return '' }
    $names = @($env:USERNAME, (Split-Path $env:USERPROFILE -Leaf)) | Where-Object { $_ } | Select-Object -Unique
    foreach ($u in $names) {
        if ($u.Length -ge 3) { $s = $s -replace [regex]::Escape($u), '<user>' }
        else { $s = $s -replace ('([\\/]Users[\\/])' + [regex]::Escape($u) + '(?=[\\/]|$)'), '$1<user>' }
    }
    return $s
}

function Log([string]$msg, [string]$color = 'Gray') {
    $line = ("[{0:HH:mm:ss}] {1}" -f (Get-Date), $msg)
    Write-Host $line -ForegroundColor $color
    try { Add-Content -Path $RunLog -Value (Clean $line) -Encoding UTF8 } catch {}
}

function Get-EpochMs { return [int64](((Get-Date).ToUniversalTime() - [datetime]'1970-01-01').TotalMilliseconds) }
function From-EpochMs([int64]$ms) { return ([datetime]'1970-01-01').AddMilliseconds($ms).ToLocalTime() }
function F1($x) { if ($null -eq $x) { return 'n/a' }; return ([double]$x).ToString('0.0', $Inv) }
function Num($s) { try { return [double]::Parse(([string]$s).Trim(), $Inv) } catch { return $null } }

function Add-Test([string]$Id, [string]$Name, [string]$Target) {
    [void]$script:Results.Add([pscustomobject]@{ Id = $Id; Test = $Name; Measured = ''; Target = $Target; Status = 'Not run'; Note = '' })
}
function Set-Result([string]$Id, [string]$Status, [string]$Measured, [string]$Note) {
    $r = $script:Results | Where-Object { $_.Id -eq $Id } | Select-Object -First 1
    if ($r) { $r.Status = $Status; $r.Measured = (Clean $Measured); $r.Note = (Clean $Note) }
    $col = 'Gray'; if ($Status -eq 'Pass') { $col = 'Green' } elseif ($Status -eq 'Fail') { $col = 'Red' } elseif ($Status -eq 'Info') { $col = 'Cyan' } else { $col = 'Yellow' }
    Log ("RESULT {0}: {1}  {2}" -f $Id, $Status, $Measured) $col
}
function Get-Result([string]$Id) { return ($script:Results | Where-Object { $_.Id -eq $Id } | Select-Object -First 1) }

function Invoke-Phase([string]$Title, [string[]]$Ids, [scriptblock]$Body) {
    $script:PhaseNo++
    Write-Host ''
    Log ("===== STEP {0}: {1} =====" -f $script:PhaseNo, $Title) 'Cyan'
    $t0 = Get-Date
    try { & $Body }
    catch {
        $m = $_.Exception.Message
        Log ("PHASE ERROR in '{0}': {1}" -f $Title, $m) 'Red'
        [void]$script:Failures.Add((Clean ("{0}: {1}" -f $Title, $m)))
        foreach ($id in $Ids) {
            $r = Get-Result $id
            if ($r -and $r.Status -eq 'Not run' -and -not $r.Note) { $r.Note = Clean ("Phase error: " + $m) }
        }
    }
    [void]$script:PhaseTimes.Add([pscustomobject]@{ Phase = $Title; Start = $t0.ToString('s'); End = (Get-Date).ToString('s'); Seconds = [int]((Get-Date) - $t0).TotalSeconds })
    Log ("----- end of step: {0} -----" -f $Title)
}

function Wait-Enter([string]$msg) { [void](Read-Host $msg) }

# ---------------------------------------------------------------- Windows / process helpers
function Get-Procs {
    try { return @(Get-CimInstance Win32_Process -ErrorAction Stop) }
    catch { return @(Get-WmiObject Win32_Process) }
}

# app + descendant msedgewebview2 processes (walk ParentProcessId)
function Get-AppTree {
    $all = Get-Procs
    $set = @{}
    foreach ($p in $all) { if ($p.Name -ieq 'hy-spike-batch1.exe') { $set[[int]$p.ProcessId] = $p } }
    $changed = $true
    while ($changed) {
        $changed = $false
        foreach ($p in $all) {
            $id = [int]$p.ProcessId
            if (-not $set.ContainsKey($id) -and $set.ContainsKey([int]$p.ParentProcessId)) { $set[$id] = $p; $changed = $true }
        }
    }
    return @($set.Values | Where-Object { $_.Name -ieq 'hy-spike-batch1.exe' -or $_.Name -ieq 'msedgewebview2.exe' })
}

function Test-AppRunning { return [bool](Get-Process -Name 'hy-spike-batch1' -ErrorAction SilentlyContinue) }

# one memory sample of the app tree: counters, then WMI perf, then Get-Process
function Get-ProcSample {
    $tree = @(Get-AppTree)
    if ($tree.Count -eq 0) { return $null }
    $ids = @($tree | ForEach-Object { [int]$_.ProcessId })
    $privMB = $null; $method = ''
    try {
        $c = Get-Counter -Counter '\Process(*)\Working Set - Private', '\Process(*)\ID Process' -ErrorAction Stop
        $idmap = @{}; $privmap = @{}
        foreach ($s in $c.CounterSamples) {
            if ($s.Path -like '*\id process') { $idmap[$s.InstanceName] = [int]$s.CookedValue }
            elseif ($s.Path -like '*\working set - private') { $privmap[$s.InstanceName] = [double]$s.CookedValue }
        }
        $sum = 0.0; $found = 0
        foreach ($k in $idmap.Keys) {
            if (($ids -contains $idmap[$k]) -and $privmap.ContainsKey($k)) { $sum += $privmap[$k]; $found++ }
        }
        # only trust the counter sum when every process of the tree was matched (otherwise it under-reports)
        if ($found -eq $ids.Count) { $privMB = $sum / 1MB; $method = 'Get-Counter' }
    } catch {}
    if ($null -eq $privMB) {
        try {
            $w = @(Get-CimInstance Win32_PerfFormattedData_PerfProc_Process -ErrorAction Stop | Where-Object { $ids -contains [int]$_.IDProcess })
            if ($w.Count -gt 0) {
                $sum = 0.0; foreach ($x in $w) { $sum += [double]$x.WorkingSetPrivate }
                $privMB = $sum / 1MB; $method = 'WMI-perf'
            }
        } catch {}
    }
    $wsMB = 0.0; $commitMB = 0.0
    foreach ($p in @(Get-Process -Id $ids -ErrorAction SilentlyContinue)) { $wsMB += $p.WorkingSet64 / 1MB; $commitMB += $p.PrivateMemorySize64 / 1MB }
    if ($null -eq $privMB) { $privMB = $commitMB; $method = 'Get-Process(PrivateMemorySize64, not private WS)' }
    return [pscustomobject]@{ EpochMs = (Get-EpochMs); ProcCount = $tree.Count; PrivateMB = $privMB; WorkingSetMB = $wsMB; Method = $method }
}

function Add-SampleRow([string]$Phase, [int]$ElapsedS, $S) {
    if (-not (Test-Path $SampleCsv)) { Set-Content -Path $SampleCsv -Value 'time,epoch_ms,phase,elapsed_s,process_count,private_ws_mb,workingset64_mb,method' -Encoding UTF8 }
    $row = '{0},{1},{2},{3},{4},{5},{6},{7}' -f (Get-Date).ToString('s'), $S.EpochMs, $Phase, $ElapsedS, $S.ProcCount, (F1 $S.PrivateMB), (F1 $S.WorkingSetMB), $S.Method
    Add-Content -Path $SampleCsv -Value $row -Encoding UTF8
    [void]$script:Samples.Add([pscustomobject]@{ Phase = $Phase; ElapsedS = $ElapsedS; EpochMs = $S.EpochMs; PrivateMB = $S.PrivateMB; WorkingSetMB = $S.WorkingSetMB; Method = $S.Method; ProcCount = $S.ProcCount })
}

function Read-SharedLines([string]$path) {
    try {
        $fs = New-Object IO.FileStream($path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::ReadWrite)
        $sr = New-Object IO.StreamReader($fs)
        $t = $sr.ReadToEnd(); $sr.Close(); $fs.Close()
        return @($t -split "`r?`n" | Where-Object { $_ -ne '' })
    } catch { return @() }
}

# first log file of a given prefix created at/after a launch
function Find-LogFile([string]$prefix, [string]$ext, [int64]$sinceMs) {
    if (-not (Test-Path $LogDir)) { return $null }
    $hits = @()
    foreach ($f in @(Get-ChildItem -Path $LogDir -Filter ("{0}-*.{1}" -f $prefix, $ext) -ErrorAction SilentlyContinue)) {
        if ($f.BaseName -match '-(\d+)$') {
            $ms = [int64]$matches[1]
            if ($ms -ge ($sinceMs - 3000)) { $hits += [pscustomobject]@{ Path = $f.FullName; Ms = $ms } }
        }
    }
    if ($hits.Count -eq 0) { return $null }
    return ($hits | Sort-Object Ms | Select-Object -First 1).Path
}
function Resolve-RunFile($run, [string]$prefix, [string]$ext) {
    if (-not $run) { return $null }
    $key = "File_" + $prefix
    if ($run[$key]) { return $run[$key] }
    $f = Find-LogFile $prefix $ext $run.LaunchMs
    if ($f) { $run[$key] = $f }
    return $f
}
function Get-RunEvents($run) {
    $p = Resolve-RunFile $run 'events' 'tsv'
    $out = @()
    if (-not $p) { return $out }
    foreach ($l in (Read-SharedLines $p)) {
        $f = $l -split "`t", 4
        if ($f.Count -ge 3 -and $f[0] -match '^\d+$') {
            $json = ''; if ($f.Count -ge 4) { $json = $f[3] }
            $out += [pscustomobject]@{ Ms = [int64]$f[0]; Elapsed = $f[1]; Kind = $f[2].Trim(); Json = $json }
        }
    }
    return $out
}
function Get-RunMetrics($run) {
    $p = Resolve-RunFile $run 'metrics' 'csv'
    $out = @()
    if (-not $p) { return $out }
    foreach ($l in (Read-SharedLines $p)) {
        $f = $l -split ','
        if ($f.Count -ge 8 -and $f[0] -match '^\d+$') {
            $pws = $null; if ($f.Count -ge 9) { $pws = Num $f[8] }
            $out += [pscustomobject]@{ Ms = [int64]$f[0]; ElapsedS = (Num $f[1]); Procs = (Num $f[2]); WorkingMB = (Num $f[3]); PrivateMB = (Num $f[4]); AppCpu = (Num $f[5]); MachineCpu = (Num $f[6]); AvailMB = (Num $f[7]); PrivateWsMB = $pws }
        }
    }
    return $out
}
function Get-JsonNum([string]$json, [string]$name) {
    if ($json -match ('"' + $name + '"\s*:\s*"?([0-9.]+)')) { return (Num $matches[1]) }
    return $null
}

function Find-Installer {
    $named = Join-Path $ScriptDir 'HY Spike Batch1_0.1.0_x64-setup.exe'
    if (Test-Path $named) { return $named }
    $any = Get-ChildItem -Path $ScriptDir -Filter '*setup.exe' -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($any) { return $any.FullName }
    return $null
}

function Find-UninstallEntry {
    $roots = @('HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*',
               'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*',
               'HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*')
    foreach ($r in $roots) {
        foreach ($k in @(Get-ItemProperty -Path $r -ErrorAction SilentlyContinue)) {
            if ($k.DisplayName -and $k.DisplayName -like 'HY Spike Batch1*') { return $k }
        }
    }
    return $null
}

function Find-InstalledExe {
    $cands = @()
    $e = Find-UninstallEntry
    if ($e -and $e.InstallLocation) { $cands += (Join-Path ($e.InstallLocation.Trim('"')) 'hy-spike-batch1.exe') }
    $cands += (Join-Path $env:LOCALAPPDATA 'HY Spike Batch1\hy-spike-batch1.exe')
    $cands += (Join-Path $env:LOCALAPPDATA 'Programs\HY Spike Batch1\hy-spike-batch1.exe')
    foreach ($c in $cands) { if (Test-Path $c) { return $c } }
    return $null
}

# run the uninstaller silently and wait until the exe is gone (NSIS relaunches itself, so poll)
function Invoke-Uninstall {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    $exe = Find-InstalledExe
    $e = Find-UninstallEntry
    $cmd = $null
    if ($e -and $e.UninstallString) { $cmd = $e.UninstallString }
    $path = $null
    if ($cmd) {
        if ($cmd -match '^\s*"([^"]+)"') { $path = $matches[1] } else { $path = ($cmd -replace '\s+/.*$', '').Trim() }
    }
    if ((-not $path -or -not (Test-Path $path)) -and $exe) { $path = Join-Path (Split-Path $exe) 'uninstall.exe' }
    if (-not $path -or -not (Test-Path $path)) { return [pscustomobject]@{ Ran = $false; Gone = $false; Seconds = 0; Path = $path } }
    Get-Process -Name 'hy-spike-batch1' -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    [void](Start-Process -FilePath $path -ArgumentList '/S' -PassThru)
    $gone = $false
    while ($sw.Elapsed.TotalSeconds -lt 120) {
        Start-Sleep -Milliseconds 500
        if (-not $exe -or -not (Test-Path $exe)) { $gone = $true; break }
    }
    return [pscustomobject]@{ Ran = $true; Gone = $gone; Seconds = [math]::Round($sw.Elapsed.TotalSeconds, 1); Path = $path }
}

function Start-App([string]$Mode, [int]$CamMin) {
    Remove-Item Env:HY_AUTOTEST -ErrorAction SilentlyContinue
    Remove-Item Env:HY_CAMERA_MINUTES -ErrorAction SilentlyContinue
    if ($Mode) { $env:HY_AUTOTEST = $Mode; if ($CamMin) { $env:HY_CAMERA_MINUTES = [string]$CamMin } }
    $run = @{ LaunchMs = (Get-EpochMs); Mode = $Mode; Sw = [Diagnostics.Stopwatch]::StartNew() }
    $run.Proc = Start-Process -FilePath $script:Exe -WorkingDirectory (Split-Path $script:Exe) -PassThru
    Remove-Item Env:HY_AUTOTEST -ErrorAction SilentlyContinue
    Remove-Item Env:HY_CAMERA_MINUTES -ErrorAction SilentlyContinue
    $script:Run = $run
    return $run
}

function Get-PetWindow {
    $h = [HYNative]::FindWindow([NullString]::Value, 'HY Pet')
    if ($h -ne [IntPtr]::Zero -and [HYNative]::IsWindowVisible($h)) { return $h }
    return [IntPtr]::Zero
}

# poll every 200 ms for the pet window and the pet_visible event
function Wait-PetVisible($run, [int]$TimeoutS) {
    $winS = $null; $evS = $null
    while ($run.Sw.Elapsed.TotalSeconds -lt $TimeoutS) {
        $t = $run.Sw.Elapsed.TotalSeconds
        if ($null -eq $winS -and (Get-PetWindow) -ne [IntPtr]::Zero) { $winS = $t }
        if ($null -eq $evS) {
            $ev = @(Get-RunEvents $run | Where-Object { $_.Kind -eq 'pet_visible' })
            if ($ev.Count -gt 0) { $evS = $t }
        }
        if ($null -ne $winS -and $null -ne $evS) { break }
        if ($run.Proc.HasExited) { break }
        Start-Sleep -Milliseconds 200
    }
    $total = $null
    if ($null -ne $winS -and $null -ne $evS) { $total = [math]::Max($winS, $evS) }
    return [pscustomobject]@{ WindowS = $winS; EventS = $evS; TotalS = $total; Exited = $run.Proc.HasExited }
}

function Ensure-App {
    if (Test-AppRunning) { return $true }
    if (-not $script:Exe) { return $false }
    Log 'App not running; relaunching in normal mode.' 'Yellow'
    $run = Start-App '' 0
    $w = Wait-PetVisible $run 60
    return ($null -ne $w.TotalS)
}

# CloseMainWindow, kill after 10 s, then sweep leftover webview2 processes of the install
function Stop-App {
    $ps = @(Get-Process -Name 'hy-spike-batch1' -ErrorAction SilentlyContinue)
    $graceful = $true
    foreach ($p in $ps) { try { [void]$p.CloseMainWindow() } catch {} }
    $deadline = (Get-Date).AddSeconds(10)
    foreach ($p in $ps) {
        $left = [int]($deadline - (Get-Date)).TotalMilliseconds
        if ($left -lt 0) { $left = 0 }
        try { if (-not $p.WaitForExit($left)) { $graceful = $false; $p.Kill() } } catch {}
    }
    Start-Sleep -Milliseconds 800
    if ($script:InstallDir) {
        foreach ($p in (Get-Procs)) {
            if ($p.Name -ieq 'msedgewebview2.exe' -and $p.ExecutablePath -and $p.ExecutablePath.StartsWith($script:InstallDir, [StringComparison]::OrdinalIgnoreCase)) {
                Stop-Process -Id ([int]$p.ProcessId) -Force -ErrorAction SilentlyContinue
            }
        }
    }
    return $graceful
}

function Set-Foreground([IntPtr]$h) {
    $fg = [HYNative]::GetForegroundWindow()
    $dummy = [uint32]0
    $fgT = [HYNative]::GetWindowThreadProcessId($fg, [ref]$dummy)
    $me = [HYNative]::GetCurrentThreadId()
    $att = $false
    if ($fgT -ne 0 -and $fgT -ne $me) { $att = [HYNative]::AttachThreadInput($me, $fgT, $true) }
    if ([HYNative]::IsIconic($h)) { [void][HYNative]::ShowWindow($h, 9) }
    [void][HYNative]::BringWindowToTop($h)
    [void][HYNative]::SetForegroundWindow($h)
    if ($att) { [void][HYNative]::AttachThreadInput($me, $fgT, $false) }
}

# is the foreground window Notepad (same handle, or a notepad process)?
function Test-NotepadForeground([IntPtr]$npHwnd) {
    $fg = [HYNative]::GetForegroundWindow()
    if ($fg -eq $npHwnd) { return $true }
    $fpid = [uint32]0
    [void][HYNative]::GetWindowThreadProcessId($fg, [ref]$fpid)
    $p = Get-Process -Id ([int]$fpid) -ErrorAction SilentlyContinue
    return ($p -and $p.ProcessName -ieq 'notepad')
}

function Test-Online {
    $gw = $false; $ping = $false; $http = $false
    try {
        $cfg = @(Get-CimInstance Win32_NetworkAdapterConfiguration -Filter 'IPEnabled=True' -ErrorAction Stop | Where-Object { $_.DefaultIPGateway -and @($_.DefaultIPGateway).Count -gt 0 })
        $gw = ($cfg.Count -gt 0)
    } catch {}
    try { $ping = [bool](Test-Connection -ComputerName 1.1.1.1 -Count 1 -Quiet -ErrorAction Stop) } catch {}
    try {
        $req = [Net.WebRequest]::Create('http://1.1.1.1/'); $req.Timeout = 3000; $req.Proxy = $null
        $resp = $req.GetResponse(); $resp.Close(); $http = $true
    } catch [Net.WebException] { if ($_.Exception.Response) { $http = $true } } catch {}
    return [pscustomobject]@{ Gateway = $gw; Ping = $ping; Http = $http; Online = ($gw -or $ping -or $http) }
}

# apps opened to simulate an office PC during the idle test
function Start-IdleApps {
    $apps = @{ Excel = $null; NotepadPid = $null; EdgePre = @(); EdgeStarted = $false; Desc = @() }
    try {
        $xl = New-Object -ComObject Excel.Application -ErrorAction Stop
        $xl.Visible = $true
        [void]$xl.Workbooks.Add()
        $apps.Excel = $xl
        $apps.Desc += 'Excel (blank workbook)'
    } catch {
        $xlPath = $null
        foreach ($k in @('HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\excel.exe', 'HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\excel.exe')) {
            $v = (Get-ItemProperty -Path $k -ErrorAction SilentlyContinue).'(default)'
            if ($v -and (Test-Path $v)) { $xlPath = $v; break }
        }
        if ($xlPath) { try { [void](Start-Process -FilePath $xlPath); $apps.Desc += 'Excel (started via exe, not closed by script)' } catch {} }
        if (-not $xlPath) {
            $note = Join-Path $ResultsDir 'Excel-not-installed.txt'
            Set-Content -Path $note -Value 'Excel not installed' -Encoding ASCII
            try { $np = Start-Process -FilePath 'notepad.exe' -ArgumentList ('"' + $note + '"') -PassThru; $apps.NotepadPid = $np.Id; $apps.Desc += 'Notepad (Excel not installed)' } catch {}
        }
    }
    $edge = $null
    foreach ($c in @((Join-Path ${env:ProgramFiles(x86)} 'Microsoft\Edge\Application\msedge.exe'), (Join-Path $env:ProgramFiles 'Microsoft\Edge\Application\msedge.exe'))) {
        if ($c -and (Test-Path $c)) { $edge = $c; break }
    }
    if ($edge) {
        $apps.EdgePre = @(Get-Process -Name msedge -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
        try { [void](Start-Process -FilePath $edge -ArgumentList '--new-window', '--no-first-run', 'about:blank', 'about:blank'); $apps.EdgeStarted = $true; $apps.Desc += 'Edge (2 blank tabs)' } catch {}
    } else { $apps.Desc += 'Edge not found (skipped)' }
    $script:IdleApps = $apps
}
function Stop-IdleApps {
    $a = $script:IdleApps
    if (-not $a) { return }
    try {
        if ($a.Excel) {
            $a.Excel.DisplayAlerts = $false
            foreach ($wb in @($a.Excel.Workbooks)) { try { $wb.Close($false) } catch {} }
            $a.Excel.Quit()
            [void][Runtime.InteropServices.Marshal]::ReleaseComObject($a.Excel)
        }
    } catch {}
    if ($a.NotepadPid) { Stop-Process -Id $a.NotepadPid -Force -ErrorAction SilentlyContinue }
    if ($a.EdgeStarted) {
        foreach ($p in @(Get-Process -Name msedge -ErrorAction SilentlyContinue)) {
            if ($a.EdgePre -notcontains $p.Id) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue }
        }
    }
    $script:IdleApps = $null
}

function Show-Countdown([string]$label, [double]$remainS) {
    if ($remainS -lt 0) { $remainS = 0 }
    $m = [int][math]::Floor($remainS / 60); $s = [int]($remainS % 60)
    Write-Host ("`r  {0}: {1:00}:{2:00} remaining      " -f $label, $m, $s) -NoNewline
}

# ---------------------------------------------------------------- test list
Add-Test 'I1' 'Offline silent install' 'exit 0, exe present, < 120 s'
Add-Test 'P5' 'Start-up time to pet visible' '<= 10 s'
Add-Test 'I2' 'Bundled WebView2 runtime' 'runtime under install folder'
Add-Test 'P1' 'Idle memory (private WS, app log, last 2/3 of idle)' '<= 150 MB'
Add-Test 'P2' 'Idle CPU (mean, rolling 60 s max)' 'mean <= 2%, no 60 s avg > 5%'
Add-Test 'P3' 'Focus stays in Notepad after pet click' '10/10'
Add-Test 'P4' 'Click response latency' '10 clicks, all <= 500 ms'
Add-Test 'C1' 'Camera CPU (first run mean)' '<= 10%'
Add-Test 'C2' 'Camera peak memory (private WS)' '<= 350 MB'
Add-Test 'C3' 'Camera light off within 2 s (manual)' 'off within ~2 s, 5/5'
Add-Test 'C4' 'Camera permission denied handled' 'camera_error logged, pet keeps running'
Add-Test 'I3' 'Silent uninstall' 'exe removed'
Add-Test 'I4' 'Mark of the Web on installer' 'info only'

Clear-Host
Write-Host '==============================================================' -ForegroundColor Cyan
Write-Host '  HY Spike Batch 1 - automated feasibility test' -ForegroundColor Cyan
Write-Host '==============================================================' -ForegroundColor Cyan
Write-Host ('  Results folder : ' + $ResultsDir)
Write-Host ('  Idle {0} min, camera {1} min{2}' -f $IdleMinutes, $CameraMinutes, $(if ($Quick) { '  (QUICK DRY RUN)' } else { '' }))
Write-Host '  Please do not touch the mouse or keyboard once the test is running.'
Write-Host ''
$isAdmin = $false
try { $isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator) } catch {}
$script:Sys['Elevated'] = $isAdmin

try {
    # ============================================================ STEP 0
    Invoke-Phase 'Collect system info and prepare' @('I4') {
        $cpu = (Get-CimInstance Win32_Processor -ErrorAction SilentlyContinue | Select-Object -First 1)
        $os = Get-CimInstance Win32_OperatingSystem -ErrorAction SilentlyContinue
        $cs = Get-CimInstance Win32_ComputerSystem -ErrorAction SilentlyContinue
        $dispVer = (Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion' -ErrorAction SilentlyContinue).DisplayVersion
        $logical = 0
        foreach ($c in @(Get-CimInstance Win32_Processor -ErrorAction SilentlyContinue)) { $logical += [int]$c.NumberOfLogicalProcessors }
        $dpi = 96; try { $dpi = [HYNative]::Dpi() } catch {}
        $scr = [Windows.Forms.Screen]::PrimaryScreen.Bounds
        $cams = @()
        try { $cams = @(Get-PnpDevice -Class Camera, Image -PresentOnly -ErrorAction Stop) }
        catch { try { $cams = @(Get-CimInstance Win32_PnPEntity -ErrorAction Stop | Where-Object { $_.PNPClass -in @('Camera', 'Image') -and $_.Status -eq 'OK' }) } catch {} }
        $s = $script:Sys
        $s['Processor'] = if ($cpu) { $cpu.Name.Trim() } else { 'unknown' }
        $s['Windows'] = ("{0} {1} (build {2}, version {3})" -f $os.Caption, $os.OSArchitecture, $os.BuildNumber, $dispVer)
        $s['RAM_GB'] = [math]::Round($cs.TotalPhysicalMemory / 1GB, 1)
        $s['LogicalCPUs'] = $logical
        $s['DisplayScaling'] = ("{0}% (DPI {1}), {2}x{3}" -f [int]($dpi / 96 * 100), $dpi, $scr.Width, $scr.Height)
        $s['Webcam'] = if ($cams.Count -gt 0) { 'yes: ' + (($cams | ForEach-Object { $_.FriendlyName } | Select-Object -First 3) -join '; ') } else { 'none detected' }
        $s['PowerShell'] = $PSVersionTable.PSVersion.ToString()
        $s['Started'] = (Get-Date).ToString('s')
        foreach ($k in $s.Keys) { Log ("  {0,-15}: {1}" -f $k, $s[$k]) }
        # Scanners also register as 'Image' devices; only count things that look like cameras.
        $realCams = @($cams | Where-Object { $_.Class -eq 'Camera' -or $_.PNPClass -eq 'Camera' -or $_.FriendlyName -match 'cam' })
        if ($realCams.Count -eq 0 -and -not $SkipCamera) {
            Write-Host '  No webcam detected. Plug the webcam in, check it works in the Windows Camera app, close that app, then press Enter.' -ForegroundColor Yellow
            Wait-Enter '  Press Enter when the webcam is connected (or just Enter to continue without it)'
        }
        if ($isAdmin) { Log 'WARNING: script is running elevated; the test is meant to run as a standard user.' 'Yellow' }

        # network check
        $n = Test-Online
        Log ("Network: gateway={0} ping={1} http={2}" -f $n.Gateway, $n.Ping, $n.Http)
        while ($n.Online) {
            Write-Host ''
            Write-Host '  This PC is ONLINE. The test must run offline.' -ForegroundColor Yellow
            $a = Read-Host '  Disconnect Wi-Fi and cable, then press Enter (or type SKIP to continue online)'
            if ($a -and $a.Trim().ToUpper() -eq 'SKIP') { $script:OnlineSkipped = $true; Log 'Owner chose to continue ONLINE (recorded).' 'Yellow'; break }
            $n = Test-Online
        }
        if (-not $n.Online) { Log 'Network is offline - good.' 'Green' }
        $script:Sys['Network'] = if ($script:OnlineSkipped) { 'ONLINE (owner chose SKIP)' } else { 'offline' }

        # Mark of the Web on the installer (I4)
        $inst = Find-Installer
        if ($inst) {
            $zi = $null
            try { $zi = Get-Content -Path $inst -Stream Zone.Identifier -ErrorAction Stop } catch {}
            if ($zi) { Set-Result 'I4' 'Info' 'Mark of the Web PRESENT' (($zi -join ' ') + ' (Windows may show SmartScreen/Unknown publisher prompts)') }
            else { Set-Result 'I4' 'Info' 'Mark of the Web absent' 'No Zone.Identifier stream on the installer' }
        } else { Set-Result 'I4' 'Info' 'installer not found' '' }

        # close anything that could hold the webcam
        Get-Process -Name 'hy-spike-batch1', 'WindowsCamera' -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
        Write-Host ''
        Write-Host '  Please close any other program that may use the webcam' -ForegroundColor Yellow
        Write-Host '  (Teams, Zoom, browser tabs with video calls, Camera app).' -ForegroundColor Yellow
        Wait-Enter '  Press Enter to start the test'
    }

    # ============================================================ STEP 1  I1
    Invoke-Phase 'I1 Offline silent install' @('I1') {
        $installer = Find-Installer
        if (-not $installer) { Set-Result 'I1' 'Fail' 'installer not found' ("Expected next to the script: " + $ScriptDir); return }
        if (Find-InstalledExe) {
            Log 'App already installed - uninstalling first.'
            $u = Invoke-Uninstall
            Log ("Pre-uninstall: ran={0} gone={1} {2}s" -f $u.Ran, $u.Gone, $u.Seconds)
        }
        Log 'Running installer with /S ...'
        $sw = [Diagnostics.Stopwatch]::StartNew()
        $code = $null; $err = ''
        try { $p = Start-Process -FilePath $installer -ArgumentList '/S' -Wait -PassThru; $code = $p.ExitCode }
        catch { $err = $_.Exception.Message }
        $secs = [math]::Round($sw.Elapsed.TotalSeconds, 1)
        # give the installer a moment to finish writing files
        $exe = $null
        for ($i = 0; $i -lt 20 -and -not $exe; $i++) { $exe = Find-InstalledExe; if (-not $exe) { Start-Sleep -Milliseconds 500 } }
        Get-Process -Name 'hy-spike-batch1' -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
        if ($exe) { $script:Exe = $exe; $script:InstallDir = Split-Path $exe }
        $note = "Script non-elevated: {0}. Exit code 0 with /S = no admin prompt (per-user install). Exe: {1}" -f (-not $isAdmin), $exe
        if ($err) { $note = "Installer failed to start: $err (an elevation request would appear here). " + $note }
        $ok = ($code -eq 0 -and $exe -and $secs -lt 120)
        $st = 'Fail'; if ($ok) { $st = 'Pass' }
        Set-Result 'I1' $st ("exit {0}, {1} s, exe {2}" -f $code, $secs, $(if ($exe) { 'found' } else { 'MISSING' })) $note
    }

    # ============================================================ STEP 2  P5 / I2
    Invoke-Phase 'P5/I2 First launch (offline)' @('P5', 'I2') {
        if (-not $script:Exe) { Log 'No installed exe - skipping.' 'Yellow'; Set-Result 'P5' 'Not run' '' 'App not installed'; Set-Result 'I2' 'Not run' '' 'App not installed'; return }
        Log 'Launching the app (normal mode) ...'
        $run = Start-App '' 0
        $w = Wait-PetVisible $run 60
        $script:IdleRun = $run
        if ($null -eq $w.TotalS) {
            $why = if ($w.Exited) { 'app exited early' } else { 'timeout 60 s' }
            Set-Result 'P5' 'Fail' 'not visible' ("{0}; window at {1} s, pet_visible at {2} s" -f $why, (F1 $w.WindowS), (F1 $w.EventS))
        } else {
            $st = 'Fail'; if ($w.TotalS -le 10) { $st = 'Pass' }
            Set-Result 'P5' $st ((F1 $w.TotalS) + ' s') ("window at {0} s, pet_visible event at {1} s" -f (F1 $w.WindowS), (F1 $w.EventS))
        }
        # I2: WebView2 paths from info file and live processes
        Start-Sleep -Seconds 3
        $infoPath = Resolve-RunFile $run 'info' 'txt'
        $evidence = @(); $ver = ''
        if ($infoPath) {
            $text = (Read-SharedLines $infoPath) -join "`n"
            $text = $text -replace '\\\\', '\'
            if ($text -match 'webview2_version\s*[=:\t ]\s*"?([0-9.]+)') { $ver = $matches[1] }
            foreach ($m in [regex]::Matches($text, '[A-Za-z]:\\[^"\r\n\t<>|*?]+')) {
                if ($m.Value -match '(?i)webview2|EdgeWebView') { $evidence += $m.Value.Trim() }
            }
        }
        foreach ($p in @(Get-AppTree)) { if ($p.Name -ieq 'msedgewebview2.exe' -and $p.ExecutablePath) { $evidence += $p.ExecutablePath } }
        $evidence = @($evidence | Select-Object -Unique)
        $inside = @($evidence | Where-Object { $_.StartsWith($script:InstallDir, [StringComparison]::OrdinalIgnoreCase) })
        $system = @($evidence | Where-Object { $_ -match '(?i)\\Program Files( \(x86\))?\\Microsoft\\(EdgeWebView|Edge)\\' })
        $outside = @($evidence | Where-Object { -not $_.StartsWith($script:InstallDir, [StringComparison]::OrdinalIgnoreCase) })
        $note = "WebView2 version {0}. Paths seen: {1} inside install folder, {2} in system Edge/EdgeWebView, {3} elsewhere. Info file: {4}" -f $ver, $inside.Count, $system.Count, $outside.Count, $(if ($infoPath) { Split-Path $infoPath -Leaf } else { 'MISSING' })
        if ($evidence.Count -eq 0) { Set-Result 'I2' 'Fail' 'no webview2 paths found' $note }
        elseif ($system.Count -eq 0 -and $outside.Count -eq 0 -and $inside.Count -gt 0) { Set-Result 'I2' 'Pass' 'bundled runtime in use' $note }
        else { Set-Result 'I2' 'Fail' 'runtime not (only) from install folder' ($note + ' First outside: ' + ($outside | Select-Object -First 1)) }
    }

    # ============================================================ STEP 3  idle load
    Invoke-Phase 'Idle load (office apps open, app idle)' @('P1', 'P2') {
        if (-not (Ensure-App)) { Set-Result 'P1' 'Not run' '' 'App not running'; Set-Result 'P2' 'Not run' '' 'App not running'; return }
        $script:IdleRun = $script:Run
        Start-IdleApps
        Log ("Idle apps: " + ($script:IdleApps.Desc -join ', '))
        Log ("Idling for {0} min. Please leave the PC alone." -f $IdleMinutes) 'Yellow'
        $idleSec = $IdleMinutes * 60
        $script:IdleStartMs = Get-EpochMs
        $sw = [Diagnostics.Stopwatch]::StartNew()
        $next = 0; $crashed = $false
        while ($sw.Elapsed.TotalSeconds -lt $idleSec) {
            if ($sw.Elapsed.TotalSeconds -ge $next) {
                $s = Get-ProcSample
                if ($s) { Add-SampleRow 'idle' ([int]$sw.Elapsed.TotalSeconds) $s }
                $next += 30
            }
            if (-not (Test-AppRunning)) { $crashed = $true; break }
            Show-Countdown 'Idle' ($idleSec - $sw.Elapsed.TotalSeconds)
            Start-Sleep -Seconds 1
        }
        Write-Host ''
        $script:IdleEndMs = Get-EpochMs
        $script:IdleCrashed = $crashed
        if ($crashed) { Log 'The app disappeared during the idle period!' 'Red' }
        Stop-IdleApps
        Log 'Idle phase finished.'
    }

    # ============================================================ STEP 4  P1 / P2
    Invoke-Phase 'P1/P2 Compute idle memory and CPU' @('P1', 'P2') {
        if (-not $script:IdleEndMs) { return }
        $idleSec = $IdleMinutes * 60
        $fromMs = $script:IdleStartMs + [int64]($idleSec / 3 * 1000)
        $fromS = $idleSec / 3
        $win = "minute {0:0.#} to {1}" -f ($fromS / 60), $IdleMinutes
        $ss = @($script:Samples | Where-Object { $_.Phase -eq 'idle' -and $_.ElapsedS -ge $fromS })
        $mt = @(Get-RunMetrics $script:IdleRun | Where-Object { $_.Ms -ge $fromMs -and $_.Ms -le $script:IdleEndMs })
        $crashNote = ''; if ($script:IdleCrashed) { $crashNote = ' APP EXITED DURING IDLE.' }

        # P1: scored on app-log private_ws_mb only (never on the Get-Counter sampler)
        $avgPws = $null; $appPriv = $null; $appWs = $null
        $pwsRows = @($mt | Where-Object { $null -ne $_.PrivateWsMB })
        if ($pwsRows.Count -gt 0) { $avgPws = ($pwsRows | Measure-Object -Property PrivateWsMB -Average).Average }
        if ($mt.Count -gt 0) { $appPriv = ($mt | Measure-Object -Property PrivateMB -Average).Average; $appWs = ($mt | Measure-Object -Property WorkingMB -Average).Average }
        $smpPriv = $null; $method = ''
        if ($ss.Count -gt 0) { $smpPriv = ($ss | Measure-Object -Property PrivateMB -Average).Average; $method = $ss[0].Method }
        if ($null -eq $avgPws) { Set-Result 'P1' 'Not run' '' ('private working set not logged by this build.' + $crashNote) }
        else {
            $st = 'Fail'; if ($avgPws -le 150 -and -not $script:IdleCrashed) { $st = 'Pass' }
            Set-Result 'P1' $st ("{0} MB private working set (mean, app log); info: private bytes {1} MB, working set {2} MB" -f (F1 $avgPws), (F1 $appPriv), (F1 $appWs)) ("Window {0}; {1} app-log rows ({2} with private_ws_mb). Sampler (info only, not scored): {3} MB over {4} samples ({5}).{6}" -f $win, $mt.Count, $pwsRows.Count, (F1 $smpPriv), $ss.Count, $method, $crashNote)
        }

        # P2
        if ($mt.Count -eq 0) { Set-Result 'P2' 'Not run' '' ('No app metrics rows in window.' + $crashNote) }
        else {
            $mean = ($mt | Measure-Object -Property AppCpu -Average).Average
            $maxRoll = $null; $lo = 0
            for ($i = 0; $i -lt $mt.Count; $i++) {
                while ($mt[$i].Ms - $mt[$lo].Ms -gt 60000) { $lo++ }
                if (($mt[$i].Ms - $mt[$lo].Ms) -ge 55000) {
                    $sum = 0.0; for ($j = $lo; $j -le $i; $j++) { $sum += $mt[$j].AppCpu }
                    $avg = $sum / ($i - $lo + 1)
                    if ($null -eq $maxRoll -or $avg -gt $maxRoll) { $maxRoll = $avg }
                }
            }
            if ($null -eq $maxRoll) { $maxRoll = $mean }
            $st = 'Fail'; if ($mean -le 2 -and $maxRoll -le 5 -and -not $script:IdleCrashed) { $st = 'Pass' }
            Set-Result 'P2' $st ("mean {0}%, worst 60 s avg {1}% of machine" -f (F1 $mean), (F1 $maxRoll)) ("Window {0}; {1} app-log rows.{2}" -f $win, $mt.Count, $crashNote)
        }
    }

    # ============================================================ STEP 5  P3 / P4
    Invoke-Phase 'P3/P4 Focus and click response' @('P3', 'P4') {
        if ($SkipFocus) { Set-Result 'P3' 'Not run' '' 'Skipped (-SkipFocus)'; Set-Result 'P4' 'Not run' '' 'Skipped (-SkipFocus)'; return }
        if (-not (Ensure-App)) { Set-Result 'P3' 'Not run' '' 'App not running'; Set-Result 'P4' 'Not run' '' 'App not running'; return }
        $run = $script:Run
        $baseClicks = @(Get-RunEvents $run | Where-Object { $_.Kind -eq 'click' }).Count

        $before = @(Get-Process -Name notepad -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
        [void](Start-Process -FilePath 'notepad.exe')
        $np = $null; $hNp = [IntPtr]::Zero
        for ($i = 0; $i -lt 50 -and $hNp -eq [IntPtr]::Zero; $i++) {
            Start-Sleep -Milliseconds 200
            $np = Get-Process -Name notepad -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
            if ($np) { $hNp = $np.MainWindowHandle }
        }
        if ($hNp -eq [IntPtr]::Zero) { $hNp = [HYNative]::FindWindow('Notepad', [NullString]::Value) }
        if ($hNp -eq [IntPtr]::Zero) { Set-Result 'P3' 'Not run' '' 'Notepad window not found'; Set-Result 'P4' 'Not run' '' 'Notepad window not found'; return }
        $focused = $false
        for ($i = 0; $i -lt 5 -and -not $focused; $i++) { Set-Foreground $hNp; Start-Sleep -Milliseconds 400; $focused = Test-NotepadForeground $hNp }
        if (-not $focused) {
            Get-Process -Name notepad -ErrorAction SilentlyContinue | Where-Object { $before -notcontains $_.Id } | Stop-Process -Force -ErrorAction SilentlyContinue
            Set-Result 'P3' 'Not run' '' 'Could not bring Notepad to the foreground (Windows focus lock)'
            Set-Result 'P4' 'Not run' '' 'Depends on P3 clicks'
            return
        }
        [Windows.Forms.SendKeys]::SendWait('abc')
        $trials = @(); $stay = 0
        for ($n = 1; $n -le 10; $n++) {
            $hp = [HYNative]::FindWindow([NullString]::Value, 'HY Pet')
            if ($hp -eq [IntPtr]::Zero) { $trials += [pscustomobject]@{ Trial = $n; StayedInNotepad = $false; ForegroundTitle = 'HY Pet window not found' }; continue }
            $r = New-Object HYNative+RECT
            [void][HYNative]::GetWindowRect($hp, [ref]$r)
            $cx = [int](($r.Left + $r.Right) / 2); $cy = [int](($r.Top + $r.Bottom) / 2)
            [void][HYNative]::SetCursorPos($cx, $cy)
            Start-Sleep -Milliseconds 150
            [HYNative]::MouseDown(); Start-Sleep -Milliseconds 50; [HYNative]::MouseUp()
            Start-Sleep -Milliseconds 600
            $ok = Test-NotepadForeground $hNp
            $title = [HYNative]::Title([HYNative]::GetForegroundWindow())
            if ($ok) { $stay++ }
            [Windows.Forms.SendKeys]::SendWait('x')
            $trials += [pscustomobject]@{ Trial = $n; StayedInNotepad = $ok; ForegroundTitle = (Clean $title) }
            Log ("  click {0}/10 at ({1},{2}): focus {3}" -f $n, $cx, $cy, $(if ($ok) { 'stayed in Notepad' } else { 'LOST: ' + $title }))
            if (-not $ok) { Set-Foreground $hNp; Start-Sleep -Milliseconds 400 }
            Start-Sleep -Milliseconds 200
        }
        $trials | Export-Csv -Path (Join-Path $ResultsDir 'focus-trials.csv') -NoTypeInformation -Encoding UTF8
        Get-Process -Name notepad -ErrorAction SilentlyContinue | Where-Object { $before -notcontains $_.Id } | Stop-Process -Force -ErrorAction SilentlyContinue
        $st = 'Fail'; if ($stay -eq 10) { $st = 'Pass' }
        Set-Result 'P3' $st ("{0}/10 stayed in Notepad" -f $stay) 'Real mouse clicks (SendInput) on the centre of the HY Pet window'

        # P4: latency from the events log
        Start-Sleep -Milliseconds 1500
        $clicks = @(Get-RunEvents $run | Where-Object { $_.Kind -eq 'click' })
        $newClicks = $clicks.Count - $baseClicks
        $recent = @($clicks | Select-Object -Last 10)
        $lat = @($recent | ForEach-Object { Get-JsonNum $_.Json 'latency_ms' } | Where-Object { $null -ne $_ })
        if ($recent.Count -lt 10 -or $lat.Count -lt 10 -or $newClicks -lt 10) {
            Set-Result 'P4' 'Fail' ("{0} click events logged ({1} new, {2} with latency)" -f $clicks.Count, $newClicks, $lat.Count) 'Expected 10 new click events from the 10 test clicks'
        } else {
            $mx = ($lat | Measure-Object -Maximum).Maximum; $av = ($lat | Measure-Object -Average).Average
            $st = 'Fail'; if ($mx -le 500) { $st = 'Pass' }
            Set-Result 'P4' $st ("max {0} ms, mean {1} ms (n=10)" -f (F1 $mx), (F1 $av)) ('latencies: ' + (($lat | ForEach-Object { F1 $_ }) -join ', '))
        }
    }

    # ============================================================ STEP 6  C1-C3
    Invoke-Phase 'C1-C3 Camera load test' @('C1', 'C2', 'C3') {
        if ($SkipCamera) { foreach ($id in 'C1', 'C2', 'C3') { Set-Result $id 'Not run' '' 'Skipped (-SkipCamera)' }; return }
        if (-not $script:Exe) { foreach ($id in 'C1', 'C2', 'C3') { Set-Result $id 'Not run' '' 'App not installed' }; return }
        [void](Stop-App)
        Write-Host ''
        Write-Host '  CAMERA TEST' -ForegroundColor Yellow
        Write-Host ("  The camera will run for about {0} minutes, then switch on and off 5 times." -f $CameraMinutes) -ForegroundColor Yellow
        Write-Host '  Please watch the camera light during the 5 short on/off cycles at the end:' -ForegroundColor Yellow
        Write-Host '  count if it went off within about 2 seconds each time.' -ForegroundColor Yellow
        Write-Host '  Do not click or type in the HY app window during the camera test; it can press Stop.' -ForegroundColor Yellow
        Wait-Enter '  Press Enter to start the camera test'

        $run = Start-App 'camera' $CameraMinutes
        $limit = ($CameraMinutes + 5) * 60
        $expect = $CameraMinutes * 60 + 150
        $sw = [Diagnostics.Stopwatch]::StartNew()
        $nextSample = 0; $nextEv = 0; $alerted = $false
        $done = $false; $camErr = $null; $exited = $false
        while ($sw.Elapsed.TotalSeconds -lt $limit) {
            $t = $sw.Elapsed.TotalSeconds
            if ($t -ge $nextSample) {
                $s = Get-ProcSample
                if ($s) { Add-SampleRow 'camera' ([int]$t) $s }
                $nextSample += 15
            }
            if ($t -ge $nextEv) {
                $nextEv = $t + 3
                $ev = @(Get-RunEvents $run)
                if (@($ev | Where-Object { $_.Kind -eq 'autotest_done' }).Count -gt 0) { $done = $true; break }
                $ce = $ev | Where-Object { $_.Kind -eq 'camera_error' } | Select-Object -First 1
                if ($ce) { $camErr = $ce; break }
                if (-not $alerted -and @($ev | Where-Object { $_.Kind -eq 'camera_stopped' }).Count -ge 1) {
                    $alerted = $true
                    Write-Host ''
                    Write-Host '  >>> NOW: watch the camera light for the 5 short on/off cycles (about 2.5 minutes) <<<' -ForegroundColor Magenta
                    try { [Console]::Beep(880, 400) } catch {}
                }
            }
            if ($run.Proc.HasExited -and -not (Test-AppRunning)) { $exited = $true; break }
            Show-Countdown 'Camera test (approx)' ($expect - $t)
            Start-Sleep -Seconds 1
        }
        Write-Host ''
        $ev = @(Get-RunEvents $run)
        if (-not $camErr) { $camErr = $ev | Where-Object { $_.Kind -eq 'camera_error' } | Select-Object -First 1 }
        if ($camErr) {
            $nm = ''; if ($camErr.Json -match '"name"\s*:\s*"([^"]*)"') { $nm = $matches[1] }
            $script:CameraUnavailable = $true
            $why = "camera_error '{0}' (camera busy, missing or blocked). Close other camera apps and re-run." -f $nm
            foreach ($id in 'C1', 'C2', 'C3') { Set-Result $id 'Not run' ("camera_error " + $nm) $why }
            return
        }
        $started = @($ev | Where-Object { $_.Kind -eq 'camera_started' })
        $stopped = @($ev | Where-Object { $_.Kind -eq 'camera_stopped' })
        if ($started.Count -eq 0) {
            $why = 'No camera_started event. '; if ($exited) { $why += 'App exited. ' }; if (-not $done) { $why += 'autotest_done not seen within timeout.' }
            foreach ($id in 'C1', 'C2', 'C3') { Set-Result $id 'Fail' 'camera never started' $why }
            return
        }
        $wStart = $started[0].Ms
        $wEnd = $null
        foreach ($e in $stopped) { if ($e.Ms -gt $wStart) { $wEnd = $e.Ms; break } }
        if ($null -eq $wEnd) { $wEnd = Get-EpochMs }
        $mt = @(Get-RunMetrics $run | Where-Object { $_.Ms -ge $wStart -and $_.Ms -le $wEnd })
        $ss = @($script:Samples | Where-Object { $_.Phase -eq 'camera' -and $_.EpochMs -ge $wStart -and $_.EpochMs -le $wEnd })
        $durMin = [math]::Round(($wEnd - $wStart) / 60000, 1)

        # pair each camera_started after the first with the first camera_stopped that is later than it
        $closers = @{}
        $cycles = @()
        for ($k = 0; $k -lt $started.Count; $k++) {
            $cl = $null
            foreach ($e in $stopped) { if ($e.Ms -gt $started[$k].Ms) { $cl = $e; break } }
            if ($cl) { $closers[[string]$cl.Ms] = $true }
            if ($k -ge 1 -and $cycles.Count -lt 5 -and $cl) {
                $on = $started[$k].Ms; $off = $cl.Ms
                $cycles += [pscustomobject]@{ Cycle = $cycles.Count + 1; OnTime = (From-EpochMs $on).ToString('HH:mm:ss.fff'); OffTime = (From-EpochMs $off).ToString('HH:mm:ss.fff'); OnSeconds = [math]::Round(($off - $on) / 1000, 1); OnMs = $on; OffMs = $off }
            }
        }
        $extraStops = @($stopped | Where-Object { -not $closers.ContainsKey([string]$_.Ms) })
        $extraTxt = ''
        if ($extraStops.Count -gt 0) {
            $extraTxt = ' Unexpected/duplicate camera_stopped events (not closing a start): ' + (($extraStops | ForEach-Object { "{0} (+{1:0.0}s after launch)" -f (From-EpochMs $_.Ms).ToString('HH:mm:ss.fff'), (($_.Ms - $run.LaunchMs) / 1000) }) -join ', ') + '.'
        }

        # C1
        if ($mt.Count -eq 0) { Set-Result 'C1' 'Not run' '' 'No app metrics rows during first camera run' }
        else {
            $cpu = ($mt | Measure-Object -Property AppCpu -Average).Average
            $st = 'Fail'; if ($cpu -le 10) { $st = 'Pass' }
            $c1note = "First camera run {0} min, {1} app-log rows. Machine CPU mean {2}%." -f $durMin, $mt.Count, (F1 ($mt | Measure-Object -Property MachineCpu -Average).Average)
            if ((($wEnd - $wStart) / 60000) -lt (0.9 * $CameraMinutes)) {
                $c1note += " first run cut short at $durMin min (Stop pressed or app interrupted)."
                $shortRows = @(Get-RunMetrics $run | Where-Object { $m = $_; @($cycles | Where-Object { $m.Ms -ge $_.OnMs -and $m.Ms -le $_.OffMs }).Count -gt 0 })
                if ($shortRows.Count -gt 0) { $c1note += " Mean app CPU across the {0} short cycles: {1}% of machine ({2} rows)." -f $cycles.Count, (F1 ($shortRows | Measure-Object -Property AppCpu -Average).Average), $shortRows.Count }
                else { $c1note += " Mean app CPU across the short cycles: n/a (no rows)." }
            }
            Set-Result 'C1' $st ((F1 $cpu) + '% of machine (mean)') $c1note
        }
        # C2: scored on app-log private_ws_mb only (never on the Get-Counter sampler)
        $pwsRows = @($mt | Where-Object { $null -ne $_.PrivateWsMB })
        $pkP = $null; $pkW = $null
        if ($mt.Count -gt 0) { $pkP = ($mt | Measure-Object -Property PrivateMB -Maximum).Maximum; $pkW = ($mt | Measure-Object -Property WorkingMB -Maximum).Maximum }
        if ($pwsRows.Count -eq 0) { Set-Result 'C2' 'Not run' '' 'private working set not logged by this build' }
        else {
            $pkPws = ($pwsRows | Measure-Object -Property PrivateWsMB -Maximum).Maximum
            $st = 'Fail'; if ($pkPws -le 350) { $st = 'Pass' }
            $pkS = $null; $meth = ''
            if ($ss.Count -gt 0) { $pkS = ($ss | Measure-Object -Property PrivateMB -Maximum).Maximum; $meth = $ss[0].Method }
            Set-Result 'C2' $st ("{0} MB private working set (peak, app log); info: private bytes peak {1} MB, working set peak {2} MB" -f (F1 $pkPws), (F1 $pkP), (F1 $pkW)) ("{0} app-log rows with private_ws_mb. Sampler (info only, not scored): peak {1} MB over {2} samples ({3})." -f $pwsRows.Count, (F1 $pkS), $ss.Count, $meth)
        }
        # C3: software stop timestamps
        $cycles | Select-Object Cycle, OnTime, OffTime, OnSeconds | Export-Csv -Path (Join-Path $ResultsDir 'camera-cycles.csv') -NoTypeInformation -Encoding UTF8
        $cycTxt = ($cycles | ForEach-Object { "#{0} on {1} off {2} ({3}s)" -f $_.Cycle, $_.OnTime, $_.OffTime, $_.OnSeconds }) -join '; '
        if (-not $done) { Log 'autotest_done not seen before the timeout.' 'Yellow' }
        Write-Host ''
        $ans = Read-Host '  Did the light go off within 2 s every time? (y/n)'
        $ans = "$ans".Trim().ToLower()
        $lightSec = Read-Host '  Roughly how many seconds did the light stay on after each stop? (number, or Enter to skip)'
        $lightSec = "$lightSec".Trim()
        $lightTxt = 'not given'; if ($lightSec -ne '') { $lightTxt = "$lightSec s" }
        $note = "Software stop times: $cycTxt.$extraTxt autotest_done seen: $done. Owner answer: '$ans'. Owner light-on-after-stop estimate: $lightTxt."
        if ($cycles.Count -lt 5) { Set-Result 'C3' 'Fail' ("only {0}/5 cycles logged" -f $cycles.Count) $note }
        elseif ($ans -eq 'y' -or $ans -eq 'yes') { Set-Result 'C3' 'Pass' ("owner: light off within 2 s, 5/5; light stayed on after stop: $lightTxt") $note }
        elseif ($ans -eq 'n' -or $ans -eq 'no') { Set-Result 'C3' 'Fail' ("owner: light did NOT go off within 2 s every time; light stayed on after stop: $lightTxt") $note }
        else { Set-Result 'C3' 'Not run' 'no clear owner answer' $note }
    }

    # Windows' own record of camera use by this app (LastUsedTimeStart/Stop, FILETIME UTC). A stop
    # time close to the app's last camera_stopped means Windows saw the camera released.
    try {
        $np = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\webcam\NonPackaged'
        $lines = @()
        foreach ($k in @(Get-ChildItem $np -ErrorAction SilentlyContinue | Where-Object { $_.PSChildName -like '*HY Spike*' })) {
            $p = Get-ItemProperty $k.PSPath
            $st = ''; $sp = ''
            if ($p.LastUsedTimeStart) { $st = [DateTime]::FromFileTimeUtc([int64]$p.LastUsedTimeStart).ToLocalTime().ToString('HH:mm:ss.fff') }
            if ($p.LastUsedTimeStop) { $sp = [DateTime]::FromFileTimeUtc([int64]$p.LastUsedTimeStop).ToLocalTime().ToString('HH:mm:ss.fff') }
            $lines += ("{0}`tlast_start={1}`tlast_stop={2}" -f ($k.PSChildName -replace [regex]::Escape($env:USERNAME), '<user>'), $st, $sp)
        }
        if ($lines.Count -eq 0) { $lines = @('no Windows camera-usage record for HY Spike (capture may bypass the Windows privacy layer)') }
        Set-Content -Path (Join-Path $ResultsDir 'camera-usage.txt') -Value $lines -Encoding ASCII
        foreach ($l in $lines) { Log ("Windows camera-usage record: " + $l) }
    } catch { Log ("camera-usage record not readable: " + $_.Exception.Message) 'Yellow' }

    # ============================================================ STEP 7  C4
    Invoke-Phase 'C4 Camera permission denied' @('C4') {
        if ($SkipCamera) { Set-Result 'C4' 'Not run' '' 'Skipped (-SkipCamera)'; return }
        if (-not $script:Exe) { Set-Result 'C4' 'Not run' '' 'App not installed'; return }
        if ($script:CameraUnavailable) { Set-Result 'C4' 'Not run' '' 'Camera already failed in C1-C3 (busy/missing), so a denial cannot be told apart'; return }
        [void](Stop-App)
        Write-Host ''
        Write-Host '  NOTE: the Windows camera privacy setting will be switched OFF briefly for this test' -ForegroundColor Yellow
        Write-Host '  and then restored automatically.' -ForegroundColor Yellow

        $keyParent = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\webcam'
        $keyNP = $keyParent + '\NonPackaged'
        # snapshot both keys (parent first); restore in reverse order
        $states = @()
        foreach ($k in @($keyParent, $keyNP)) {
            $existed = Test-Path $k
            $origVal = $null; $valExisted = $false
            if ($existed) {
                $pr = Get-ItemProperty -Path $k -Name 'Value' -ErrorAction SilentlyContinue
                if ($pr -and ($pr.PSObject.Properties.Name -contains 'Value')) { $valExisted = $true; $origVal = $pr.Value }
            }
            # remember which keys do not exist yet, so we can remove them again
            $missing = @(); $probe = $k
            while ($probe -and -not (Test-Path $probe)) { $missing += $probe; $probe = Split-Path $probe -Parent }
            $states += [pscustomobject]@{ Key = $k; Existed = $existed; ValExisted = $valExisted; Orig = $origVal; Missing = $missing }
        }
        $backupTxt = Join-Path $ResultsDir 'camera-consent-backup.txt'
        $bk = @()
        foreach ($stt in $states) { $bk += ("key={0}`r`nkeyExisted={1}`r`nvalueExisted={2}`r`noriginal={3}" -f $stt.Key, $stt.Existed, $stt.ValExisted, $stt.Orig) }
        Set-Content -Path $backupTxt -Value ($bk -join "`r`n") -Encoding ASCII
        $res = $null
        try {
            foreach ($stt in $states) {
                if (-not $stt.Existed) { New-Item -Path $stt.Key -Force | Out-Null }
                Set-ItemProperty -Path $stt.Key -Name 'Value' -Value 'Deny' -Type String
            }
            Log 'Camera privacy set to Deny (webcam and webcam\NonPackaged).' 'Yellow'
            Start-Sleep -Seconds 2
            $run = Start-App 'camera_denied' 0
            $sw = [Diagnostics.Stopwatch]::StartNew()
            $done = $false; $petAfter = $false; $err = $null; $started = $null
            while ($sw.Elapsed.TotalSeconds -lt 90) {
                $ev = @(Get-RunEvents $run)
                $err = $ev | Where-Object { $_.Kind -eq 'camera_error' } | Select-Object -First 1
                $started = $ev | Where-Object { $_.Kind -eq 'camera_started' } | Select-Object -First 1
                $done = (@($ev | Where-Object { $_.Kind -eq 'autotest_done' }).Count -gt 0)
                if ($err) { $petAfter = (@($ev | Where-Object { $_.Kind -eq 'pet_frames' -and $_.Ms -ge $err.Ms }).Count -gt 0) }
                if ($done -and ($petAfter -or -not $err)) { break }
                if ($run.Proc.HasExited) { break }
                Show-Countdown 'Camera-denied test' (90 - $sw.Elapsed.TotalSeconds)
                Start-Sleep -Milliseconds 1000
            }
            Write-Host ''
            $alive = Test-AppRunning
            $nm = ''; if ($err -and $err.Json -match '"name"\s*:\s*"([^"]*)"') { $nm = $matches[1] }
            $res = [pscustomobject]@{ Err = $err; Started = $started; Done = $done; PetAfter = $petAfter; Alive = $alive; Name = $nm }
        }
        finally {
            # ALWAYS restore the original camera privacy settings (NonPackaged child first, then parent)
            $restoreOk = $true
            for ($ri = $states.Count - 1; $ri -ge 0; $ri--) {
                $stt = $states[$ri]
                try {
                    if ($stt.ValExisted) { Set-ItemProperty -Path $stt.Key -Name 'Value' -Value $stt.Orig -Type String }
                    elseif ($stt.Existed) { Remove-ItemProperty -Path $stt.Key -Name 'Value' -ErrorAction SilentlyContinue }
                    elseif ($stt.Missing.Count -gt 0) { Remove-Item -Path $stt.Missing[$stt.Missing.Count - 1] -Recurse -Force -ErrorAction SilentlyContinue }
                } catch {
                    $restoreOk = $false
                    Log ("COULD NOT RESTORE camera privacy key {0}: {1}. Original value was '{2}'. Windows Settings > Privacy > Camera." -f $stt.Key, $_.Exception.Message, $stt.Orig) 'Red'
                }
            }
            if ($restoreOk) {
                Log 'Camera privacy setting restored.' 'Green'
                Write-Host '  The camera privacy setting was switched off briefly and has been restored.' -ForegroundColor Green
            }
        }
        if (-not $res) { Set-Result 'C4' 'Not run' '' 'Test did not complete'; return }
        $note = "camera_error name '{0}'; autotest_done {1}; pet_frames after error {2}; app running at end {3}" -f $res.Name, $res.Done, $res.PetAfter, $res.Alive
        if ($res.Started -and -not $res.Err) { Set-Result 'C4' 'Fail' 'camera started despite Deny on webcam and NonPackaged' $note }
        elseif ($res.Err -and $res.Alive -and $res.PetAfter) { Set-Result 'C4' 'Pass' 'camera_error logged, pet still running' $note }
        else { Set-Result 'C4' 'Fail' $(if ($res.Err) { 'camera_error logged but pet/app not confirmed' } else { 'no camera_error logged' }) $note }
    }

    # ============================================================ STEP 8  I3
    Invoke-Phase 'I3 Silent uninstall' @('I3') {
        if (-not $script:Exe) { Set-Result 'I3' 'Not run' '' 'App was not installed'; return }
        $g = Stop-App
        Log ("App closed (graceful: {0})." -f $g)
        $u = Invoke-Uninstall
        if (-not $u.Ran) { Set-Result 'I3' 'Fail' 'uninstaller not found' ("UninstallString/uninstall.exe missing: " + $u.Path); return }
        $regGone = -not (Find-UninstallEntry)
        $dirGone = -not (Test-Path $script:InstallDir)
        $st = 'Fail'; if ($u.Gone) { $st = 'Pass' }
        Set-Result 'I3' $st ("exe {0} after {1} s" -f $(if ($u.Gone) { 'removed' } else { 'STILL PRESENT' }), $u.Seconds) ("Uninstall registry entry removed: {0}; install folder removed: {1}. Log folder intentionally kept." -f $regGone, $dirGone)
    }
}
finally {
    # safety net: never leave the app, helper apps or the sleep block behind
    try { Stop-IdleApps } catch {}
    try { Get-Process -Name 'hy-spike-batch1' -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue } catch {}
    Remove-Item Env:HY_AUTOTEST -ErrorAction SilentlyContinue
    Remove-Item Env:HY_CAMERA_MINUTES -ErrorAction SilentlyContinue
    try { [void][HYNative]::SetThreadExecutionState([uint32]2147483648) } catch {}
}

# ============================================================ STEP 9  results
Invoke-Phase 'Write results and zip' @() {
    # copy the app logs, then strip the user name from the copies
    $dest = Join-Path $ResultsDir 'logs'
    if (Test-Path $LogDir) {
        Copy-Item -Path $LogDir -Destination $dest -Recurse -Force
        $utf8 = New-Object System.Text.UTF8Encoding($false)
        foreach ($f in @(Get-ChildItem -Path $dest -Recurse -File -ErrorAction SilentlyContinue)) {
            if ($f.Extension -in '.txt', '.tsv', '.csv', '.json', '.log' -and $f.Length -lt 20MB) {
                try { [IO.File]::WriteAllText($f.FullName, (Clean ([IO.File]::ReadAllText($f.FullName))), $utf8) } catch {}
            }
        }
        Log 'App logs copied.'
    } else { Log 'No app log folder found.' 'Yellow' }

    $sys = $script:Sys
    $sysLines = @($sys.Keys | Sort-Object | ForEach-Object { "{0}: {1}" -f $_, $sys[$_] })
    Set-Content -Path (Join-Path $ResultsDir 'system-info.txt') -Value (Clean ($sysLines -join "`r`n")) -Encoding UTF8
    $script:PhaseTimes | Export-Csv -Path (Join-Path $ResultsDir 'phase-times.csv') -NoTypeInformation -Encoding UTF8

    $rows = @($script:Results | ForEach-Object { [pscustomobject]@{ Test = ($_.Id + ' ' + $_.Test); Measured = (Clean $_.Measured); Target = $_.Target; Result = $_.Status; Note = (Clean $_.Note) } })
    $rows | Export-Csv -Path (Join-Path $ResultsDir 'results.csv') -NoTypeInformation -Encoding UTF8

    $md = New-Object System.Collections.ArrayList
    [void]$md.Add('# HY Spike Batch 1 - test results')
    [void]$md.Add('')
    if ($Quick) { [void]$md.Add('**QUICK DRY RUN (idle 3 min, camera 2 min) - not valid for a final verdict.**'); [void]$md.Add('') }
    foreach ($l in $sysLines) { [void]$md.Add('- ' + (Clean $l)) }
    [void]$md.Add(('- Idle minutes: {0}; camera minutes: {1}' -f $IdleMinutes, $CameraMinutes))
    if ($script:OnlineSkipped) { [void]$md.Add('- **WARNING: test was run ONLINE (owner typed SKIP).**') }
    if ($script:Failures.Count -gt 0) { [void]$md.Add('- Phase errors: ' + (($script:Failures | ForEach-Object { Clean $_ }) -join ' | ')) }
    [void]$md.Add('')
    [void]$md.Add('| Test | Measured | Target | Result | Note |')
    [void]$md.Add('|---|---|---|---|---|')
    foreach ($r in $rows) {
        $cells = @($r.Test, $r.Measured, $r.Target, $r.Result, $r.Note) | ForEach-Object { ("$_" -replace '\|', '/') -replace "[\r\n]+", ' ' }
        [void]$md.Add('| ' + ($cells -join ' | ') + ' |')
    }
    Set-Content -Path (Join-Path $ResultsDir 'results.md') -Value $md -Encoding UTF8

    # on-screen table
    Write-Host ''
    Write-Host '==================== RESULTS ====================' -ForegroundColor Cyan
    foreach ($r in $script:Results) {
        $col = 'Yellow'
        if ($r.Status -eq 'Pass') { $col = 'Green' } elseif ($r.Status -eq 'Fail') { $col = 'Red' } elseif ($r.Status -eq 'Info') { $col = 'Cyan' }
        $meas = Clean $r.Measured; if ($meas.Length -gt 60) { $meas = $meas.Substring(0, 57) + '...' }
        Write-Host ("{0,-4} {1,-8} {2,-60} (target: {3})" -f $r.Id, $r.Status, $meas, $r.Target) -ForegroundColor $col
        if ($r.Note) { $n = Clean $r.Note; if ($n.Length -gt 110) { $n = $n.Substring(0, 107) + '...' }; Write-Host ('       ' + $n) -ForegroundColor DarkGray }
    }

    # zip to the Desktop
    $zip = $ResultsDir + '.zip'
    if (Test-Path $zip) { Remove-Item $zip -Force }
    try {
        Add-Type -AssemblyName System.IO.Compression.FileSystem
        [IO.Compression.ZipFile]::CreateFromDirectory($ResultsDir, $zip, [IO.Compression.CompressionLevel]::Optimal, $true)
    } catch { Compress-Archive -Path $ResultsDir -DestinationPath $zip -Force }
    Write-Host ''
    if (Test-Path $zip) {
        Write-Host '  DONE. Please attach this file in the project thread:' -ForegroundColor Green
        Write-Host ('  ' + $zip) -ForegroundColor Green
    } else { Write-Host ('  Zip could not be created. Please zip this folder by hand: ' + $ResultsDir) -ForegroundColor Red }
}
