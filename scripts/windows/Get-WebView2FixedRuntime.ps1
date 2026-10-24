<#
.SYNOPSIS
  Fetch the official fixed-version WebView2 runtime (x64) for the companion installer.

.DESCRIPTION
  Slice V1-6 (architecture 7 and 8: the installer bundles a fixed WebView2 runtime, so
  the companion installs and runs offline with no administrator rights). The same method
  as feasibility batch 1: the newest x64 fixed-version cab linked from Microsoft's
  WebView2 page, accepted only if its Authenticode signature is valid and from Microsoft
  Corporation, then expanded into apps/companion/src-tauri/webview2-fixed (git-ignored).
  Writes the version and the cab's SHA-256 to -InfoFile.
#>
[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)][string]$Destination,
  [Parameter(Mandatory = $true)][string]$InfoFile,
  [string]$WorkDir = $env:RUNNER_TEMP
)
$ErrorActionPreference = 'Stop'
if (-not $WorkDir) { $WorkDir = [IO.Path]::GetTempPath() }

$page = (Invoke-WebRequest -UseBasicParsing 'https://developer.microsoft.com/en-us/microsoft-edge/webview2/').Content
# The links sit in the page's embedded JSON with escaped slashes.
$page = $page -replace '\\u002F', '/'
$urls = [regex]::Matches($page, 'https://[^"''\s<>]+?FixedVersionRuntime\.[0-9.]+\.x64\.cab') | ForEach-Object Value | Select-Object -Unique
if (-not $urls) { throw 'No fixed-version runtime link was found on the WebView2 page.' }
$pick = $urls |
  ForEach-Object { [pscustomobject]@{ Url = $_; Ver = [version]([regex]::Match($_, 'FixedVersionRuntime\.([0-9.]+)\.x64').Groups[1].Value) } } |
  Sort-Object Ver -Descending | Select-Object -First 1
Write-Host "WebView2 fixed runtime $($pick.Ver)"

$cab = Join-Path $WorkDir 'webview2-fixed.cab'
Invoke-WebRequest -UseBasicParsing $pick.Url -OutFile $cab
$sig = Get-AuthenticodeSignature $cab
if ($sig.Status -ne 'Valid' -or $sig.SignerCertificate.Subject -notmatch 'O=Microsoft Corporation') {
  throw "The runtime cab is not validly signed by Microsoft ($($sig.Status))."
}
$expand = Join-Path $WorkDir 'webview2-expand'
if (Test-Path $expand) { Remove-Item -Recurse -Force $expand }
New-Item -ItemType Directory -Force $expand | Out-Null
expand.exe $cab -F:* $expand | Out-Null
$inner = Get-ChildItem $expand -Directory | Select-Object -First 1
if (-not $inner -or -not (Test-Path (Join-Path $inner.FullName 'msedgewebview2.exe'))) { throw 'msedgewebview2.exe is missing from the runtime.' }
if (Test-Path $Destination) { Remove-Item -Recurse -Force $Destination }
Move-Item $inner.FullName $Destination
$mb = [math]::Round(((Get-ChildItem -Recurse -File $Destination | Measure-Object Length -Sum).Sum) / 1MB, 1)
@(
  "webview2_fixed_version=$($pick.Ver)",
  "webview2_cab_sha256=$((Get-FileHash $cab -Algorithm SHA256).Hash)",
  "webview2_cab_signer=Microsoft Corporation",
  "webview2_expanded_mb=$mb"
) | Out-File -Append -Encoding utf8 $InfoFile
Remove-Item -Force $cab
Write-Host "Runtime expanded: $mb MB"
