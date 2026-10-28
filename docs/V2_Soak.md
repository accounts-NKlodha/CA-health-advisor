# V2 posture check: the camera soak (slice V2-5)

Status: built. The CI variant runs on every pull request; the 2-hour runs with a real webcam
are open items for the owner before V2 UAT (`docs/V1_Acceptance.md` section 9). Nothing here
is marked passed until a physical report is filed.

The architecture (`docs/Architecture_V1_V2.md` on the architecture branch, section 12) defines
V2-5 as "2-hour camera soak, Windows 10, second webcam, fixes". Section 11 puts the 2-hour
camera soak and the second webcam before V2 UAT. Assumption A2 is that memory and CPU stay
flat over hours: the camera's 301 MB peak was 86% of the 350 MB budget, measured over
10 minutes only. Assumption A3 is that capture works on Windows 10 and on USB or second
webcams. The PRD's V2 targets are: combined companion and inference memory at most
350 MB during check-ins (by DEC14, the app's own memory: the private working set); at most 10% total CPU averaged during capture; capture ends within
two seconds of Off ("failures are release blockers"); no posture data leaves the PC.

## What is built

| Part | Where | What |
| --- | --- | --- |
| Stop counters | `crates/hy-core/src/camera.rs` (`CameraStats`) | `stops`, `forced_stops` and `max_stop_ms` next to the existing `last_stop_ms`, so a run of hours can show that every release met the deadline, not only the last one. Unit-tested (`every_stop_is_counted_with_the_longest_kept_for_the_soak`). |
| Camera log | `apps/companion/src-tauri/src/camera_log.rs` | Started only with `HY_CAMERA_SOAK_LOG=<file>`. Every `HY_CAMERA_SOAK_EVERY_S` seconds (default 5) the app appends one JSON line: the status as a kind and detail (for example `held` / `meeting`), whether capture is wanted and the camera page exists, the frame rate in force, frames read, stalls, worker restarts, the stop counters, and the mean inference time. It writes only numbers and fixed names: no frame, landmark, posture feature, user name or path. It appends only to a file that already exists, and stops when the file is gone, is over 64 MiB or cannot be written. |
| CI camera soak | `apps/companion/src-tauri/src/selftest/camera_soak.rs` | With `HY_SELFTEST_CAMERA_SOAK_S`, after the camera self-test, the app keeps the posture check capturing on Chromium's synthetic camera. Each cycle is 20 s of steady capture (frames counted against the rate set), then one release in turn: lock, presentation, sleep, meeting, Off. Lock, sleep and presentation go through the coaching engine, as the Windows signals do; meeting uses the hold input; Off is the user's action. Each release is held 4 s, then ended, and the time until capture reads again is measured. |
| Soak script | `scripts/windows/Test-CameraSoak.ps1` | One analysis for three modes: the office run (default), `-CiSelfTest` and `-SelfTest` (synthetic series). It samples the whole process tree (working set, private working set, private bytes, handles, threads, CPU, and TCP connection counts by class: loopback, LAN, internet; never an address). It reads the camera log and gives each check PASS, FAIL or INCONCLUSIVE. Exit code 0 PASS, 1 FAIL, 2 INCONCLUSIVE. |

## The checks

Every check needs its evidence. A check with no evidence is INCONCLUSIVE, never PASS, and
the run is PASS only when every required check is PASS (the convention of
`Test-NetworkCapture.ps1`).

| Check | Office run (required) | CI run |
| --- | --- | --- |
| Camera log read | At least 10 lines, else INCONCLUSIVE | Same |
| No crash or restart | The same main process throughout | Same, inside the soak window |
| Camera time covered | At least `-Hours` (2) of reading time from the log: only intervals where the camera reads (reading or no reading). Starting, stalled, holds, sleep and conflicts do not count. Otherwise INCONCLUSIVE | At least half the soak's minutes |
| Process samples cover the camera time | Samples while the camera page is in use: at least 80% of those expected at the **configured** interval (30 s) with a 25% allowance for each sample's own time and scheduler drift (so at least one per 46.9 s on average, never judged by the surviving samples' own cadence); at least one in every 15-minute stretch with 5 or more minutes of use; at most 20% of sampling attempts failed. Otherwise INCONCLUSIVE, and the budget and trend checks below cannot PASS | Same, at the 5 s CI interval |
| Never stopped as "not responding" | No log line with the camera stopped for two stalls | Same |
| Frame-rate budget held | Over 5 s log intervals of steady capture: delivered/set median at least 0.8; 5th percentile at least 0.5; 95th-percentile rate at most 3.6 a second (3 fps cap with 20% margin). Time at the reduced rate (1 fps) is reported. At least 5 intervals, else INCONCLUSIVE | Same over 2 s intervals; the app also checks each 20 s window (median 0.8, worst 0.5) |
| Watchdog stalls bounded | At most 1 stall per hour of camera time; a worker restart only after a stall | **No** stall or restart (the synthetic camera never stops sending) |
| Every release within 2 s | Stop counter deltas: none forced; the longest at most 2000 ms; at least one release, else INCONCLUSIVE | Same; the app also checks each release, that the page closed, and that it stayed released through the hold |
| Holds release the camera | From the second log line of each hold on, no camera page and no capture. At least one hold is needed, else INCONCLUSIVE ("lock the PC once") | Same; every kind is exercised |
| PRD memory budget | The approved cap, counted as the app's own memory (DEC14): private working set summed over the process tree at most 350 MB at **every** sample while the camera page is in use (the highest sample counts; the median is reported). The whole-tree working set is reported for information. Samples without the private working set give INCONCLUSIVE | Informational (a runner is not an office PC) |
| PRD CPU budget | Process-tree CPU at most 10% of the machine on average while capturing | Informational |
| No growth trend | Working set, private bytes, handles and threads while capturing. Fails if the last third is past tolerance AND the fitted slope adds up to it (as `Test-Soak.ps1`). Too few samples gives INCONCLUSIVE | Same, after a 20% warm-up |
| No internet connection | No TCP connection by the tree to an internet address at any sample. LAN (the office server) is allowed. Unreadable gives INCONCLUSIVE | Same |
| Off within 2 s | The final Off's own stop, timed by the app, at most 2000 ms. INCONCLUSIVE if the camera was not capturing when turned off | Covered by the release check (Off is one of the kinds) |
| Webcam light off (seen) | The person's yes/no; skip gives INCONCLUSIVE | Not possible |
| Memory returned after Off | Informational (within 60 MB of the camera-off baseline) | Not run (the app exits) |

