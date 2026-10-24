# V1 acceptance: open physical tests, content gates and tracked work

Status: **open list for V1 UAT** (slice V1-7, 24 Oct 2026). It collects in one place every V1 check that CI cannot run, from the architecture (`docs/Architecture_V1_V2.md` on the architecture branch, sections 10, 11 and 14), the V1-5 contract (`docs/V1-5_Pairing_Security.md`, section 14), the V1-6 notes (`docs/V1-6_Install_Update.md`, sections 6 and 8) and the "needs physical testing" lists of PRs #16 to #20. Nothing here is marked passed: an item passes only when it has been run on the real machine and its report is filed.

Who runs each item:

- **Owner**: on the office companion PCs, signed in as the office user (no administrator), unless the item says otherwise.
- **Server administrator**: on the office server PC, as an administrator (the owner or office IT).
- **Codex**: security and implementation review.
- **Content reviewer**: Codex's independent content-review agent (PRD "movement-content acceptance"); the owner approves the experience.
- **Firm**: the firm's named contact for privacy and legal questions.

Every script below writes a report with no user name, PC name, folder path, IP address or device id. Read a report before sharing it. Capture files (`.etl`, `.pcapng`) hold the PC's raw traffic and are never shared.

## 1. V1-7: the 8-hour soak and the network capture

| Id | Test | How | Expected | Who |
| --- | --- | --- | --- | --- |
| S1 | 8-hour companion soak (architecture 11 "Soak", A2, A7; PRD performance targets) | `scripts/windows/Test-Soak.ps1 -Role Companion -Report hy-soak-companion.txt` on a companion PC, ideally the 4 GB Windows 10 PC, with ordinary office apps open. Add `-Installer <setup.exe>` if HealthierYou is not installed. Leave it running overnight or through a working day; locking and sleep are allowed (only awake time counts, at least 8 h) | Overall PASS: same process throughout (no crash or restart); no growth trend in working set, private bytes, handles or threads; working set median at most 150 MB; CPU at most 2% of the machine over every 30 minutes, no spike over 5% for 3 samples; write-ahead log at most 16 MiB, database growth at most 4 MiB; reminders never within the 5-minute calm gap, at most 6 in an hour | Owner |
| S2 | 8-hour server soak during a working day | `scripts/windows/Test-Soak.ps1 -Role Server -Report hy-soak-server.txt`, elevated, on the server PC while the three pilot companions are paired and syncing | Overall PASS: the service process never restarts; no growth trend; write-ahead log at most 16 MiB; log within rotation; at most 16 backups; no panic and no 5xx answer in the server log (request counts and p95 latency reported) | Server administrator |
| S3 | Network capture, T-S23 (architecture 9 and 11, A12) | `scripts/windows/Test-NetworkCapture.ps1 -Server <server name> -Canary <PC label> -Out C:\hy-capture`, elevated, on a companion PC. It guides five phases (before pairing, pending, paired with every category off, Water on, Water off), capturing each with pktmon and the Windows connection audit. Pair with the `-Canary` text as the PC label | Overall PASS (exit 0). FAIL (exit 1) is a finding; INCONCLUSIVE (exit 2) means evidence a phase needs is missing (companion not running for the whole phase, the connection audit not switched on or the Security log unreadable, no audit event, no companion connection to the server, no packets, or no complete TLS connection with both hellos and no gap): fix what it names and run that phase again. Per phase: the companion and its WebView2 connect to nothing but the server on TCP 8443 (no other LAN host, no internet address); every server connection is TLS 1.3 only (no other version offered, no early data, no resumption, ALPN hy/1, TLS 1.3 chosen, only encrypted records after the hellos); no HTTP text, `/v1/` path, header name or JSON key on the wire, and the label in no packet anywhere; nothing to the server before pairing | Owner (needs an administrator account on that PC) |
| S4 | Second-PC check of S3 on Windows 10 | As S3 on the Windows 10 PC | As S3 | Owner |

**What CI proves for these, and what only the physical run proves.**

