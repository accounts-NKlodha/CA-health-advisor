# Batch 1 results: scripted run 5, laptop (20 Oct 2026, 16:00 IST)

Raw zip kept privately by the owner. Build 6ef6eb3 (Media Foundation capture restored). Same laptop as runs 3 and 4 (i5-1035G1, 7.8 GB, Windows 11 25H2). Minimum PC target is 8 GB (Rohit, 20 Oct). Rohit ran the full test, not -Quick.

| Test | Measured | Target | Verdict |
| --- | --- | --- | --- |
| I1 Offline install | 27.3 s | < 120 s | Pass |
| I2 Bundled runtime | Install folder only | Bundled | Pass |
| P5 Launch | 2.0 s | <= 10 s | Pass |
| P1 Idle memory | 95.3 MB private working set | <= 150 MB | Pass |
| P2 Idle CPU | 1.1% mean, worst minute 1.3% | <= 2%, <= 5% | Pass |
| P3 Focus | 10/10 | 10/10 | Pass |
| P4 Click | max 98 ms | <= 500 ms | Pass |
| C1 Camera CPU, 10 min | 4.9% of machine | <= 10% | Pass |
| C2 Camera memory | 301 MB peak private working set | <= 350 MB | Pass |
| C3 Camera released on stop | Windows record: released 0.35 s after the app's stop (last cycle: app stop 16:48:37.623, Windows last_stop 16:48:37.977). Rohit: Stop in normal use closes the camera immediately | <= 2 s | **Pass** (script printed Fail) |
| C4 Permission denied | Scripted registry Deny: camera started (method invalid). Manual Settings check (Rohit, 20 Oct): "Camera Unavailable (NotAllowedError). The pet keeps working." | camera_error, pet keeps running | **Pass** (manual) |
| I3 Uninstall | 1.4 s | removed | Pass (registry entry left) |

## Notes

- C3: the script's question was ambiguous. Rohit's "about 20 s" is the length of each on-period (the script runs the camera 20 s per cycle), not a delay after stop. Windows' own camera-usage record and Rohit's manual Stop both show immediate release. Windows only keeps the last start/stop pair, so this is one recorded cycle plus manual observation.
- C4: Windows registers the app in its camera privacy list (the usage record exists), yet direct registry writes did not block it. The Settings page notifies the Windows camera service when it changes; a raw registry write may not. Needs a 1-minute manual check: Settings > Privacy & security > Camera > turn off "Let desktop apps access your camera", open the app, press Start.

- C4 result (20 Oct, 22:52 IST): with "Let desktop apps access your camera" off in Settings, the app showed "Camera Unavailable (NotAllowedError). The pet keeps working." Pass. The script's raw registry write is not a valid way to deny; future scripts should drop it and keep the manual step.

**Batch 1 on the laptop: 12 of 12 pass.**
