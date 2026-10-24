# Batch 1 results: scripted runs 1 and 2 (19 Oct 2026)

Raw zips kept privately by the owner. Both runs used build 7523f69 on the same PC: Intel Core i7-4790S, 15.9 GB RAM, Windows 10 Pro 22H2, 100% scaling, network offline. **This is not the 4 GB target PC**, so P1 and P2 are indicative only.

| Test | Run 1 | Run 2 | Verdict | Why |
| --- | --- | --- | --- | --- |
| I1 Offline install, no admin | 30.2 s, exit 0 | 27.5 s, exit 0 | Pass | Network off, standard user |
| I2 Bundled runtime used offline | Yes | Yes | Pass | msedgewebview2.exe ran from the install folder; WebView2 155.0.4283.62 |
| P1 Idle memory | 86.0 MB | 81.6 MB | Pass on this PC; repeat on 4 GB PC | Task-Manager-style private working set. Stricter private bytes: 160.2 / 154.9 MB; working set incl. shared: 385 / 371 MB |
| P2 Idle CPU | 1.0% mean, 1.5% worst minute | 1.0%, 1.6% | Pass on this PC; repeat on 4 GB PC | 8-thread i7; a slower CPU will read higher |
| P5 Launch | Script error | Script error | Pass on this PC by Rohit's manual timing (2.028 s, 19 Oct); repeat on 4 GB PC | Script bug: pet window lookup failed. App logged the pet painted 1.6 s / 2.2 s after launch |
| P3 Focus | Script error | Script error | Not measured | Same script bug; no clicks reached the pet |
| P4 Click response | Script error | Script error | Not measured | Same bug. Manual clicks on 18 Oct logged 51 to 93 ms (target 500 ms) |
| C1 to C4 Camera | NotFoundError | NotFoundError | Not measured; camera never started in any session | 18 Oct manual tries: "Timeout starting video source" (AbortError) with the Camera app closed. Run 2: Windows listed no webcam at all |
| I3 Uninstall | Folder removed | Folder removed | Pass, one note | Uninstall registry entry still present when checked |
| I4 Mark of the Web | Absent | Absent | Info | Installer had no download mark, so no SmartScreen test happened |

## Fixes in build 2c3b002

- Script: PowerShell passed an empty string instead of null to FindWindow, so the pet window was never found. Fixed.
- App camera: logs how many cameras WebView2 sees, retries without a fixed 640x480 size, and uses DirectShow capture instead of Media Foundation. If the camera still fails, the WebView2 camera route is a feasibility finding and the native-capture fallback needs Rohit's approval.
- Script: warns and waits if no webcam is detected (scanners no longer count as cameras).
