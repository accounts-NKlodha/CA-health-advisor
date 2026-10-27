# Batch 1 results: scripted run 4, laptop (20 Oct 2026, 12:51 IST)

Raw zip kept privately by the owner. Build 701207e. Same laptop as run 3: Intel Core i5-1035G1, 7.8 GB RAM, Windows 11 Pro 25H2, 125% scaling, integrated webcam, offline, standard user. Rohit, 20 Oct: no lower-spec PC is available, so this is the slowest machine tested. **Rohit, 20 Oct (decision card): the minimum PC target is now 8 GB, replacing 4 GB.**

| Test | Measured | Target | Verdict |
| --- | --- | --- | --- |
| I1 Offline install | 28.3 s, exit 0 | < 120 s | Pass |
| I2 Bundled runtime | Install-folder runtime only | Bundled | Pass |
| P5 Launch | 1.9 s | <= 10 s | Pass |
| P1 Idle memory | 97.7 MB private working set (private bytes 164 MB) | <= 150 MB | Pass |
| P2 Idle CPU | 1.1% mean, worst minute 1.3% | <= 2%, <= 5% | Pass |
| P3 Focus | 10/10 | 10/10 | Pass |
| P4 Click response | max 82 ms, mean 42 ms | <= 500 ms | Pass |
| C1 Camera CPU, 10 min | 4.1% of machine (machine total 8.0%) | <= 10% | Pass |
| C2 Camera memory | 280 MB peak private working set (private bytes 369 MB) | <= 350 MB | Pass |
| C3 Light off <= 2 s | Owner: light stayed on about 20 s after stop | 5/5 within 2 s | **Fail** |
| C4 Permission denied | Camera started with Deny on both Windows keys | camera_error, pet keeps running | **Fail** |
| I3 Uninstall | Removed in 1.4 s | removed | Pass (registry entry left) |

Software stops were on time (each cycle 20.0 s on). The off gap between cycles was only 10 s, so a 20 s estimate means the light probably never went off between cycles.

## Diagnosis and next build (6ef6eb3)

Both camera failures appeared after build 2c3b002 switched capture from Media Foundation to DirectShow (added for the i7 PC's USB webcam, which later proved absent from Windows). DirectShow capture can keep the device open after the page stops it and can bypass the Windows camera privacy layer. Build 6ef6eb3 returns to Media Foundation and the script now saves Windows' own camera-usage record (camera-usage.txt) as an objective release check. Unverified until rerun.
