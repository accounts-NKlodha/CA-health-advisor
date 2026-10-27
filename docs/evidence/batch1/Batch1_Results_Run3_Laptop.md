# Batch 1 results: scripted run 3, laptop (19 Oct 2026, 21:45 IST)

Raw zip kept privately by the owner. Build 2c3b002, WebView2 155.0.4283.62 from the install folder. Laptop: Intel Core i5-1035G1 (4 cores, 8 threads), 7.8 GB RAM, Windows 11 Pro 25H2, 125% scaling, integrated webcam, network offline, standard user. **Still not the 4 GB target PC.**

| Test | Measured | Target | Verdict | Why |
| --- | --- | --- | --- | --- |
| I1 Offline install | 27.5 s, exit 0 | < 120 s | Pass | |
| I2 Bundled runtime | Install-folder runtime only | Bundled | Pass | |
| P5 Launch | 1.8 s to pet visible | <= 10 s | Pass | |
| P1 Idle memory | Private bytes 158 MB mean (minute 10 to 30) | <= 150 MB private working set | **Not measured** | Script's private working set sampler returned 7 MB for an 8-process tree (broken) yet scored Pass. Private bytes is an upper bound; on the i7 PC private WS was about half of private bytes |
| P2 Idle CPU | 1.1% mean, worst minute 1.3% | <= 2%, <= 5% | Pass | Excel and Edge open |
| P3 Focus | 10/10 stayed in Notepad | 10/10 | Pass | First real measurement |
| P4 Click response | max 69 ms, mean 34 ms | <= 500 ms | Pass | |
| C1 Camera CPU | 3.2% to 4.3% of machine while on | <= 10% | Pass, partial | First run was stopped after 25 s instead of 10 min (an extra Stop at 34 s, not from the script); figure covers 25 s plus the 5 cycles of 20 s. Pose found a person in every frame, inference 37 ms, 2.6 fps |
| C2 Camera memory | Private bytes 343 MB steady, 369 MB peak | <= 350 MB private working set | **Not measured** | Same broken sampler. Private bytes peak is above 350 MB, so this needs the real private WS figure |
| C3 Light off <= 2 s | Owner: no, not every time; read as about 5 s | 5/5 within 2 s | **Fail** | Script printed negative durations (pairing bug); software stops were on time |
| C4 Permission denied | Camera started anyway | camera_error, pet keeps running | **Not valid** | Script set Deny only on the master per-user key; Windows 11 desktop apps use the NonPackaged key too |
| I3 Uninstall | Exe and folder removed in 1.4 s | removed | Pass | Registry entry still present (same as i7 PC) |

## Main finding

The camera works through WebView2 on this laptop's integrated webcam. The earlier failures on the i7 PC look specific to that PC's USB webcam.

## Fixes queued for the next build

- App: logs private working set directly (Task Manager "Memory" figure) so P1 and C2 can be scored.
- Script: scores P1 and C2 only on that figure; pairs camera on/off events correctly and lists unexpected stops; flags a cut-short first run; asks how many seconds the light stayed on; sets Deny on both camera privacy keys for C4; warns not to click the app during the camera test.
