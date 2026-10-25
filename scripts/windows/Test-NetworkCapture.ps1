<#
.SYNOPSIS
  Network capture of a HealthierYou companion PC during its sync with the office server
  (slice V1-7; V1-5 contract test T-S23; architecture 9 and 11, assumption A12).

.DESCRIPTION
  Run it on a companion PC in an elevated PowerShell (Windows PowerShell 5.1 or
  PowerShell 7): pktmon and the Windows Filtering Platform audit need an administrator.
  The companion itself keeps running as the signed-in office user.

  It walks through the T-S23 phases, one capture each, and tells you what to do in each:

    BeforePairing  the companion is installed but not paired: it must make no connection.
    Pending        paired, not yet confirmed by the administrator.
    PairedOff      confirmed, every sharing category off.
    CategoryOn     one category (Water is the one V1 fills most) switched on, water logged.
    AfterDisable   the category switched off again, water logged again.

  For each phase it records every packet on the network adapters with pktmon (converted
  to pcapng) and, at the same time, every connection Windows allows or blocks with the
  name of the program that made it (Filtering Platform audit, Security log events 5156
  and 5157). Then it checks:

    1. The HealthierYou programs (hy-companion.exe and its bundled msedgewebview2.exe)
       connect to nothing but the office server on its port (TCP 8443 by default): no
       other LAN host, no internet address (A12: WebView2 makes no outbound connection).
    2. Every connection with the server is TLS 1.3 only: the ClientHello offers no other
       version, no early data and no resumption, and asks for ALPN hy/1; the ServerHello
       picks TLS 1.3; after the hellos only encrypted records cross.
    3. Nothing readable crosses: no HTTP request line, /v1/ path, header name or JSON key
       in any packet to or from the server, and the PC label you paired with (pass it as
       -Canary) in no packet at all, to any address.
    4. Before pairing, nothing at all goes to the server from this PC.

  Each phase is PASS, FAIL (a finding) or INCONCLUSIVE (evidence missing): PASS needs the
  same companion process running from start to end, the connection audit switched on and
  read back, the Security log read, audit events in the window, and in every phase after
  pairing the companion's own connection to the server and at least one complete TLS
  connection (both hellos, whole records, no missing segment). A connection with a gap,
  a cut-off record or one direction missing is checked as far as it goes but never counts
  as verified. Exit code 0 PASS, 1 FAIL, 2 INCONCLUSIVE.

  What a capture cannot see: inside TLS, which fields each phase sends. That part of
  T-S23 (operational data only with every category off, only the schema's fields when one
  is on, nothing for a category after it is switched off) is proven by the CI test
  crates/hy-server/tests/capture.rs on the real client and server, and this script says
  so in its report. docs/V1_Acceptance.md lists what each side proves.

  The connection audit policy is read (by its GUID and the numeric setting of auditpol
  /backup, the same in every Windows language) before anything is changed; if it cannot be
  read or is not recognised, the run stops INCONCLUSIVE and changes nothing. At the end it
  is set back and read back; if it is not exactly as it was, the run is never PASS and the
  report gives the command to restore it. The report (text and JSON) names
  addresses only by role (server, LAN, internet, this PC) and programs only by file name:
  no user name, PC name, path, IP address or device id. The .etl and .pcapng files stay in
  -Out and hold raw traffic of the whole PC: do not share them.

.EXAMPLE
  # On the companion PC, elevated. -Canary is the PC label typed when pairing.
  .\Test-NetworkCapture.ps1 -Server hy-server.office.local -Canary CAPTURE-PC-7F3A -Out C:\hy-capture

.EXAMPLE
  # Analyse a capture file again, or one made elsewhere (no administrator needed).
  .\Test-NetworkCapture.ps1 -Analyze .\phase-PairedOff.pcapng -ServerIp 192.168.1.20

.EXAMPLE
  # The analyser's own checks on synthetic captures (CI runs this).
  .\Test-NetworkCapture.ps1 -SelfTest
