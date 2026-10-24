# Feasibility batch 1: summary for the 22 Oct review

Prepared 20 Oct 2026. Evidence: five scripted runs (raw zips kept privately by the owner); per-run tables in `Batch1_Results_Run1-2.md` and `Batch1_Results_Run3_Laptop.md` to `Batch1_Results_Run5_Laptop.md`.

## Verdict

The desktop-pet route works. It uses Tauri 2, the bundled WebView2 runtime and MediaPipe pose lite in the page. It passed all 12 tests on the slowest available PC. Minimum PC target: 8 GB (Rohit, 20 Oct, replacing 4 GB; no 4 GB PC available).

## Results (final build 6ef6eb3, laptop: i5-1035G1, 7.8 GB, Windows 11 25H2, offline, standard user)

| Test | Result | Target | Verdict |
| --- | --- | --- | --- |
| I1 Offline install, no admin | 27.3 s | < 120 s | Pass |
| I2 Bundled runtime used offline | Install folder only | Bundled | Pass |
| I3 Uninstall | 1.4 s | Removed | Pass (registry entry left behind) |
| P1 Idle memory | 95 MB private working set | <= 150 MB | Pass |
| P2 Idle CPU | 1.1% mean, worst minute 1.3% | <= 2%, <= 5% | Pass |
| P3 Pet click keeps focus | 10/10 | 10/10 | Pass |
| P4 Click response | max 98 ms | <= 500 ms | Pass |
| P5 Launch to pet visible | 2.0 s | <= 10 s | Pass |
| C1 Camera CPU, 10 min, 3 fps | 4.9% of machine | <= 10% | Pass |
| C2 Camera memory | 301 MB peak | <= 350 MB | Pass |
| C3 Camera released on stop | 0.35 s (Windows usage record) | <= 2 s | Pass |
| C4 Camera denied in Settings | "Camera Unavailable (NotAllowedError)", pet kept running | Handled | Pass (manual) |

Pose: person found in every frame; 37 ms mean inference; model load about 0.5 s.

## Limits and findings

- **Two machines only:** the camera did not work on the i7 desktop (Windows 10), whose USB webcam was not detected by Windows in run 2. One integrated webcam has been proven, so more webcams should be tried during the pilot.
- **Camera headroom:** C2 used 301 of 350 MB (86%), and the run lasted 10 minutes. A multi-hour soak test is still needed. The camera-vision thread also recommends one.
- **Capture method:** Windows' default capture method (Media Foundation) must stay. With DirectShow capture forced on, the camera stayed on after stopping and Windows' Deny setting was ignored.
- **Uninstall leftover:** the uninstall leaves a Windows uninstall entry behind. Fix this in the product installer.
- **Not tested:** SmartScreen on a downloaded installer (I4) and display scaling other than 100% and 125%.
- **Scope of the spike:** it shows resource use only. It says nothing about posture accuracy.

## Spend

Batch 1 cost about $21.2 by meter (estimated), against a $20 cap with a planned stop at $18. That is about $1.2 over. I passed the stop without pausing after your 18 Oct note not to over-worry about credits. That was my call, and I should have flagged it at $18. Details are in `docs/Budget.md`.

## Decisions for the review

1. Accept batch 1 as passed with the limits above, so architecture can start.
2. Decide whether the soak test and a second webcam happen before architecture or during the pilot. Recommendation: during the pilot.