## What CI proves, and what only the office run proves

| CI (every PR, the Windows job, about 10 minutes) | Only the office run |
| --- | --- |
| The installed app's real camera page, real pose model and real watchdog, on a synthetic picture with no person, for 10 minutes. About 15 releases by lock, sleep and presentation (through the engine), meeting and Off, each confirmed within 2 s and followed by capture reading again. The frame rate the governor sets is delivered. No stall. No growth trend in the process tree while capturing. No TCP connection to the internet. The camera log and the script's analysis are the same as the office run's | A real webcam and its driver (the built-in one and a USB second webcam), on Windows 10 and Windows 11. Two real hours with a person in view (real inference load, reading and no-reading changes). Real lock, sleep, meetings and slide shows. The PC's own CPU and memory against the PRD budgets. That the webcam light really goes out |

The CI numbers for this PR's run are in the PR description.

## Privacy

- The camera log and the reports hold counts, rates, durations, fixed status names, the
  Windows build, the processor and memory, and the webcam kind the owner names (built-in,
  USB or other). They hold no frame, landmark, posture feature, user name, PC name, path,
  address or device id.
- The script deletes its temporary log file at the end; that also stops the app's log. The
  samples and log lines go into the report's JSON.
- The camera log is never written unless the script asks for it (the environment variable
  and an existing file). Nothing is sent anywhere.

## Deviations from the architecture

1. **Runs, not a fixed time, decide the evidence.** The office run lasts until it has 2 h of
   camera time, not 2 h of wall time, so that holds, conflicts and sleep do not shorten the
   evidence. It is capped at twice that plus 30 minutes, and INCONCLUSIVE if the cap is
   reached first.
2. **The CI soak is 10 minutes on a synthetic camera.** A runner cannot wait 2 hours and
   has no webcam. Releases are packed far more densely than in a working day, so release,
   resume and trend behaviour is exercised harder. Budgets are informational there, as in
   the V2-1 self-test.
3. **"Windows 10" and "second webcam" are owner runs of the same script.** No code in
   this slice is specific to them. The acceptance rows ask for the soak on the Windows 10
   PC and with a USB webcam. The V2-1 session phase still covers unplugging it.
4. **Memory budget read as the app's own memory over the combined process tree (DEC14).**
   The PRD says "combined companion/inference working set at most 350 MB during check-ins".
   By the owner's decision of 28 October 2026 this counts the app's own memory: the office
   run checks that the private working set summed over the tree is at most 350 MB at its
   highest sample while the camera page is in use. The whole-tree working set, which also
   counts pages shared with other programs, is reported for information. The V2-1 self-test reports the camera's share (on minus off) and
   the office run reports that share too.
5. **Thresholds set here (D, owner to confirm):** at most 1 watchdog stall per hour of
   camera time on a real PC; frame-rate delivery median 0.8 and 5th percentile 0.5 of the
   rate set; trend tolerances as the V1-7 soak.
6. **Network check by sampling.** The script counts the tree's TCP connections at every
   sample (30 s on the office PC, 5 s on CI). A short connection between samples is not
   seen. The camera page's content policy refusal (`camera_network_blocked`, V2-1) and the
   V1-7 network capture (`Test-NetworkCapture.ps1`, which attributes every connection to its
   program) are the strong checks.
7. **One fix from the soak (pending confirmation by CI and the office capture).** On the
   CI runner the soak found a WebView2 process of the app (`msedgewebview2`) holding a TLS
   connection to a Microsoft address (150.171.27.11:443) at 16 of about 120 samples. Nothing
   the app sends is involved: the camera page's content policy refuses every request. It is
   the WebView2 runtime's own background traffic, which architecture 10 ("no internet
   traffic") and A12 rule out. All windows now also get `--disable-background-networking
   --disable-component-update --no-pings` (`windows.rs`, `BROWSER_ARGS`). The V1-7 network
   capture (S3, which attributes every connection to its program) remains the check on
   office PCs. On the CI runner only, the script names the process type, endpoint and
   reverse name of any such connection in a warning, for diagnosis.