#>
[CmdletBinding(DefaultParameterSetName = 'Capture')]
param(
  [Parameter(ParameterSetName = 'Capture', Mandatory = $true)][string]$Server,
  [Parameter(ParameterSetName = 'Capture')][string]$Out = (Join-Path ([IO.Path]::GetTempPath()) 'hy-capture'),
  [Parameter(ParameterSetName = 'Capture')][ValidateSet('BeforePairing', 'Pending', 'PairedOff', 'CategoryOn', 'AfterDisable')][string[]]$Phases = @('BeforePairing', 'Pending', 'PairedOff', 'CategoryOn', 'AfterDisable'),
  [Parameter(ParameterSetName = 'Capture')][int]$Seconds = 0,
  [Parameter(ParameterSetName = 'Analyze', Mandatory = $true)][string]$Analyze,
  [Parameter(ParameterSetName = 'Analyze', Mandatory = $true)][string[]]$ServerIp,
  [Parameter(ParameterSetName = 'Analyze')][string]$Phase = 'PairedOff',
  [Parameter(ParameterSetName = 'SelfTest', Mandatory = $true)][switch]$SelfTest,
  [int]$ServerPort = 8443,
  # Text that must never appear in clear: the PC label used when pairing, the alias.
  [string[]]$Canary = @(),
  [string]$Report
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2
# -Canary 'a,b' (as powershell -File passes it) means two canaries.
$Canary = @($Canary | ForEach-Object { $_ -split ',' } | Where-Object { $_ })

# --------------------------------------------------------------------- byte helpers

function Get-U16BE([byte[]]$b, [int]$i) { return ([int]$b[$i] -shl 8) -bor $b[$i + 1] }
function Get-U32BE([byte[]]$b, [int]$i) { return ([uint32]$b[$i] -shl 24) -bor ([uint32]$b[$i + 1] -shl 16) -bor ([uint32]$b[$i + 2] -shl 8) -bor $b[$i + 3] }
function Get-U16([byte[]]$b, [int]$i, [bool]$le) { if ($le) { [BitConverter]::ToUInt16($b, $i) } else { [uint16](Get-U16BE $b $i) } }
function Get-U32([byte[]]$b, [int]$i, [bool]$le) { if ($le) { [BitConverter]::ToUInt32($b, $i) } else { [uint32](Get-U32BE $b $i) } }
$Latin1 = [System.Text.Encoding]::GetEncoding(28591)
function Get-Slice([byte[]]$b, [int]$start, [int]$len) {
  $out = New-Object byte[] $len
  [Array]::Copy($b, $start, $out, 0, $len)
  return , $out
}

# ------------------------------------------------------------------------- reading

# Packets from a pcapng (pktmon etl2pcap) or classic pcap file: link-layer frames.
function Read-CaptureFrames([string]$Path) {
  $b = [IO.File]::ReadAllBytes($Path)
  $frames = New-Object System.Collections.Generic.List[object]
  if ($b.Length -lt 24) { throw 'Not a capture file (too short)' }
  $magic = [BitConverter]::ToUInt32($b, 0)
  # Classic pcap magics, written in decimal (PowerShell reads 0xA1B2C3D4 as a negative
  # Int32): A1B2C3D4 and A1B23C4D (microsecond and nanosecond) in either byte order.
  $pcapLe = @(2712847316, 2712812621)
  $pcapBe = @(3569595041, 1295823521)
  if ($pcapLe -contains $magic -or $pcapBe -contains $magic) {
    $le = $pcapLe -contains $magic
    $link = Get-U32 $b 20 $le
    $at = 24
    while ($at + 16 -le $b.Length) {
      $cap = [int](Get-U32 $b ($at + 8) $le)
      if ($at + 16 + $cap -gt $b.Length) { break }
      $frames.Add([pscustomobject]@{ Link = [int]$link; Data = (Get-Slice $b ($at + 16) $cap) })
      $at += 16 + $cap
    }
    return , $frames
  }
  if ($magic -ne 0x0A0D0D0A) { throw 'Not a pcap or pcapng file' }
  $le = $true
  $links = New-Object System.Collections.Generic.List[int]
  $at = 0
  while ($at + 12 -le $b.Length) {
    $type = Get-U32 $b $at $le
    if ($type -eq 0x0A0D0D0A) {
      $le = ([BitConverter]::ToUInt32($b, $at + 8) -eq 0x1A2B3C4D)
      $links.Clear()
    }
    $len = [int](Get-U32 $b ($at + 4) $le)
    if ($len -lt 12 -or $at + $len -gt $b.Length) { break }
    if ($type -eq 1) { $links.Add([int](Get-U16 $b ($at + 8) $le)) }
    elseif ($type -eq 6) {
      $iface = [int](Get-U32 $b ($at + 8) $le)
      $cap = [int](Get-U32 $b ($at + 20) $le)
      $link = if ($iface -lt $links.Count) { $links[$iface] } else { 1 }
      $frames.Add([pscustomobject]@{ Link = $link; Data = (Get-Slice $b ($at + 28) $cap) })
    }
    elseif ($type -eq 3) {
      $cap = $len - 16
      $frames.Add([pscustomobject]@{ Link = $(if ($links.Count) { $links[0] } else { 1 }); Data = (Get-Slice $b ($at + 12) $cap) })
    }
    $at += $len
  }
  return , $frames
}

function Format-Ip([byte[]]$b, [int]$at, [int]$len) {
  if ($len -eq 4) { return ('{0}.{1}.{2}.{3}' -f $b[$at], $b[$at + 1], $b[$at + 2], $b[$at + 3]) }
  return ([System.Net.IPAddress]::new((Get-Slice $b $at 16))).ToString()
}

# One IP packet's transport fields, or $null for anything that is not IPv4/IPv6 TCP/UDP.
function Read-Packet($frame) {
  $d = $frame.Data
  $at = 0
  if ($frame.Link -eq 1) {
    if ($d.Length -lt 14) { return $null }
    $eth = Get-U16BE $d 12
    $at = 14
    while ($eth -eq 0x8100 -or $eth -eq 0x88A8) { $eth = Get-U16BE $d ($at + 2); $at += 4 }
    if ($eth -ne 0x0800 -and $eth -ne 0x86DD) { return $null }
  }
  elseif ($frame.Link -ne 101 -and $frame.Link -ne 228 -and $frame.Link -ne 229) { return $null }
  if ($at -ge $d.Length) { return $null }
  $ver = $d[$at] -shr 4
  if ($ver -eq 4) {
    $ihl = ($d[$at] -band 0x0F) * 4
    $proto = $d[$at + 9]
    $total = Get-U16BE $d ($at + 2)
    $src = Format-Ip $d ($at + 12) 4; $dst = Format-Ip $d ($at + 16) 4
    $l4 = $at + $ihl
    $end = [Math]::Min($d.Length, $at + $total)
  }
  elseif ($ver -eq 6) {
    $proto = $d[$at + 6]
    $src = Format-Ip $d ($at + 8) 16; $dst = Format-Ip $d ($at + 24) 16
    $l4 = $at + 40
    $end = [Math]::Min($d.Length, $l4 + (Get-U16BE $d ($at + 4)))
  }
  else { return $null }
  if ($proto -eq 6) {
    if ($l4 + 20 -gt $end) { return $null }
    $off = ($d[$l4 + 12] -shr 4) * 4
    $p = $l4 + $off
    $payload = if ($p -lt $end) { Get-Slice $d $p ($end - $p) } else { New-Object byte[] 0 }
    return [pscustomobject]@{ Proto = 'TCP'; Src = $src; Dst = $dst; SPort = (Get-U16BE $d $l4); DPort = (Get-U16BE $d ($l4 + 2)); Seq = (Get-U32BE $d ($l4 + 4)); Flags = $d[$l4 + 13]; Payload = $payload }
  }
  if ($proto -eq 17) {
    if ($l4 + 8 -gt $end) { return $null }
    $payload = if ($l4 + 8 -lt $end) { Get-Slice $d ($l4 + 8) ($end - $l4 - 8) } else { New-Object byte[] 0 }
    return [pscustomobject]@{ Proto = 'UDP'; Src = $src; Dst = $dst; SPort = (Get-U16BE $d $l4); DPort = (Get-U16BE $d ($l4 + 2)); Seq = [uint32]0; Flags = 0; Payload = $payload }
  }
  return $null
}

# One direction of a TCP flow put back in order by sequence number (duplicates and
# retransmissions dropped). Returns the bytes up to the first gap and whether one was seen.
function Join-TcpStream($segments) {
  $ms = New-Object IO.MemoryStream
  if ($segments.Count -eq 0) { return [pscustomobject]@{ Bytes = $ms.ToArray(); Gap = $false } }
  $base = [int64]$segments[0].Seq
  $syn = @($segments | Where-Object { $_.Flags -band 0x02 })
  if ($syn.Count) { $base = ([int64]$syn[0].Seq + 1) % 4294967296 }
  $data = @($segments | Where-Object { $_.Payload.Length -gt 0 } | ForEach-Object {
      [pscustomobject]@{ Rel = (([int64]$_.Seq - $base + 4294967296) % 4294967296); Payload = $_.Payload }
    } | Sort-Object Rel)
  if (-not $syn.Count -and $data.Count) { $base = 0; $min = ($data | Measure-Object Rel -Minimum).Minimum; foreach ($s in $data) { $s.Rel -= $min } }
  $next = [int64]0; $gap = $false
  foreach ($s in $data) {
    $len = $s.Payload.Length
    if ($s.Rel + $len -le $next) { continue }
    if ($s.Rel -gt $next) { $gap = $true; break }
    $skip = [int]($next - $s.Rel)
    $ms.Write($s.Payload, $skip, $len - $skip)
    $next = $s.Rel + $len
  }
  return [pscustomobject]@{ Bytes = $ms.ToArray(); Gap = $gap }
}

# ---------------------------------------------------------------------------- TLS

function Get-TlsRecords([byte[]]$s) {
  $out = New-Object System.Collections.Generic.List[object]
  $at = 0
  while ($at + 5 -le $s.Length) {
    $len = Get-U16BE $s ($at + 3)
    $end = [Math]::Min($s.Length, $at + 5 + $len)
    $out.Add([pscustomobject]@{ Type = [int]$s[$at]; Version = (Get-U16BE $s ($at + 1)); Body = (Get-Slice $s ($at + 5) ($end - $at - 5)) })
    $at += 5 + $len
  }
  return , $out
}

# Whether a stream is whole TLS records: the last record's declared length ends exactly at
# the last byte (a record cut off by the end of the capture is not evidence).
function Test-RecordsWhole([byte[]]$s) {
  $at = 0
  while ($at + 5 -le $s.Length) { $at += 5 + (Get-U16BE $s ($at + 3)) }
  return ($at -eq $s.Length)
}

function Get-HelloExtensions([byte[]]$m, [bool]$client) {
  $ext = @{}
  $at = 4 + 2 + 32
  if ($at -ge $m.Length) { return $ext }
  $at += 1 + $m[$at]
  if ($client) { $at += 2 + (Get-U16BE $m $at); $at += 1 + $m[$at] } else { $at += 3 }
  if ($at + 2 -gt $m.Length) { return $ext }
  $end = [Math]::Min($m.Length, $at + 2 + (Get-U16BE $m $at))
  $at += 2
  while ($at + 4 -le $end) {
    $t = Get-U16BE $m $at; $l = Get-U16BE $m ($at + 2)
    $ext[[int]$t] = Get-Slice $m ($at + 4) ([Math]::Min($l, $end - $at - 4))
    $at += 4 + $l
  }
  return $ext
}


# One TLS connection (client-to-server and server-to-client bytes): Problems are what the
# bytes show is wrong (FAIL); Missing is evidence the bytes do not hold (INCONCLUSIVE). Only
# a connection with neither is a verified TLS 1.3 connection: both hellos seen, whole
# records in both directions.
function Test-Tls13Only([byte[]]$up, [byte[]]$down) {
  $problems = New-Object System.Collections.Generic.List[string]
  $missing = New-Object System.Collections.Generic.List[string]
  $u = Get-TlsRecords $up; $d = Get-TlsRecords $down
  if ($up.Length -and -not (Test-RecordsWhole $up)) { $missing.Add('the last TLS record from the PC is cut off') }
  if ($down.Length -and -not (Test-RecordsWhole $down)) { $missing.Add('the last TLS record from the server is cut off') }
  if ($u.Count -eq 0) {
    $missing.Add('nothing from the PC in this connection (no ClientHello): the handshake is not proven')
    return [pscustomobject]@{ Problems = $problems; Missing = $missing }
  }
  $hellos = 0
  foreach ($r in $u) {
    if ($r.Type -eq 22 -and $r.Body.Length -gt 0 -and $r.Body[0] -eq 1) {
      $hellos++
      $e = Get-HelloExtensions $r.Body $true
      if (-not $e.ContainsKey(43)) { $problems.Add('the ClientHello has no supported_versions (TLS 1.2 or older)') }
      else {
        $v = $e[43]
        $offered = @(); for ($i = 1; $i + 1 -lt $v.Length; $i += 2) { $offered += '{0:x4}' -f (Get-U16BE $v $i) }
        if (($offered -join ',') -ne '0304') { $problems.Add("the ClientHello offers versions $($offered -join ', ') (only 0304, TLS 1.3, is allowed)") }
      }
      if ($e.ContainsKey(42)) { $problems.Add('the ClientHello offers early data (0-RTT)') }
      if ($e.ContainsKey(41)) { $problems.Add('the ClientHello offers resumption (pre-shared key)') }
      if (-not $e.ContainsKey(16) -or ($Latin1.GetString($e[16]).IndexOf('hy/1') -lt 0)) { $problems.Add('the ClientHello does not ask for ALPN hy/1') }
    }
    elseif ($r.Type -eq 20 -and $r.Body.Length -eq 1) { }
    elseif ($r.Type -eq 23) { }
    else { $problems.Add("the PC sent a clear TLS record of type $($r.Type)") }
  }
  if ($hellos -eq 0) { $problems.Add('no ClientHello from the PC') }
  $serverHello = $false
  foreach ($r in $d) {
    if (-not $serverHello -or ($r.Type -eq 22 -and $r.Body.Length -gt 37 -and $r.Body[0] -eq 2)) {
      if ($r.Type -ne 22 -or $r.Body.Length -lt 38 -or $r.Body[0] -ne 2) { $problems.Add("the server's first record is not a ServerHello (type $($r.Type))"); break }
      $e = Get-HelloExtensions $r.Body $false
      $v = if ($e.ContainsKey(43) -and $e[43].Length -eq 2) { '{0:x4}' -f (Get-U16BE $e[43] 0) } else { 'none (TLS 1.2 or older)' }
      if ($v -ne '0304') { $problems.Add("the server chose version $v") }
      $serverHello = $true
      continue
    }
    if (($r.Type -eq 20 -and $r.Body.Length -eq 1) -or $r.Type -eq 23) { continue }
    $problems.Add("the server sent a clear TLS record of type $($r.Type) after its hello")
  }
  if ($d.Count -eq 0) { $missing.Add('nothing from the server in this connection (no ServerHello): the handshake is not proven') }
  elseif (-not $serverHello) { $problems.Add('no ServerHello') }
  return [pscustomobject]@{ Problems = $problems; Missing = $missing }
}

# ----------------------------------------------------------------------- analysis

function Test-PrivateAddress([string]$ip) {
  $a = [System.Net.IPAddress]::Parse($ip)
  if ([System.Net.IPAddress]::IsLoopback($a)) { return 'loopback' }
  $b = $a.GetAddressBytes()
  if ($b.Length -eq 4) {
    if ($b[0] -eq 10 -or ($b[0] -eq 172 -and $b[1] -ge 16 -and $b[1] -le 31) -or ($b[0] -eq 192 -and $b[1] -eq 168) -or ($b[0] -eq 169 -and $b[1] -eq 254)) { return 'lan' }
    if ($b[0] -ge 224 -or ($b[0] -eq 0)) { return 'local' }
    return 'internet'
  }
  if ($a.IsIPv6LinkLocal -or $a.IsIPv6SiteLocal -or $a.IsIPv6Multicast -or (($b[0] -band 0xFE) -eq 0xFC)) { return 'lan' }
  return 'internet'
}

# Plain-text markers that must never cross the wire in clear between the PC and the
# server: protocol text and the user's own values.
$ProtocolNeedles = @('HTTP/1.1', 'HTTP/2', '/v1/', 'authorization', 'content-type', 'user-agent', '"ml"', 'hydration', '"category"', 'hy1-')

function Find-Needle([byte[]]$bytes, [string[]]$needles) {
  if ($bytes.Length -eq 0) { return $null }
  $text = $Latin1.GetString($bytes)
  foreach ($n in $needles) { if ($n -and $text.IndexOf($n, [StringComparison]::OrdinalIgnoreCase) -ge 0) { return $n } }
  return $null
}

# Analyse one capture: returns findings (FAIL), missing evidence (INCONCLUSIVE), notes and
# counts. Addresses are reported by role only.
function Invoke-CaptureAnalysis([string]$Path, [string[]]$ServerIps, [int]$Port, [string[]]$Canaries, [string]$PhaseName) {
  $fails = New-Object System.Collections.Generic.List[string]
  $missing = New-Object System.Collections.Generic.List[string]
  $notes = New-Object System.Collections.Generic.List[string]
  $frames = Read-CaptureFrames $Path
  $flows = @{}
  $internet = @{}
  $packets = 0; $serverPackets = 0
  foreach ($f in $frames) {
    $p = Read-Packet $f
    if (-not $p) { continue }
    $packets++
    $toServer = $ServerIps -contains $p.Dst
    $fromServer = $ServerIps -contains $p.Src
    if ($Canaries.Count) {
      $hit = Find-Needle $p.Payload $Canaries
      if ($hit) { $fails.Add("the canary text appears in clear in a $($p.Proto) packet ($(if ($toServer -or $fromServer) { 'with the server' } else { 'with another address' }))") }
    }
    foreach ($ip in @($p.Src, $p.Dst)) {
      if (-not ($ServerIps -contains $ip) -and (Test-PrivateAddress $ip) -eq 'internet') { $internet["$($p.Proto)"] = 1 + [int]$internet["$($p.Proto)"] }
    }
    if (-not ($toServer -or $fromServer)) { continue }
    $serverPackets++
    $serverPort = if ($toServer) { $p.DPort } else { $p.SPort }
    if ($p.Proto -ne 'TCP' -or $serverPort -ne $Port) {
      $fails.Add("traffic with the server on $($p.Proto) port $serverPort (only TCP $Port is expected)")
      continue
    }
    $pcPort = if ($toServer) { $p.SPort } else { $p.DPort }
    $key = "$pcPort"
    if (-not $flows.ContainsKey($key)) { $flows[$key] = [pscustomobject]@{ Up = New-Object System.Collections.Generic.List[object]; Down = New-Object System.Collections.Generic.List[object] } }
    if ($toServer) { $flows[$key].Up.Add($p) } else { $flows[$key].Down.Add($p) }
  }
  if ($PhaseName -eq 'BeforePairing' -and $serverPackets -gt 0) { $fails.Add("before pairing, $serverPackets packets went between this PC and the server") }
  $tlsOk = 0
  foreach ($k in $flows.Keys) {
    $up = Join-TcpStream $flows[$k].Up
    $down = Join-TcpStream $flows[$k].Down
    if ($up.Bytes.Length -eq 0 -and $down.Bytes.Length -eq 0) { continue }
    foreach ($side in @(@('from the PC', $up.Bytes), @('from the server', $down.Bytes))) {
      $n = Find-Needle $side[1] $ProtocolNeedles
      if ($n) { $fails.Add("readable protocol text ($n) $($side[0]) in clear") }
    }
    $t = Test-Tls13Only $up.Bytes $down.Bytes
    foreach ($x in $t.Problems) { $fails.Add("a connection with the server: $x") }
    foreach ($x in $t.Missing) { $missing.Add("a connection with the server: $x") }
    if ($up.Gap -or $down.Gap) { $missing.Add('a connection with the server was captured with missing segments: checked up to the gap, not counted as verified') }
    if (-not $t.Problems.Count -and -not $t.Missing.Count -and -not $up.Gap -and -not $down.Gap) { $tlsOk++ }
  }
  if ($packets -eq 0) { $missing.Add('the capture holds no packets at all') }
  if ($PhaseName -ne 'BeforePairing' -and $tlsOk -eq 0) { $missing.Add('no complete TLS connection with the server in this phase: the sync the phase asks for was not captured') }
  foreach ($proto in $internet.Keys) { $notes.Add("$($internet[$proto]) $proto packets of this PC with internet addresses (any program; the program audit says whether any was HealthierYou)") }
  return [pscustomobject]@{
    Phase = $PhaseName; Packets = $packets; ServerPackets = $serverPackets
    ServerConnections = $flows.Count; Tls13Connections = $tlsOk
    Fails = @($fails | Select-Object -Unique); Missing = @($missing | Select-Object -Unique); Notes = @($notes | Select-Object -Unique)
  }
}

# Connections the HealthierYou programs made, from Filtering Platform audit events
# (objects with Application, Direction, DestAddress, DestPort, Protocol, Allowed).
function Test-ProgramConnections($events, [string[]]$ServerIps, [int]$Port, [string]$PhaseName) {
  $fails = New-Object System.Collections.Generic.List[string]
  $notes = New-Object System.Collections.Generic.List[string]
  $ours = @($events | Where-Object { $_.Application -like '*\healthieryou\*' -and ($_.Application -like '*\hy-companion.exe' -or $_.Application -like '*\msedgewebview2.exe') })
  $toServer = 0
  foreach ($e in $ours) {
    $prog = Split-Path -Leaf $e.Application
    if ($e.Direction -ne 'outbound') { $fails.Add("$prog accepted an inbound connection (it never listens)"); continue }
    if ($ServerIps -contains $e.DestAddress) {
      if ($e.Protocol -eq 6 -and [int]$e.DestPort -eq $Port -and $prog -eq 'hy-companion.exe') { $toServer++ }
      else { $fails.Add("$prog connected to the server on protocol $($e.Protocol) port $($e.DestPort)") }
      continue
    }
    $role = Test-PrivateAddress $e.DestAddress
    if ($role -eq 'loopback') { $notes.Add("$prog connected to this PC itself (port $($e.DestPort))"); continue }
    $fails.Add("$prog connected to $(if ($role -eq 'internet') { 'an internet address' } else { 'another LAN or local address' }) on protocol $($e.Protocol) port $($e.DestPort)$(if (-not $e.Allowed) { ' (blocked by Windows)' })")
  }
  if ($PhaseName -eq 'BeforePairing' -and $toServer -gt 0) { $fails.Add('before pairing, the companion connected to the server') }
  return [pscustomobject]@{ Connections = $ours.Count; ToServer = $toServer; Fails = @($fails | Select-Object -Unique); Notes = @($notes | Select-Object -Unique) }
}

# The Filtering Platform audit events of a window. "No events" is an empty list; any other
# failure to read the Security log is an error (the phase is then INCONCLUSIVE, not quiet).
function Read-AuditEvents([datetime]$From, [datetime]$To) {
  try {
    $raw = @(Get-WinEvent -FilterHashtable @{ LogName = 'Security'; Id = 5156, 5157; StartTime = $From; EndTime = $To } -ErrorAction Stop)
  }
  catch {
    if ($_.FullyQualifiedErrorId -like 'NoMatchingEventsFound*') { return [pscustomobject]@{ Events = @(); Error = $null } }
    return [pscustomobject]@{ Events = @(); Error = $_.Exception.GetType().Name }
  }
  $events = @($raw | ForEach-Object {
      $x = [xml]$_.ToXml()
      $d = @{}; foreach ($n in $x.Event.EventData.Data) { $d[$n.Name] = $n.'#text' }
      [pscustomobject]@{
        Application = [string]$d['Application']
        Direction   = $(if ($d['Direction'] -eq '%%14592') { 'inbound' } else { 'outbound' })
        DestAddress = [string]$d['DestAddress']; DestPort = [int]$d['DestPort']; Protocol = [int]$d['Protocol']
        Allowed     = ($_.Id -eq 5156)
      }
    })
  return [pscustomobject]@{ Events = $events; Error = $null }
}

# One phase's verdict. FAIL: something the evidence shows is wrong. INCONCLUSIVE: evidence
# the phase needs is missing (no PASS without it). PASS: neither.
#   $Cap    Invoke-CaptureAnalysis result.
#   $Audit  $null for a wire-only analysis (-Analyze), else the program audit of the phase:
#           Enabled (the audit policy was switched on and read back), Error (reading the
#           Security log failed), Events (the window's events).
#   $CompanionRunning  the same companion process ran from the start to the end of the phase.
function Get-PhaseVerdict($Cap, $Audit, [bool]$CompanionRunning, [string[]]$ServerIps, [int]$Port) {
  $fails = @($Cap.Fails)
  $missing = @($Cap.Missing)
  $prog = $null
  if ($null -ne $Audit) {
    if (-not $Audit.Enabled) { $missing += 'the connection audit could not be switched on and read back, so no program attribution is possible' }
    elseif ($Audit.Error) { $missing += "the Security log could not be read ($($Audit.Error)), so no program attribution is possible" }
    else {
      $events = @($Audit.Events)
      $prog = Test-ProgramConnections $events $ServerIps $Port $Cap.Phase
      $fails += @($prog.Fails)
      if ($events.Count -eq 0) { $missing += 'the connection audit recorded no connection by any program in this window, so it shows nothing' }
      elseif ($Cap.Phase -ne 'BeforePairing' -and $prog.ToServer -eq 0) { $missing += 'the connection audit shows no connection by the companion to the server in this phase (the steps were not done, or the companion is not the one installed)' }
    }
    if (-not $CompanionRunning) { $missing += 'the companion was not running from the start to the end of this phase' }
  }
  $verdict = if ($fails.Count) { 'FAIL' } elseif ($missing.Count) { 'INCONCLUSIVE' } else { 'PASS' }
  return [pscustomobject]@{ Phase = $Cap.Phase; Verdict = $verdict; Fails = $fails; Missing = $missing; Programs = $prog }
}

function Get-OverallVerdict([string[]]$Verdicts) {
  if (-not $Verdicts -or $Verdicts.Count -eq 0) { return 'INCONCLUSIVE' }
  if ($Verdicts -contains 'FAIL') { return 'FAIL' }
  if ($Verdicts -contains 'INCONCLUSIVE') { return 'INCONCLUSIVE' }
  return 'PASS'
}

# The overall verdict of a physical run: the phases' verdict, except that a run which could
# not put the connection audit policy back exactly as it was is never PASS (a FAIL finding
# still stands; otherwise it is INCONCLUSIVE, with the manual fix in the report).
function Get-RunVerdict([string[]]$Verdicts, [string]$RestoreError) {
  $overall = Get-OverallVerdict $Verdicts
  if ($RestoreError -and $overall -ne 'FAIL') { return 'INCONCLUSIVE' }
  return $overall
}

# ------------------------------------------------------------- connection audit policy

# The Filtering Platform Connection audit subcategory, by GUID (its name is localized).
$AuditSubcategory = '{0CCE9226-69AE-11D9-BED3-505054503030}'

# The subcategory's setting from the lines of an `auditpol /backup` file. That CSV holds,
# per subcategory, its GUID (column 4) and its numeric "Setting Value" (column 7: 0 no
# auditing, 1 success, 2 failure, 3 success and failure). Both are the same in every Windows
# language; the header, the subcategory's name and the "Inclusion Setting" text are not, so
# none of them is read. Returns Value (0..3) or Error: no line for the GUID, more than one,
# or any other value is an Error, never a setting.
function ConvertFrom-AuditBackup([string[]]$Lines, [string]$Guid) {
  $lines = @($Lines | Where-Object { $_ -and $_.Trim() })
  if (-not $lines.Count) { return [pscustomobject]@{ Value = $null; Error = 'the audit policy backup is empty' } }
  $want = $Guid.Trim('{', '}', ' ')
  $rows = @($lines | ConvertFrom-Csv -Header 'c1', 'c2', 'c3', 'c4', 'c5', 'c6', 'c7', 'c8' |
      Where-Object { $_.c4 -and $_.c4.Trim().Trim('{', '}') -eq $want })
  if ($rows.Count -eq 0) { return [pscustomobject]@{ Value = $null; Error = "the audit policy backup has no line for subcategory $Guid" } }
  if ($rows.Count -gt 1) { return [pscustomobject]@{ Value = $null; Error = "the audit policy backup has $($rows.Count) lines for subcategory $Guid (per-user policy?), so its system setting is ambiguous" } }
  $raw = if ($null -eq $rows[0].c7) { '' } else { ([string]$rows[0].c7).Trim() }
  if ($raw -notmatch '^[0-3]$' -or $rows[0].c8) { return [pscustomobject]@{ Value = $null; Error = "the audit policy backup has an unrecognised setting value '$raw' for subcategory $Guid" } }
  return [pscustomobject]@{ Value = [int]$raw; Error = $null }
}

# The auditpol /set switches for a setting value; anything but 0..3 is refused, so no
# unknown state can be turned into a policy.
function Get-AuditFlags($Value) {
  if ($null -eq $Value -or "$Value" -notmatch '^[0-3]$') { throw "Unrecognised audit setting value '$Value': refusing to set it." }
  switch ([int]$Value) {
    0 { return @('/success:disable', '/failure:disable') }
    1 { return @('/success:enable', '/failure:disable') }
    2 { return @('/success:disable', '/failure:enable') }
    3 { return @('/success:enable', '/failure:enable') }
  }
}

function Read-AuditPolicyState {
  $file = Join-Path ([IO.Path]::GetTempPath()) ("hy-auditpol-{0}.csv" -f [guid]::NewGuid().ToString('N'))
  try {
    & auditpol.exe /backup "/file:$file" | Out-Null
    if ($LASTEXITCODE -ne 0) { return [pscustomobject]@{ Value = $null; Error = "auditpol /backup failed (exit code $LASTEXITCODE)" } }
    if (-not (Test-Path -LiteralPath $file)) { return [pscustomobject]@{ Value = $null; Error = 'auditpol /backup wrote no file' } }
    return ConvertFrom-AuditBackup @(Get-Content -LiteralPath $file) $AuditSubcategory
  }
  catch { return [pscustomobject]@{ Value = $null; Error = "the audit policy could not be read ($($_.Exception.Message))" } }
  finally { Remove-Item -LiteralPath $file -Force -ErrorAction SilentlyContinue }
}

function Set-AuditPolicyState($Value) {
  $flags = Get-AuditFlags $Value
  & auditpol.exe /set "/subcategory:$AuditSubcategory" @flags | Out-Null
  return ($LASTEXITCODE -eq 0)
}

# Runs $Body with the connection audit switched on, and puts the policy back exactly.
#   $ReadState  returns Value (0..3) or Error; $SetState takes a value, returns success.
# The original setting is read and validated before anything is changed: if it cannot be
# read or is not recognised, nothing is changed and Aborted says why. After $Body (also when
# it throws) the original value is set again and read back; RestoreError says what is wrong
# if the read-back is not exactly the original. $Body gets whether the audit is on (set
# and read back as 3).
function Invoke-AuditGuarded([scriptblock]$ReadState, [scriptblock]$SetState, [scriptblock]$Body) {
  $r = [pscustomobject]@{ Original = $null; Aborted = $null; Enabled = $false; Changed = $false; Restored = $false; RestoreError = $null }
  $read = { try { $s = & $ReadState; if ($null -eq $s) { [pscustomobject]@{ Value = $null; Error = 'no audit policy state returned' } } else { $s } } catch { [pscustomobject]@{ Value = $null; Error = "the audit policy could not be read ($($_.Exception.Message))" } } }
  $orig = & $read
  if ($orig.Error -or $null -eq $orig.Value -or "$($orig.Value)" -notmatch '^[0-3]$') {
    $r.Aborted = if ($orig.Error) { [string]$orig.Error } else { "unrecognised original audit setting '$($orig.Value)'" }
    return $r
  }
  $r.Original = [int]$orig.Value
  try {
    $setOk = $true
    if ($r.Original -ne 3) {
      $r.Changed = $true
      $setOk = $false; try { $setOk = [bool](& $SetState 3) } catch { $setOk = $false }
    }
    $now = & $read
    $r.Enabled = $setOk -and -not $now.Error -and "$($now.Value)" -eq '3'
    $null = & $Body $r.Enabled
  }
  finally {
    if ($r.Changed) {
      $restoreMsg = $null
      try { if (-not (& $SetState $r.Original)) { $restoreMsg = 'auditpol /set reported a failure' } } catch { $restoreMsg = $_.Exception.Message }
      $back = & $read
      if ($back.Error) { $r.RestoreError = "the policy could not be read back after restoring it ($($back.Error))" }
      elseif ("$($back.Value)" -ne "$($r.Original)") { $r.RestoreError = "after restoring, the policy reads setting value $($back.Value), not the original $($r.Original)$(if ($restoreMsg) { " ($restoreMsg)" })" }
      else { $r.Restored = $true }
      if ($r.RestoreError) { Write-Warning "The connection audit policy was NOT put back: $($r.RestoreError). Restore it by hand (elevated): auditpol /set /subcategory:$AuditSubcategory $((Get-AuditFlags $r.Original) -join ' ')" }
    }
    else { $r.Restored = $true }
  }
  return $r
}

# ----------------------------------------------------------------------- self-test

function New-PcapNg($packets) {
  $ms = New-Object IO.MemoryStream
  $w = New-Object IO.BinaryWriter($ms)
  $w.Write([uint32]0x0A0D0D0A); $w.Write([uint32]28); $w.Write([uint32]0x1A2B3C4D); $w.Write([uint16]1); $w.Write([uint16]0); $w.Write([int64]-1); $w.Write([uint32]28)
  $w.Write([uint32]1); $w.Write([uint32]20); $w.Write([uint16]1); $w.Write([uint16]0); $w.Write([uint32]65535); $w.Write([uint32]20)
  foreach ($f in $packets) {
    $pad = (4 - $f.Length % 4) % 4
    $total = 32 + $f.Length + $pad
    $w.Write([uint32]6); $w.Write([uint32]$total); $w.Write([uint32]0); $w.Write([uint32]0); $w.Write([uint32]0)
    $w.Write([uint32]$f.Length); $w.Write([uint32]$f.Length); $w.Write([byte[]]$f); for ($i = 0; $i -lt $pad; $i++) { $w.Write([byte]0) }
    $w.Write([uint32]$total)
  }
  $w.Flush()
  return , $ms.ToArray()
}

function New-Frame([string]$src, [string]$dst, [string]$proto, [int]$sport, [int]$dport, [uint32]$seq, [byte]$flags, [byte[]]$payload) {
  $l4 = if ($proto -eq 'TCP') { 20 } else { 8 }
  $total = 20 + $l4 + $payload.Length
  $f = New-Object System.Collections.Generic.List[byte]
  $f.AddRange([byte[]](2, 0, 0, 0, 0, 1, 2, 0, 0, 0, 0, 2, 8, 0))
  $f.AddRange([byte[]](0x45, 0, ($total -shr 8), ($total -band 0xFF), 0, 0, 0x40, 0, 64, $(if ($proto -eq 'TCP') { 6 } else { 17 }), 0, 0))
  $f.AddRange([System.Net.IPAddress]::Parse($src).GetAddressBytes())
  $f.AddRange([System.Net.IPAddress]::Parse($dst).GetAddressBytes())
  $f.AddRange([byte[]](($sport -shr 8), ($sport -band 0xFF), ($dport -shr 8), ($dport -band 0xFF)))
  if ($proto -eq 'TCP') {
    $f.AddRange([byte[]]((($seq -shr 24) -band 0xFF), (($seq -shr 16) -band 0xFF), (($seq -shr 8) -band 0xFF), ($seq -band 0xFF), 0, 0, 0, 0, 0x50, $flags, 0xFF, 0xFF, 0, 0, 0, 0))
  }
  else { $f.AddRange([byte[]](0, (8 + $payload.Length), 0, 0)) }
  $f.AddRange($payload)
  return , $f.ToArray()
}

function New-Record([int]$type, [byte[]]$body) {
  return , ([byte[]](@($type, 3, 3, ($body.Length -shr 8), ($body.Length -band 0xFF)) + $body))
}

function New-Hello([bool]$client, [int[]]$versions, [int[]]$extraExt) {
  $ext = New-Object System.Collections.Generic.List[byte]
  if ($versions.Count) {
    if ($client) {
      $sv = [byte[]](@($versions.Count * 2) + @($versions | ForEach-Object { ($_ -shr 8), ($_ -band 0xFF) }))
    }
    else { $sv = [byte[]](($versions[0] -shr 8), ($versions[0] -band 0xFF)) }
    $ext.AddRange([byte[]](0, 43, 0, $sv.Length)); $ext.AddRange($sv)
  }
  if ($client) { $ext.AddRange([byte[]](0, 16, 0, 7, 0, 5, 4, 0x68, 0x79, 0x2F, 0x31)) }
  foreach ($x in $extraExt) { $ext.AddRange([byte[]](0, $x, 0, 0)) }
  $body = New-Object System.Collections.Generic.List[byte]
  $body.AddRange([byte[]](3, 3)); $body.AddRange((New-Object byte[] 32))
  if ($client) { $body.AddRange([byte[]](0, 0, 2, 0x13, 1, 1, 0)) } else { $body.AddRange([byte[]](0, 0x13, 1, 0)) }
  $body.AddRange([byte[]](($ext.Count -shr 8), ($ext.Count -band 0xFF))); $body.AddRange($ext)
  $msg = [byte[]](@($(if ($client) { 1 } else { 2 }), 0, ($body.Count -shr 8), ($body.Count -band 0xFF)) + $body.ToArray())
  return , (New-Record 22 $msg)
}

function Invoke-SelfTest {
  $srv = '192.168.50.10'; $pc = '192.168.50.21'
  $dir = Join-Path ([IO.Path]::GetTempPath()) "hy-capture-selftest-$PID"
  New-Item -ItemType Directory -Force $dir | Out-Null
  $app = New-Record 23 ([byte[]](1..40))
  $ch = New-Hello $true @(0x0304) @()
  $sh = New-Hello $false @(0x0304) @()
  function Conn([byte[]]$up, [byte[]]$down, [int]$port = 8443, [int]$cport = 50001) {
    @((New-Frame $pc $srv 'TCP' $cport $port 999 2 @()), (New-Frame $pc $srv 'TCP' $cport $port 1000 0x18 $up), (New-Frame $srv $pc 'TCP' $port $cport 5000 0x18 $down))
  }
  $cases = @(
    @{ Name = 'genuine TLS 1.3'; Frames = (Conn ([byte[]]($ch + $app)) ([byte[]]($sh + $app))); Verdict = 'PASS'; Want = '' },
    @{ Name = 'TLS 1.2 also offered'; Frames = (Conn ([byte[]]((New-Hello $true @(0x0304, 0x0303) @()) + $app)) ([byte[]]($sh + $app))); Verdict = 'FAIL'; Want = 'offers versions' },
    @{ Name = 'no supported_versions (TLS 1.2 client)'; Frames = (Conn ([byte[]]((New-Hello $true @() @()) + $app)) ([byte[]]($sh + $app))); Verdict = 'FAIL'; Want = 'no supported_versions' },
    @{ Name = 'server picks TLS 1.2'; Frames = (Conn ([byte[]]($ch + $app)) ([byte[]]((New-Hello $false @(0x0303) @()) + $app))); Verdict = 'FAIL'; Want = 'chose version 0303' },
    @{ Name = 'early data offered'; Frames = (Conn ([byte[]]((New-Hello $true @(0x0304) @(42)) + $app)) ([byte[]]($sh + $app))); Verdict = 'FAIL'; Want = 'early data' },
    @{ Name = 'plain HTTP to the server port'; Frames = (Conn ($Latin1.GetBytes("POST /v1/upload HTTP/1.1`r`nAuthorization: Bearer x`r`n`r`n")) ([byte[]]@())); Verdict = 'FAIL'; Want = 'readable protocol text' },
    @{ Name = 'server reached on another port'; Frames = (Conn ([byte[]]($ch + $app)) ([byte[]]($sh + $app)) 80); Verdict = 'FAIL'; Want = 'only TCP 8443' },
    @{ Name = 'canary in clear to an internet address'; Frames = @(, (New-Frame $pc '8.8.8.8' 'UDP' 5353 53 0 0 ($Latin1.GetBytes('xxCAPTURE-PC-7F3Axx')))); Verdict = 'FAIL'; Want = 'canary text' },
    @{ Name = 'before pairing, any packet to the server'; Frames = (Conn ([byte[]]($ch + $app)) ([byte[]]($sh + $app))); Verdict = 'FAIL'; Want = 'before pairing'; Phase = 'BeforePairing' }
  )
  # Out of order with a retransmission: the reassembly must still find a clean TLS 1.3 stream.
  $upBytes = [byte[]]($ch + $app)
  $half = [int]($upBytes.Length / 2)
  $p1 = Get-Slice $upBytes 0 $half; $p2 = Get-Slice $upBytes $half ($upBytes.Length - $half)
  $cases += @{ Name = 'reordered and retransmitted segments'; Frames = @(
      (New-Frame $pc $srv 'TCP' 50002 8443 999 2 @()),
      (New-Frame $pc $srv 'TCP' 50002 8443 ([uint32](1000 + $half)) 0x18 $p2),
      (New-Frame $pc $srv 'TCP' 50002 8443 1000 0x18 $p1),
      (New-Frame $pc $srv 'TCP' 50002 8443 1000 0x18 $p1),
      (New-Frame $srv $pc 'TCP' 8443 50002 5000 0x18 ([byte[]]($sh + $app)))); Verdict = 'PASS'; Want = ''
  }
  # Incomplete evidence is never PASS: the server's side missing, a gap after the hello, a
  # record cut off, nothing captured at all.
  $cases += @{ Name = 'a ClientHello with nothing from the server'; Frames = (Conn ([byte[]]($ch + $app)) ([byte[]]@())); Verdict = 'INCONCLUSIVE'; Want = 'nothing from the server' }
  $cases += @{ Name = 'a gap after the ClientHello'; Frames = @(
      (New-Frame $pc $srv 'TCP' 50003 8443 999 2 @()),
      (New-Frame $pc $srv 'TCP' 50003 8443 1000 0x18 $ch),
      (New-Frame $pc $srv 'TCP' 50003 8443 ([uint32](1000 + $ch.Length + 100)) 0x18 $app),
      (New-Frame $srv $pc 'TCP' 8443 50003 5000 0x18 ([byte[]]($sh + $app)))); Verdict = 'INCONCLUSIVE'; Want = 'missing segments'
  }
  $cases += @{ Name = 'a record cut off by the end of the capture'; Frames = (Conn ([byte[]]($ch + (Get-Slice $app 0 12))) ([byte[]]($sh + $app))); Verdict = 'INCONCLUSIVE'; Want = 'cut off' }
  $cases += @{ Name = 'an empty capture (active phase)'; Frames = @(); Verdict = 'INCONCLUSIVE'; Want = 'no packets' }
  $cases += @{ Name = 'an empty capture (before pairing)'; Frames = @(); Verdict = 'INCONCLUSIVE'; Want = 'no packets'; Phase = 'BeforePairing' }
  $cases += @{ Name = 'an active phase with no server connection'; Frames = @(, (New-Frame $pc '192.168.50.30' 'UDP' 5353 5353 0 0 ([byte[]](1..20)))); Verdict = 'INCONCLUSIVE'; Want = 'no complete TLS connection' }
  $cases += @{ Name = 'before pairing, other traffic only'; Frames = @(, (New-Frame $pc '192.168.50.30' 'UDP' 5353 5353 0 0 ([byte[]](1..20)))); Verdict = 'PASS'; Want = ''; Phase = 'BeforePairing' }
  $failed = 0
  foreach ($c in $cases) {
    $file = Join-Path $dir 'case.pcapng'
    [IO.File]::WriteAllBytes($file, (New-PcapNg $c.Frames))
    $phaseName = if ($c.ContainsKey('Phase')) { $c.Phase } else { 'PairedOff' }
    $r = Invoke-CaptureAnalysis $file @($srv) 8443 @('CAPTURE-PC-7F3A') $phaseName
    $v = Get-PhaseVerdict $r $null $true @($srv) 8443
    $said = @($v.Fails) + @($v.Missing)
    $ok = ($v.Verdict -eq $c.Verdict) -and ($c.Verdict -eq 'PASS' -or (($said -join ' | ') -like "*$($c.Want)*"))
    if ($c.Verdict -ne 'PASS' -and $r.Tls13Connections -gt 0 -and $c.Verdict -eq 'INCONCLUSIVE') { $ok = $false }
    if (-not $ok) { $failed++ }
    Write-Host ("[{0}] {1}: {2} {3}" -f $(if ($ok) { 'ok' } else { 'WRONG' }), $c.Name, $v.Verdict, ($said -join ' | '))
  }
  # The classic pcap format.
  $frames = Conn ([byte[]]($ch + $app)) ([byte[]]($sh + $app))
  $ms = New-Object IO.MemoryStream; $w = New-Object IO.BinaryWriter($ms)
  $w.Write([uint32]2712847316); $w.Write([uint16]2); $w.Write([uint16]4); $w.Write([int32]0); $w.Write([uint32]0); $w.Write([uint32]65535); $w.Write([uint32]1)
  foreach ($f in $frames) { $w.Write([uint32]0); $w.Write([uint32]0); $w.Write([uint32]$f.Length); $w.Write([uint32]$f.Length); $w.Write([byte[]]$f) }
  $w.Flush()
  $file = Join-Path $dir 'case.pcap'
  [IO.File]::WriteAllBytes($file, $ms.ToArray())
  $r = Invoke-CaptureAnalysis $file @($srv) 8443 @() 'PairedOff'
  $ok = $r.Fails.Count -eq 0 -and $r.Tls13Connections -eq 1
  if (-not $ok) { $failed++ }
  Write-Host ("[{0}] classic pcap file: {1} TLS 1.3 connection(s)" -f $(if ($ok) { 'ok' } else { 'WRONG' }), $r.Tls13Connections)
  # The program audit.
  $ev = @(
    [pscustomobject]@{ Application = '\device\harddiskvolume3\users\x\appdata\local\healthieryou\hy-companion.exe'; Direction = 'outbound'; DestAddress = $srv; DestPort = 8443; Protocol = 6; Allowed = $true },
    [pscustomobject]@{ Application = '\device\harddiskvolume3\program files\other\other.exe'; Direction = 'outbound'; DestAddress = '8.8.8.8'; DestPort = 443; Protocol = 6; Allowed = $true }
  )
  $checks = @(
    @{ Name = 'companion to the server only'; Events = $ev; Pass = $true },
    @{ Name = 'bundled WebView2 to the internet'; Events = ($ev + [pscustomobject]@{ Application = '\device\harddiskvolume3\users\x\appdata\local\healthieryou\webview2-fixed\msedgewebview2.exe'; Direction = 'outbound'; DestAddress = '20.1.2.3'; DestPort = 443; Protocol = 6; Allowed = $true }); Pass = $false },
    @{ Name = 'companion to another LAN host'; Events = ($ev + [pscustomobject]@{ Application = '\device\x\healthieryou\hy-companion.exe'; Direction = 'outbound'; DestAddress = '192.168.50.99'; DestPort = 8443; Protocol = 6; Allowed = $true }); Pass = $false }
  )
  foreach ($c in $checks) {
    $r = Test-ProgramConnections $c.Events @($srv) 8443 'PairedOff'
    $ok = (($r.Fails.Count -eq 0) -eq $c.Pass)
    if (-not $ok) { $failed++ }
    Write-Host ("[{0}] program audit, {1}: {2}" -f $(if ($ok) { 'ok' } else { 'WRONG' }), $c.Name, $(if ($r.Fails.Count) { $r.Fails -join ' | ' } else { 'no finding' }))
  }
  # A physical phase's verdict: PASS needs the companion running, the audit on and read,
  # events in the window, the companion's own connection to the server and a verified TLS
  # 1.3 connection on the wire.
  $file = Join-Path $dir 'good.pcapng'
  [IO.File]::WriteAllBytes($file, (New-PcapNg (Conn ([byte[]]($ch + $app)) ([byte[]]($sh + $app)))))
  $good = Invoke-CaptureAnalysis $file @($srv) 8443 @() 'PairedOff'
  $file = Join-Path $dir 'empty.pcapng'
  [IO.File]::WriteAllBytes($file, (New-PcapNg @()))
  $empty = Invoke-CaptureAnalysis $file @($srv) 8443 @() 'PairedOff'
  $emptyBefore = Invoke-CaptureAnalysis $file @($srv) 8443 @() 'BeforePairing'
  $file = Join-Path $dir 'quiet.pcapng'
  [IO.File]::WriteAllBytes($file, (New-PcapNg @(, (New-Frame $pc '192.168.50.30' 'UDP' 5353 5353 0 0 ([byte[]](1..20))))))
  $quietBefore = Invoke-CaptureAnalysis $file @($srv) 8443 @() 'BeforePairing'
  $on = [pscustomobject]@{ Enabled = $true; Error = $null; Events = $ev }
  $others = @($ev | Where-Object { $_.Application -notlike '*healthieryou*' })
  $phases = @(
    @{ Name = 'full evidence'; Cap = $good; Audit = $on; Running = $true; Want = 'PASS' },
    @{ Name = 'the companion absent'; Cap = $good; Audit = $on; Running = $false; Want = 'INCONCLUSIVE' },
    @{ Name = 'the audit could not be switched on'; Cap = $good; Audit = [pscustomobject]@{ Enabled = $false; Error = $null; Events = @() }; Running = $true; Want = 'INCONCLUSIVE' },
    @{ Name = 'the Security log could not be read'; Cap = $good; Audit = [pscustomobject]@{ Enabled = $true; Error = 'UnauthorizedAccessException'; Events = @() }; Running = $true; Want = 'INCONCLUSIVE' },
    @{ Name = 'no audit events at all'; Cap = $good; Audit = [pscustomobject]@{ Enabled = $true; Error = $null; Events = @() }; Running = $true; Want = 'INCONCLUSIVE' },
    @{ Name = 'no companion connection in the audit'; Cap = $good; Audit = [pscustomobject]@{ Enabled = $true; Error = $null; Events = $others }; Running = $true; Want = 'INCONCLUSIVE' },
    @{ Name = 'an empty capture'; Cap = $empty; Audit = $on; Running = $true; Want = 'INCONCLUSIVE' },
    @{ Name = 'an empty capture before pairing'; Cap = $emptyBefore; Audit = [pscustomobject]@{ Enabled = $true; Error = $null; Events = $others }; Running = $true; Want = 'INCONCLUSIVE' },
    @{ Name = 'before pairing, quiet with full evidence'; Cap = $quietBefore; Audit = [pscustomobject]@{ Enabled = $true; Error = $null; Events = $others }; Running = $true; Want = 'PASS' },
    @{ Name = 'the companion absent and a finding'; Cap = $good; Audit = [pscustomobject]@{ Enabled = $true; Error = $null; Events = @($ev + [pscustomobject]@{ Application = '\device\x\healthieryou\hy-companion.exe'; Direction = 'outbound'; DestAddress = '20.1.2.3'; DestPort = 443; Protocol = 6; Allowed = $true }) }; Running = $false; Want = 'FAIL' }
  )
  foreach ($c in $phases) {
    $v = Get-PhaseVerdict $c.Cap $c.Audit $c.Running @($srv) 8443
    $ok = $v.Verdict -eq $c.Want
    if (-not $ok) { $failed++ }
    Write-Host ("[{0}] phase verdict, {1}: {2} {3}" -f $(if ($ok) { 'ok' } else { 'WRONG' }), $c.Name, $v.Verdict, ((@($v.Fails) + @($v.Missing)) -join ' | '))
  }
  $overallOk = ((Get-OverallVerdict @('PASS', 'INCONCLUSIVE', 'PASS')) -eq 'INCONCLUSIVE') -and ((Get-OverallVerdict @('INCONCLUSIVE', 'FAIL')) -eq 'FAIL') -and ((Get-OverallVerdict @()) -eq 'INCONCLUSIVE') -and ((Get-OverallVerdict @('PASS', 'PASS')) -eq 'PASS')
  if (-not $overallOk) { $failed++ }
  Write-Host ("[{0}] overall verdict: any FAIL fails, then any INCONCLUSIVE, no phases is INCONCLUSIVE" -f $(if ($overallOk) { 'ok' } else { 'WRONG' }))
  # The connection audit policy: read by GUID and numeric value from an auditpol /backup
  # file, refused when unreadable or unknown (never taken as "disabled"), put back and
  # read back after the run.
  $g = $AuditSubcategory
  # Accented letters by code, so Windows PowerShell 5.1 reads this file the same as 7.
  $ea = [char]0xE9; $eg = [char]0xE8; $Ec = [char]0xC9; $Uu = [char]0xDC
  $hdr = 'Machine Name,Policy Target,Subcategory,Subcategory GUID,Inclusion Setting,Exclusion Setting,Setting Value'
  $other = 'PC1,System,Audit Logon,{0cce9215-69ae-11d9-bed3-505054503030},Success,,1'
  $parse = @(
    @{ Name = 'English, success and failure'; Lines = @($hdr, $other, "PC1,System,Filtering Platform Connection,$($g.ToLower()),Success and Failure,,3"); Value = 3 },
    @{ Name = 'English, no auditing'; Lines = @($hdr, "PC1,System,Filtering Platform Connection,$g,No Auditing,,0"); Value = 0 },
    @{ Name = 'German output, success only'; Lines = @('Computername,Richtlinienziel,Unterkategorie,Unterkategorie-GUID,Einschlusseinstellung,Ausschlusseinstellung,Einstellungswert', "PC1,System,Filterplattformverbindung,$g,Erfolg,,1"); Value = 1 },
    @{ Name = 'French output, failure only'; Lines = @("Nom de l'ordinateur,Cible de la strat$($ea)gie,Sous-cat$($ea)gorie,GUID de sous-cat$($ea)gorie,Param$($eg)tre d'inclusion,Param$($eg)tre d'exclusion,Valeur du param$($eg)tre", "PC1,Syst$($eg)me,""Connexion de la plateforme de filtrage, test"",$g,$($Ec)chec,,2"); Value = 2 },
    @{ Name = 'read failure (nothing read)'; Lines = @(); Value = $null },
    @{ Name = 'subcategory missing'; Lines = @($hdr, $other); Value = $null },
    @{ Name = 'unknown setting value'; Lines = @($hdr, "PC1,System,Filtering Platform Connection,$g,Success and Failure,,7"); Value = $null },
    @{ Name = 'setting value missing (localized text only)'; Lines = @('Computername,Richtlinienziel,Unterkategorie,Unterkategorie-GUID,Einschlusseinstellung', "PC1,System,Filterplattformverbindung,$g,Erfolg und Fehler"); Value = $null },
    @{ Name = 'text where the value should be'; Lines = @($hdr, "PC1,System,Filtering Platform Connection,$g,Keine $($Uu)berwachung,,Keine $($Uu)berwachung"); Value = $null },
    @{ Name = 'two lines for the subcategory'; Lines = @($hdr, "PC1,System,Filtering Platform Connection,$g,Success,,1", "PC1,S-1-5-21-1,Filtering Platform Connection,$g,Include Success,,5"); Value = $null }
  )
  foreach ($c in $parse) {
    $s = ConvertFrom-AuditBackup $c.Lines $g
    $ok = if ($null -eq $c.Value) { $s.Error -and $null -eq $s.Value } else { -not $s.Error -and $s.Value -eq $c.Value }
    if (-not $ok) { $failed++ }
    Write-Host ("[{0}] audit policy read, {1}: {2}" -f $(if ($ok) { 'ok' } else { 'WRONG' }), $c.Name, $(if ($s.Error) { "refused: $($s.Error)" } else { "value $($s.Value)" }))
  }
  $flagsOk = (((Get-AuditFlags 0) -join ' ') -eq '/success:disable /failure:disable') -and (((Get-AuditFlags 3) -join ' ') -eq '/success:enable /failure:enable')
  foreach ($bad in @($null, '', 4, 'x', -1)) { try { Get-AuditFlags $bad | Out-Null; $flagsOk = $false } catch { } }
  if (-not $flagsOk) { $failed++ }
  Write-Host ("[{0}] audit flags: 0..3 map to switches, anything else is refused (never 'disable')" -f $(if ($flagsOk) { 'ok' } else { 'WRONG' }))
  # The guarded run against a fake policy: Reads are the states the reader returns in turn
  # (the last repeats), Sets records every value set.
  function FakeRun($reads, [bool]$setResult = $true, [bool]$bodyThrows = $false) {
    $st = @{ Reads = @($reads); I = 0; Sets = New-Object System.Collections.Generic.List[object]; BodyRan = $false; BodyEnabled = $null }
    $rd = { $x = $st.Reads[[Math]::Min($st.I, $st.Reads.Count - 1)]; $st.I++; if ($x -is [string]) { if ($x -eq 'THROW') { throw 'auditpol not found' }; return [pscustomobject]@{ Value = $null; Error = $x } }; return [pscustomobject]@{ Value = $x; Error = $null } }.GetNewClosure()
    $wr = { param($v) $st.Sets.Add($v); return $setResult }.GetNewClosure()
    $bd = { param($en) $st.BodyRan = $true; $st.BodyEnabled = $en; if ($bodyThrows) { throw 'pktmon could not start' } }.GetNewClosure()
    $res = $null; $threw = $false
    $wp = $WarningPreference; $WarningPreference = 'SilentlyContinue'
    try { $res = Invoke-AuditGuarded $rd $wr $bd } catch { $threw = $true } finally { $WarningPreference = $wp }
    return [pscustomobject]@{ R = $res; Sets = $st.Sets.ToArray(); BodyRan = $st.BodyRan; BodyEnabled = $st.BodyEnabled; Threw = $threw }
  }
  $guard = @(
    @{ Name = 'read failure aborts, nothing set'; Run = (FakeRun @('auditpol /backup failed (exit code 5)')); Check = { param($x) $x.R.Aborted -and $x.Sets.Count -eq 0 -and -not $x.BodyRan } },
    @{ Name = 'reader throws: aborts, nothing set'; Run = (FakeRun @('THROW')); Check = { param($x) $x.R.Aborted -and $x.Sets.Count -eq 0 -and -not $x.BodyRan } },
    @{ Name = 'unknown value aborts, nothing set'; Run = (FakeRun @(7)); Check = { param($x) $x.R.Aborted -and $x.Sets.Count -eq 0 -and -not $x.BodyRan } },
    @{ Name = 'off: switched on, restored to 0, verified'; Run = (FakeRun @(0, 3, 0)); Check = { param($x) -not $x.R.Aborted -and $x.BodyEnabled -and ($x.Sets -join ',') -eq '3,0' -and $x.R.Restored -and -not $x.R.RestoreError } },
    @{ Name = 'success only: restored to 1, not to disabled'; Run = (FakeRun @(1, 3, 1)); Check = { param($x) ($x.Sets -join ',') -eq '3,1' -and $x.R.Restored } },
    @{ Name = 'already on: left alone'; Run = (FakeRun @(3, 3)); Check = { param($x) $x.Sets.Count -eq 0 -and $x.BodyEnabled -and $x.R.Restored } },
    @{ Name = 'switch-on not read back: audit off for the phases, still restored'; Run = (FakeRun @(2, 2, 2)); Check = { param($x) $x.BodyRan -and -not $x.BodyEnabled -and ($x.Sets -join ',') -eq '3,2' -and $x.R.Restored } },
    @{ Name = 'restore read back differs: RestoreError'; Run = (FakeRun @(1, 3, 0)); Check = { param($x) $x.R.RestoreError -and -not $x.R.Restored -and (Get-RunVerdict @('PASS') $x.R.RestoreError) -eq 'INCONCLUSIVE' } },
    @{ Name = 'restore read back fails: RestoreError'; Run = (FakeRun @(0, 3, 'access denied')); Check = { param($x) $x.R.RestoreError -and -not $x.R.Restored } },
    @{ Name = 'restore set fails and policy stays on: RestoreError'; Run = (FakeRun @(0, 3, 3) $false); Check = { param($x) $x.R.RestoreError -and -not $x.R.Restored } },
    @{ Name = 'phases throw: still restored'; Run = (FakeRun @(0, 3, 0) $true $true); Check = { param($x) $x.Threw -and ($x.Sets -join ',') -eq '3,0' } }
  )
  foreach ($c in $guard) {
    $ok = [bool](& $c.Check $c.Run)
    if (-not $ok) { $failed++ }
    $x = $c.Run
    Write-Host ("[{0}] audit policy guard, {1}: sets [{2}]{3}{4}" -f $(if ($ok) { 'ok' } else { 'WRONG' }), $c.Name, ($x.Sets -join ','), $(if ($x.R -and $x.R.Aborted) { "; aborted: $($x.R.Aborted)" }), $(if ($x.R -and $x.R.RestoreError) { "; restore error: $($x.R.RestoreError)" }))
  }
  $runOk = ((Get-RunVerdict @('PASS', 'PASS') $null) -eq 'PASS') -and ((Get-RunVerdict @('PASS', 'PASS') 'x') -eq 'INCONCLUSIVE') -and ((Get-RunVerdict @('FAIL') 'x') -eq 'FAIL')
  if (-not $runOk) { $failed++ }
  Write-Host ("[{0}] run verdict: a policy not put back is never PASS" -f $(if ($runOk) { 'ok' } else { 'WRONG' }))
  Remove-Item -Recurse -Force $dir
  if ($failed) { throw "Capture analyser self-test: $failed case(s) wrong" }
  Write-Host 'Capture analyser self-test: all cases right.'
}

# ------------------------------------------------------------------------ reports

$Lines = New-Object System.Collections.Generic.List[string]
function Add-Line([string]$Text) { $Lines.Add($Text); Write-Host $Text }
function Write-PhaseResult($cap, $v) {
  $prog = $v.Programs
  Add-Line ("[{0}] T-S23 phase {1}" -f $v.Verdict, $cap.Phase)
  Add-Line ("       packets {0}; with the server {1}; server connections {2}, verified TLS 1.3 only {3}" -f $cap.Packets, $cap.ServerPackets, $cap.ServerConnections, $cap.Tls13Connections)
  if ($prog) { Add-Line ("       HealthierYou program connections {0}, to the server {1}" -f $prog.Connections, $prog.ToServer) }
  foreach ($f in $v.Fails) { Add-Line "       FAIL: $f" }
  foreach ($m in $v.Missing) { Add-Line "       MISSING EVIDENCE: $m" }
  foreach ($n in @($cap.Notes) + $(if ($prog) { @($prog.Notes) } else { @() })) { Add-Line "       note: $n" }
  return $v.Verdict
}

if ($SelfTest) { Invoke-SelfTest; return }

if ($PSCmdlet.ParameterSetName -eq 'Analyze') {
  $r = Invoke-CaptureAnalysis (Resolve-Path $Analyze).Path $ServerIp $ServerPort $Canary $Phase
  $v = Get-PhaseVerdict $r $null $true $ServerIp $ServerPort
  $verdict = Write-PhaseResult $r $v
  Add-Line 'This is the wire only: which program made each connection needs the connection audit of a physical run.'
  Add-Line 'Content inside TLS (which fields each phase sends) is proven by the CI test crates/hy-server/tests/capture.rs, not by a capture.'
  Add-Line "Overall: $verdict"
  if ($Report) { $Lines | Set-Content -Encoding UTF8 $Report; $r | ConvertTo-Json -Depth 4 | Set-Content -Encoding UTF8 ($Report + '.json') }
  if ($verdict -eq 'FAIL') { exit 1 }
  if ($verdict -ne 'PASS') { exit 2 }
  return
}

# ------------------------------------------------------------- physical capture run

$admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $admin) { throw 'Run this in an elevated PowerShell (pktmon and the connection audit need an administrator).' }
if (-not (Get-Command pktmon.exe -ErrorAction SilentlyContinue)) { throw 'pktmon is not available (Windows 10 version 2004 or later is needed).' }
if (-not $Report) { $Report = Join-Path $Out 'hy-capture-report.txt' }
New-Item -ItemType Directory -Force $Out | Out-Null
$ips = @([System.Net.Dns]::GetHostAddresses($Server) | ForEach-Object { $_.ToString() })
if (-not $ips.Count) { throw "The server name $Server does not resolve on this PC." }
$os = Get-CimInstance Win32_OperatingSystem
Add-Line "HealthierYou network capture (T-S23) - $((Get-Date).ToString('yyyy-MM-dd HH:mm'))"
Add-Line ("Windows: {0} build {1}; server resolved to {2} address(es); server port TCP {3}; canary given: {4}" -f $os.Caption, $os.BuildNumber, $ips.Count, $ServerPort, [bool]$Canary.Count)
function Get-CompanionIds { return @(Get-Process -Name 'hy-companion' -ErrorAction SilentlyContinue | ForEach-Object { $_.Id }) }
$companion = Get-CompanionIds
Add-Line "Companion running: $([bool]$companion.Count)"
if (-not $companion.Count) { Write-Warning 'The companion is not running: start it (every phase needs it running, or the phase is INCONCLUSIVE).' }
Add-Line ''