| Area | CI (every PR, GitHub runners) | Only the office run |
| --- | --- | --- |
| Companion soak | `Test-Soak.ps1 -CiSelfTest` on the Windows job: the installed app, after its self-test, runs 72 simulated office hours in 12 minutes on its injected clock: about 30 reminders shown on the real bubble and answered there in turn (done, snooze, not now, unanswered), lunchtime locks, overnight sleeps, the settings and review windows opened and closed every 3 simulated hours. The app checks the cadence (at least 3 reminders a full workday, none within the calm gap, none off hours, none while locked or asleep), every answer stored, and the store bounded; the script samples the whole process tree and fails on a growth trend | Real time (8 hours of real timers, WebView2's own housekeeping, Windows updates and other apps around it), the real 4 GB office PC and its CPU against the PRD budgets, real lock, sleep and wake, and an unattended PC (the CI run answers every reminder at once) |
| Server soak | `Test-Soak.ps1 -Sim` on the Windows job: `hy-sim soak`, the real server in process with 10 simulated companions over 30 simulated office days in 8 minutes (each PC logs water hourly and syncs every 15 minutes; hourly maintenance and the daily backup): every row checked each day, control log and mirror equal, backups within retention, write-ahead log bounded, database growth not speeding up, sync p95 not growing; the script samples the process for growth trends | The Windows service under its virtual account, real network latency, real companions, the real disk and mirror location, a full working day of real time |
| Capture | `crates/hy-server/tests/capture.rs` (Linux and Windows jobs): the companion's real pairing and sync code against the real server through a relay recording every byte, per T-S23 phase. On the bytes: TLS 1.3 only in both directions, no planted value (alias, label, token, slip secret, water amount) or protocol text in clear. From the server's endpoint log and rows: nothing before pairing, pairing and status only while pending, no upload with every category off, rows that are exactly their schema's canonical form when on, nothing after the disable. The Linux job writes that traffic as pcapng and runs `Test-NetworkCapture.ps1 -Analyze` on it (it must pass) and on a negative control (it must find the ALPN text that is in clear in every ClientHello), plus the analyser's self-test on synthetic captures (TLS 1.2 offered, early data, plain HTTP, wrong port, a canary in clear, out-of-order segments, classic pcap; and, never PASS, a ClientHello with nothing from the server, a gap after the hello, a record cut off, an empty capture, and phase verdicts with the companion absent, the audit off or unreadable, or no audit events). The self-tests also run under Windows PowerShell 5.1 on the Windows job | The real network path and adapter, name resolution, the firewall, and that **Windows and WebView2 make no other connection** for HealthierYou (A12): the program audit attributes every connection to its program, which no CI test can do. What a capture cannot see (which fields are inside TLS) is proven by the CI test, not by the capture |

## 2. Security (V1-5)

| Id | Test | How | Expected | Who |
| --- | --- | --- | --- | --- |
| T-S11a | DPAPI: another Windows user on the same PC cannot open user A's token | Pair as user A. Sign in as user B on the same PC, copy A's `%LOCALAPPDATA%\HealthierYou\data\hy.db` into B's profile and start the companion | "Re-pair needed"; no crash; nothing sent | Owner |
| T-S11b | A companion database copied to another PC or profile | Copy a paired `hy.db` to another PC; start the companion there | "Re-pair needed"; no crash | Owner |
| T-S11c | A standard user cannot read the server's key files | As a standard user on the server PC: `Get-ChildItem C:\ProgramData\HealthierYou\Server` | Access denied (the V1-6b installer sets SYSTEM, Administrators and the service account only) | Server administrator |
| T-S12a | The admin page is unreachable from another LAN host | From a companion PC: `Test-NetConnection <server> -Port 8444` | Fails (8444 listens on loopback only and has no firewall rule) | Owner |
| T-S24 | Rogue server by name | A second LAN host answers to the server's name (a hosts-file entry on the companion PC pointing at another PC running `hy-server` with its own key) | Pin mismatch shown ("not the office server"); the token is never sent; S3's capture on that PC shows no application data to the rogue host | Owner with the server administrator |
| T-P6 (LAN) | Server address change | Change the server PC's IP (or its DHCP lease); the companion finds it by name with the same pin. Revoke a device: it gets 401 and offers to re-pair | Reconnects with the pin; the revoked PC shows re-pair | Owner, server administrator |
| T-S13 (disk image) | Companion restored from an older disk image | Restore an older image or copy of a paired companion's data, then reconnect | `device_regressed`: every category off, outbox empty, admin flag; a repeat within 24 h revokes it | Owner |
| T-S22 (memory) | Denial-of-service limits on the real service | Covered in CI for behaviour; S2 records memory on the real service. Optionally run the T-S22 load from another PC | Memory stays flat (S2 trend checks) | Server administrator |
| T-J1 to T-J4 (real) | Process kill and power loss during control-log writes, on NTFS and with the mirror on an SMB share | Stop the service process (`taskkill /F`) and pull power while companions toggle sharing; restart | Chain verifies to the last complete entry; the mirror is repaired from the database at start; the high-waters are equal before uploads resume (T-J6) | Server administrator |
| Mirror | Mirror on a real second disk or network share | Install with `/MIRROR=` on the second volume or share (give the computer account modify rights on a share); unplug the share during a sync | `check-mirror` and the service accept it; with the share gone, control changes fail closed and are retried, never acknowledged without the mirror | Server administrator |
| Backup | Scheduled backup at 13:00 IST on the real service | Leave the service running across 13:00; check `backups\` and the admin page | One backup a day (plus the catch-up after 24 h), retention 14 daily and 2 weekly | Server administrator |
| Restore | Restore on the real server with the administrator witnessing | `hy-server restore --backup <file>` with the service stopped, start it, confirm users and devices on the admin page, let the PCs check in | Each user released when their evidence is complete; nothing served before; companions show "Server recovering" meanwhile | Server administrator |
| LAN-3 | Three real companions plus `hy-sim` on the office LAN (architecture 5.1) | Pair the three pilot PCs; run their normal day; S2 at the same time | All three sync; reviews show each device separately; S2 passes | Owner, server administrator |
| A14 | Slip usability | Paste the slip from internal chat, and type it from print | Users manage both; any confusion recorded | Owner |
| UI | The companion's pairing panel, Sharing section, notices and periodic sync on Windows 10 and 11 | Real UI on both | Matches the wireframes; no error in normal use | Owner |

## 3. Install and update (V1-6a), companion PCs

Run `scripts/windows/Test-HealthierYou-OfficePC.ps1 -Installer <setup.exe> -Report <file>` (guided; phases Install, AfterSignIn, Uninstall).

| Id | Test | Expected | Who |
| --- | --- | --- | --- |
| I1 | Install as a standard user on Windows 10 and on Windows 11 (I4) | No UAC prompt; per-user entry only | Owner |
| I2 | SmartScreen: a browser-downloaded copy against a LAN or USB copy | Wording recorded; "Run anyway" works; never bypassed | Owner |
| I3 | Smart App Control (Windows 11 clean install) or an office AppLocker/WDAC policy | Whether the installer is blocked. A block is a **signing decision** for the owner (A5), never worked around | Owner |
| I4 | Display scale 100, 125, 150 and 200%, and two monitors with different scales (A6) | Pet and Settings sharp and inside the work area | Owner |
| I5 | Start at sign-in after a real sign-out and sign-in; allowed by office policy (A15); after sleep and wake | Running after sign-in; enabled in Task Manager; pet back after wake | Owner |
| I6 | Update and roll back from the Settings UI with a downloaded (marked) file | Updated, then "Go back to ..." works | Owner |
| I7 | Uninstall from Windows Settings | Data box unticked and shows the folder; nothing left in Installed apps or the registry | Owner |
| I8 | Antivirus or EDR | Does not quarantine the installer, the update helper copy (`updates\helper`) or the bundled runtime | Owner, office IT |
| A16 | Runtime size | About 180 MB installed; acceptable over the LAN and by USB | Owner |

## 4. Server install (V1-6b), server PC

| Id | Test | Expected | Who |
| --- | --- | --- | --- |
| SV1 | Install on the real server PC as an administrator | SmartScreen and "Unknown publisher" as in V1-6 section 4; service running; fingerprint shown | Server administrator |
| SV2 | A companion on another PC connects through the firewall rule | The network profile is Private or Domain, never Public | Server administrator, owner |
| SV3 | Mirror on the real second disk or share (see section 2 "Mirror") | Accepted and remembered | Server administrator |
| SV4 | Power settings | No sleep in office hours; the service returns after a reboot (delayed start) and after a crash | Server administrator |
| SV5 | A real upgrade with a downloaded installer, and uninstall from Windows Settings with the data box unticked | Upgrade keeps the data; uninstall keeps the data and the mirror | Server administrator |
| SV6 | Moving the mirror to the real share (copy, then `/MIRROR=`) | `check-mirror` reads it over SMB; the service then writes it as the computer account | Server administrator |
| SV7 | Install over a data folder copied or restored by hand (domain owners, inherited groups) | `check-acl` passes after setup | Server administrator |
| SV8 | First-install provenance | The administrator compares the installer's SHA-256 with the one the owner publishes (bootstrap limitation, V1-6 section 8) | Server administrator |

## 5. Windows acceptance (architecture 11)

| Id | Test | Expected | Who |
| --- | --- | --- | --- |
| A8 | Focus: the pet never takes focus; the hotkey panel takes it and Escape gives it back | As stated, while typing in Word or a browser | Owner |
| A6 | Monitor unplugged while the pet is on it | The pet is placed again on the remaining screen | Owner |
| A10 | Presentation and full-screen detection: 20 scripted trials (PowerPoint slide show, a full-screen video, Do Not Disturb) | Prompts held during each; the manual "in a meeting" toggle always works | Owner |
| A7 | Idle memory at most 150 MB with real art and the UI | S1's working-set check on the office PC | Owner |

## 6. Content and decision gates

| Id | Gate | What is needed | Who |
| --- | --- | --- | --- |
| G1 | Movement content review | The four movements and the six-day matrix are shown as "Provisional content, pending content review" (`hy_core::coach::movement`). Before the pilot: source fidelity, body-region coverage, adaptations and stop or skip wording checked; unsuitable routines replaced (PRD "movement-content acceptance"; evidence review, not clinical certification) | Content reviewer; owner approves |
| G2 | Six pet designs | Koala, elephant, dog, flamingo, rabbit and panda are `pending_review` (labelled "art pending review" and not selectable). Each needs drawn art passing the pack checks and the owner's approval before it can be chosen | Owner approves the art |
| G3 | Privacy contact and notice wording | A named firm contact for privacy and deletion requests, and the firm's confirmation of the notice wording (architecture 6.3, owner question Q3) | Firm |
| G4 | Legal applicability | DPDP Act and IT Act rules applicability and timing (A13) | Firm |
| G5 | Production update signing keys | `release/update-keys.txt` holds no production key, so no release can be updated. The owner generates the key offline and commits only its public line (V1-6 section 2) before the pilot build | Owner |
| G6 | Code signing | If I3 or SV1 shows a block, the owner decides about signing (A5) | Owner |
| G7 | Codex security review of V1 | Architecture 12: Codex reviews V1 after V1-7 with S1 to S3 and section 2's results | Codex |

## 7. Tracked work (approved, not yet built)

These are required by the approved architecture and not dropped. Each is outside V1-7's scope and is recorded here so it is not lost.

| Id | Work | Source | Notes |
| --- | --- | --- | --- |
| W1 | Server-delivered updates: the companion takes the update file from the office server | Architecture 8; V1-6 section 7 | The file format and the check are the same as the file route; the server must hand over the same two files |
| W2 | Companion observability: JSON-line log rotated at 7 days or 10 MB, the hourly resource summary, a panic hook with webview recreation, and the user-reviewed diagnostics zip | Architecture 10 | The server has its log, rotation and panic hook. The companion has none of these yet, so S1 reads its numbers from outside (`Test-Soak.ps1`) and from the store (`--hy-soak-stats`), and T-S10's diagnostics-zip part cannot run until the zip exists |
| W3 | Supply chain: `cargo-deny` with a `deny.toml`, `npm audit`, the licence register and an SBOM | Architecture 9; V1-5 contract C18 | Not in CI yet |
| W4 | T-S25 fuzzing (cargo-fuzz, time-boxed on Linux CI): slip parser, control-entry decoder and chain verifier, head and receipt decoder, upload JSON | V1-5 contract 14.2 | Property tests cover single-bit changes today; the fuzz targets are not added |
| W5 | Pin the checksums of companion migrations 0005 and 0006 | `hy-core` migrations | They are pinned when first released (as v1 to v4 were); do it in the pilot release build |

## 8. Filing results

For each item: the report file (or a short note for manual items), the build version, the Windows build, the date, and PASS or FAIL with what was seen. A failure is fixed in a slice with a test where one can be written, and the item is run again on the fixed build.
