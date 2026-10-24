<#
.SYNOPSIS
  Guided checks of the HealthierYou companion installer on a real office PC (slice V1-6,
  "needs physical testing"). CI covers what a runner can; this covers what it cannot:
  SmartScreen and Smart App Control, a standard (non-admin) user, display scaling,
  start at sign-in after a real sign-in, and Windows Settings' uninstall.

.DESCRIPTION
  Run it in Windows PowerShell 5.1 or PowerShell 7 as the office user (no "Run as
  administrator"), with the installer and its .sig copied to the PC the way staff will
  get it. It automates the checks it can and asks a yes/no question for each thing only
  a person can see. It writes a report (text) with the PC's Windows build, the result of
  every check and your notes: no user name, PC name, folder path or serial number. Read
  the report before sharing it.

  Steps (each can be skipped):
    1. Environment: Windows build, admin or not, Smart App Control state, display scale.
    2. Install from a downloaded copy (browser) and from a LAN/USB copy: what Windows shows.
    3. After install: per-user entry only, files, start at sign-in, the pet at this scale.
    4. Sign out and in (you do it; run with -Phase AfterSignIn afterwards).
    5. Update from file in App & updates, then roll back from file.
    6. Uninstall from Windows Settings, keeping data; then reinstall; uninstall deleting data.

.EXAMPLE
  .\Test-HealthierYou-OfficePC.ps1 -Installer .\HealthierYou_0.1.0_x64-setup.exe -Report .\hy-office-report.txt
  .\Test-HealthierYou-OfficePC.ps1 -Phase AfterSignIn -Report .\hy-office-report.txt
#>
[CmdletBinding()]
param(
  [ValidateSet('Install', 'AfterSignIn', 'Uninstall')][string]$Phase = 'Install',
  [string]$Installer,
  # A newer release (and its .sig) for the update step, optional.
  [string]$NewerInstaller,
  [Parameter(Mandatory = $true)][string]$Report
)
$ErrorActionPreference = 'Stop'

$Product = 'HealthierYou'
$Root = Join-Path $env:LOCALAPPDATA $Product
$Exe = Join-Path $Root 'hy-companion.exe'
$UninstallKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$Product"
$MachineKey = "HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$Product"
$RunKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$ApprovedKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run'

$lines = [System.Collections.Generic.List[string]]::new()
function Add-Line([string]$Text) { $lines.Add($Text); Write-Host $Text }
function Result([string]$Name, [string]$Outcome, [string]$Note = '') {
  # Never write paths or names: strip anything that looks like a user folder.
  $clean = $Note -replace '[A-Za-z]:\\[^\s"]*', '<path>' -replace [regex]::Escape($env:USERNAME), '<user>' -replace [regex]::Escape($env:COMPUTERNAME), '<pc>'
  Add-Line ("[{0}] {1}{2}" -f $Outcome, $Name, $(if ($clean) { " - $clean" } else { '' }))
}
function Auto([string]$Name, [bool]$Ok, [string]$Note = '') { Result $Name $(if ($Ok) { 'PASS' } else { 'FAIL' }) $Note }
function Ask([string]$Name, [string]$Question) {
  Write-Host ''
  Write-Host $Question -ForegroundColor Cyan
  $a = Read-Host '  y = yes, n = no, s = skip'
  $outcome = switch -Regex ($a) { '^[yY]' { 'PASS' } '^[nN]' { 'FAIL' } default { 'SKIP' } }
  $note = ''
  if ($outcome -eq 'FAIL') { $note = Read-Host '  What did you see? (no names or paths)' }
  Result $Name $outcome $note
}
function RegValue([string]$Key, [string]$Name) {
  $p = Get-ItemProperty -Path $Key -Name $Name -ErrorAction SilentlyContinue
  if ($p) { $p.$Name } else { $null }
}
function Scale-Percent {
  try {
    Add-Type -Namespace HyTest -Name Dpi -MemberDefinition '[DllImport("user32.dll")] public static extern uint GetDpiForSystem();' -ErrorAction SilentlyContinue
    [math]::Round([HyTest.Dpi]::GetDpiForSystem() / 96 * 100)
  } catch { 'unknown' }
}

Add-Line "HealthierYou office PC check - phase $Phase - $((Get-Date).ToString('yyyy-MM-dd HH:mm'))"
$os = Get-CimInstance Win32_OperatingSystem
Add-Line ("Windows: {0} build {1}" -f $os.Caption, $os.BuildNumber)
$admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
Add-Line "Running elevated: $admin (should be False: staff install without admin rights)"
$sac = RegValue 'HKLM:\SYSTEM\CurrentControlSet\Control\CI\Policy' 'VerifiedAndReputablePolicyState'
$sacText = switch ($sac) { 0 { 'off' } 1 { 'on (enforcing)' } 2 { 'evaluation' } default { 'not present' } }
Add-Line "Smart App Control: $sacText"
Add-Line "System display scale: $(Scale-Percent)%"
Add-Line ''

if ($Phase -eq 'Install') {
  if (-not $Installer -or -not (Test-Path $Installer)) { throw 'Pass -Installer with the setup .exe' }
  $zone = Get-Content -Path $Installer -Stream 'Zone.Identifier' -ErrorAction SilentlyContinue
  Add-Line "Installer copy marked as downloaded (Mark of the Web): $([bool]($zone -match 'ZoneId=3'))"
  Auto 'signature file next to the installer' (Test-Path ($Installer + '.sig'))

  Write-Host "`nDouble-click the installer in File Explorer now (do not run it from this window)." -ForegroundColor Yellow
  Write-Host 'Expected for a downloaded copy: "Windows protected your PC" (SmartScreen). Click More info:'
  Write-Host 'it shows "Publisher: Unknown publisher". Click Run anyway. A LAN or USB copy may show'
  Write-Host '"Open File - Security Warning" instead, or nothing.'
  Ask 'SmartScreen: "Windows protected your PC" with More info > Run anyway (downloaded copy)' 'Did Windows show "Windows protected your PC" with a "Run anyway" button after "More info"? (n if it showed no way to continue, or something else)'
  Ask 'Smart App Control or policy did not block the installer' 'Could you get past every warning? (n if it said "Smart App Control blocked" or "Your organization" blocked it: that is a signing decision, do not work around it)'
  Ask 'no administrator prompt (UAC) during install' 'Did the install finish without asking for an administrator password or a Yes on a UAC prompt?'
  Read-Host "`nWhen the installer has finished, press Enter"

  Auto 'installed in the user''s own folder' (Test-Path $Exe)
  Auto 'per-user uninstall entry' (Test-Path $UninstallKey) "version $(RegValue $UninstallKey 'DisplayVersion')"
  Auto 'no machine-wide uninstall entry' (-not (Test-Path $MachineKey))
  Auto 'bundled WebView2 runtime installed' (Test-Path (Join-Path $Root 'webview2-fixed\msedgewebview2.exe'))
  Auto 'start at sign-in set' ($null -ne (RegValue $RunKey $Product))
  $size = (Get-ChildItem -Recurse -File $Root -ErrorAction SilentlyContinue | Measure-Object Length -Sum).Sum
  Add-Line ("Installed size: {0} MB (A16: about 180 MB)" -f [math]::Round($size / 1MB))

  Ask 'the pet appears above the taskbar, sharp (not blurry)' 'Is the pet visible near the clock and sharp at this display scale?'
  Ask 'Settings opens from the tray icon and its text is sharp and fits' 'Right-click the tray icon > Settings. Is the text sharp and nothing cut off?'
  Write-Host "`nNow change the display scale (Settings > System > Display > Scale) to each of 100%, 125%, 150%, 200% that this screen offers. Sign out and in only if Windows asks."
  foreach ($pct in 100, 125, 150, 200) {
    Ask "scale ${pct}%: pet and Settings sharp, placed above the taskbar" "At $pct%: is the pet sharp, fully on screen above the taskbar, and Settings readable? (s if this scale is not offered)"
  }
  Ask 'two monitors with different scales (if available)' 'If you have two monitors with different scales: move Settings between them. Does it stay sharp and the right size? (s if one monitor)'
  if ($NewerInstaller -and (Test-Path $NewerInstaller)) {
    Write-Host "`nOpen Settings > App & updates > Choose update file and pick the newer installer."
    Ask 'update from file: notice shown, Install and restart works' 'Did it show the new version, then close, install and start the new version within about two minutes, with "Updated to ..." on App & updates?'
    Ask 'roll back from file' 'Now choose the older installer in App & updates. Did it offer "Go back to ..." and return to the older version?'
  }
  Write-Host "`nNext: sign out of Windows and sign in again, then run this script with -Phase AfterSignIn." -ForegroundColor Yellow
}
elseif ($Phase -eq 'AfterSignIn') {
  $running = @(Get-Process -Name 'hy-companion' -ErrorAction SilentlyContinue).Count -gt 0
  Auto 'started at sign-in' $running
  $approved = RegValue $ApprovedKey $Product
  if ($approved) { Add-Line "Task Manager start-up state: $(if ($approved[0] % 2 -eq 1) { 'disabled' } else { 'enabled' })" }
  Ask 'start-up is allowed by office policy (A15)' 'Is HealthierYou listed as Enabled in Task Manager > Startup apps, and was it running after sign-in without you starting it?'
  Ask 'no network prompt or firewall prompt' 'Did Windows ask nothing about networks or the firewall when HealthierYou started? (the companion only connects out)'
  Ask 'sleep and wake: the pet is back after the PC wakes' 'Put the PC to sleep and wake it. Is the pet back in place?'
}
else {
  Write-Host "`nOpen Windows Settings > Apps > Installed apps, find HealthierYou and click Uninstall." -ForegroundColor Yellow
  Ask 'uninstall: the "Also delete my data" box is shown unticked with the folder' 'Did the uninstaller show "Also delete my data: ...\HealthierYou\data" unticked? Leave it unticked and finish.'
  Read-Host 'When the uninstaller has finished, press Enter'
  Start-Sleep -Seconds 5
  Auto 'uninstall entry gone' (-not (Test-Path $UninstallKey))
  Auto 'no HKCU\Software\HealthierYou key left' (-not (Test-Path "HKCU:\Software\$Product"))
  Auto 'start at sign-in removed' ($null -eq (RegValue $RunKey $Product))
  Auto 'Task Manager start-up record removed' ($null -eq (RegValue $ApprovedKey $Product))
  Auto 'program files removed' (-not (Test-Path $Exe) -and -not (Test-Path (Join-Path $Root 'webview2-fixed')))
  Auto 'personal data kept' (Test-Path (Join-Path $Root 'data'))
  Ask 'no HealthierYou entry left in Installed apps' 'Refresh Installed apps. Is HealthierYou gone from the list?'
}

Add-Line ''
Add-Line 'End of report. It contains no user name, PC name or folder path; check before sharing.'
$lines | Set-Content -Encoding UTF8 $Report
Write-Host "`nReport written."