$guidance = @{
  BeforePairing = 'Install the companion but do not pair it (Settings > Sharing shows "Not paired"). Leave it running.'
  Pending       = 'Pair now: Settings > Sharing > paste the slip, use the -Canary text as the PC name. Do NOT let the administrator confirm yet. Click "Check again" once.'
  PairedOff     = 'Ask the administrator to confirm this PC. Keep every sharing category off. Click "Check again" twice.'
  CategoryOn    = 'Switch on Water sharing (Settings > Sharing). Log a glass of water from the pet menu. Click "Check again" twice.'
  AfterDisable  = 'Switch Water sharing off again. Log another glass of water. Click "Check again" twice.'
}
$results = New-Object System.Collections.Generic.List[object]
$runPhases = {
  param([bool]$auditEnabled)
  if (-not $auditEnabled) { Write-Warning 'The connection audit could not be switched on and read back: every phase will be INCONCLUSIVE.' }
  foreach ($ph in $Phases) {
    Write-Host ''
    Write-Host "Phase $ph" -ForegroundColor Cyan
    Write-Host $guidance[$ph]
    Read-Host 'Press Enter to start the capture for this phase' | Out-Null
    $etl = Join-Path $Out "phase-$ph.etl"
    $pcap = Join-Path $Out "phase-$ph.pcapng"
    Remove-Item -Force $etl, $pcap -ErrorAction SilentlyContinue
    pktmon filter remove | Out-Null
    $idsAtStart = Get-CompanionIds
    $start = Get-Date
    pktmon start --capture --comp nics --pkt-size 0 --file-name $etl | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'pktmon could not start' }
    try {
      if ($Seconds -gt 0) { Write-Host "Capturing for $Seconds s: do the steps above now."; Start-Sleep -Seconds $Seconds }
      else { Read-Host 'Capturing: do the steps above now, wait about 30 seconds after the last step, then press Enter' | Out-Null }
    }
    finally { pktmon stop | Out-Null }
    $end = Get-Date
    $idsAtEnd = Get-CompanionIds
    $running = @($idsAtStart | Where-Object { $idsAtEnd -contains $_ }).Count -gt 0
    pktmon etl2pcap $etl --out $pcap | Out-Null
    if (-not (Test-Path $pcap)) { throw "pktmon could not convert the capture for $ph" }
    $cap = Invoke-CaptureAnalysis $pcap $ips $ServerPort $Canary $ph
    $read = Read-AuditEvents $start $end.AddSeconds(2)
    $audit = [pscustomobject]@{ Enabled = $auditEnabled; Error = $read.Error; Events = $read.Events }
    $v = Get-PhaseVerdict $cap $audit $running $ips $ServerPort
    $verdict = Write-PhaseResult $cap $v
    $results.Add([pscustomobject]@{ Phase = $ph; Verdict = $verdict; Capture = $cap; Programs = $v.Programs; Missing = $v.Missing })
  }
}
# The connection audit policy is read (by GUID and numeric value, the same in every Windows
# language) and validated before anything is changed, and put back and read back at the end.
try {
  $guard = Invoke-AuditGuarded ${function:Read-AuditPolicyState} ${function:Set-AuditPolicyState} $runPhases
}
finally { pktmon filter remove 2>$null | Out-Null }
$policy = [pscustomobject]@{ Original = $guard.Original; Changed = $guard.Changed; Restored = $guard.Restored; Error = $(if ($guard.Aborted) { $guard.Aborted } else { $guard.RestoreError }) }
if ($guard.Aborted) {
  Add-Line "MISSING EVIDENCE: the connection audit policy could not be read before the run ($($guard.Aborted)), so it could not be switched on and put back safely."
  Add-Line 'Nothing was captured and the audit policy was not changed. Fix what it names (auditpol /backup must work in this elevated PowerShell) and run again.'
  Add-Line 'Overall: INCONCLUSIVE'
  $Lines | Set-Content -Encoding UTF8 $Report
  [pscustomobject]@{ Overall = 'INCONCLUSIVE'; AuditPolicy = $policy; Phases = @() } | ConvertTo-Json -Depth 5 | Set-Content -Encoding UTF8 ($Report + '.json')
  Write-Host "`nReport written."
  exit 2
}
Add-Line ''
if ($guard.RestoreError) {
  Add-Line "ERROR: the connection audit policy was NOT put back as it was: $($guard.RestoreError)."
  Add-Line "       Restore it by hand in an elevated PowerShell: auditpol /set /subcategory:$AuditSubcategory $((Get-AuditFlags $guard.Original) -join ' ')"
}
elseif ($guard.Changed) { Add-Line 'The connection audit policy was put back as it was and read back.' }
else { Add-Line 'The connection audit policy was already on and was left as it was.' }
Add-Line 'What this capture proves: the endpoints and port, TLS 1.3 only, nothing readable on the wire, and (from the program audit) no other connection by the companion or its WebView2.'
Add-Line 'What it cannot see: which fields are inside TLS. T-S23''s content conditions are proven by the CI test crates/hy-server/tests/capture.rs on the real client and server.'
$overall = Get-RunVerdict @($results | ForEach-Object { $_.Verdict }) $guard.RestoreError
Add-Line "Overall: $overall$(if ($overall -eq 'INCONCLUSIVE') { $(if ($guard.RestoreError) { ' (the audit policy was not put back: restore it as above)' } else { ' (evidence missing in a phase: fix what it names and run that phase again)' }) })"
Add-Line 'The report names no user, PC, path, IP address or device id. The .etl and .pcapng files hold raw traffic of this PC: keep them, do not share them.'
$Lines | Set-Content -Encoding UTF8 $Report
[pscustomobject]@{ Overall = $overall; AuditPolicy = $policy; Phases = $results.ToArray() } | ConvertTo-Json -Depth 6 | Set-Content -Encoding UTF8 ($Report + '.json')
Write-Host "`nReport written."
if ($overall -eq 'FAIL') { exit 1 }
if ($overall -ne 'PASS') { exit 2 }
