# V2 posture check: the camera (slice V2-1)

Status: built behind the user's own choice (off by default). Slices V2-2 (calibration,
floors, drift, viewpoint check and cue), V2-3 (desk sessions) and V2-5 (2-hour soak)
follow. This page covers what V2-1 delivers, how it keeps to architecture 4.5, how it is
tested, and what still needs a real webcam.

## What the user sees

- **Pet menu, tray and Settings > Posture check:** "Posture check" while off, "Turn camera
  off" while on.
- **Consent (wireframe W16):** the first time, the camera page explains what the check
  does and does not do: not medical; frames processed in memory on this PC, never saved
  or sent; the CAM badge while on; Off at any time, with capture stopped within two
  seconds; pauses for away, lock, off hours, presentations and meetings; everything else
  works without it. The choices are **Turn on camera** and **Not now**. Consent is stored
  with the notice version. A later notice version asks again.
- **While on:** the pet shows **"CAM" plus a dot** at its top left. The dot is filled
  while frames give a reading. The tag shows "Cam: no reading" when nobody is in view, the
  light is too poor, or a shoulder is out of the frame. Closing the camera window only
  hides it, and capture goes on.
- **Statuses (W10):** Camera off, Camera starting, Camera on, Cam: no reading, Camera
  paused (away, coaching paused, off hours, presentation, meeting), Camera blocked by
  Windows, Camera in use by another app, No camera found, Posture model could not load,
  Camera stopped: not responding.
- **Conflicts:** when the camera is busy or missing, the check waits and tries again every
  minute, and nothing is measured meanwhile. When Windows blocks the camera, the page and
  a notice beside the pet say where to allow it. In every case the pet, reminders and
  logging keep working.
- **Off:** "Camera off. Capture stopped." appears beside the pet and the badge goes at
  once.
- **Withdraw consent** (Settings): turns the camera off, and the consent text is shown
  again next time.

Posture data stays on this PC. Nothing about the camera is shared with the office server,
and no sharing switch is shown for it. V2-1 records no posture events. V2-2 adds the
calibration numbers, and cues only.

## How it works

| Part | Where | What |
| --- | --- | --- |
| Rules | `crates/hy-core/src/camera.rs` | A pure state machine (unit-tested): consent, on and off, holds, conflicts, the watchdog, the stop deadline and the frame-rate governor. It returns actions for the shell. |
| Holds | `CoachView.holds` (engine) to `PetShell::set_holds` (link) to `HoldSources::camera_hold` | The holds are away or locked, coaching paused, off hours, presentation and meeting. Focus is not a hold: capture goes on, and cues are dropped in V2-2. |
| Shell | `apps/companion/src-tauri/src/shell/camera.rs` | Carries out the actions: opens or closes the camera page, sends it commands, saves the setting, shows notices, and draws the badge, menu and tray entry. A 500 ms clock drives the watchdog and retries. |
| Camera page | `ui/camera.html`, `ui/src/camera/` | The page has its own window, which exists only while needed, so its memory is returned. It runs `getUserMedia` at 640 x 480 (Media Foundation), then the pose worker. |
| Pose worker | `ui/src/camera/pose.worker.ts` | MediaPipe Pose Landmarker lite runs on the CPU, in video mode, at 3 fps (1 fps under load). It applies the valid-frame rule (`frame.ts`) and sends only `{valid, issue, inference_ms, visibility}`. Landmarks never leave the worker and are dropped after each frame. |

**Valid frame (4.5):** the nose, both eyes or both ears, and both shoulders must have
visibility of at least 0.5, and both shoulders must be inside the frame. A frame that
fails is reported as absent, low confidence or out of frame. The status says "no reading"
when most of the last 3 s of frames were not valid.

**Frame path:** where Chromium has `MediaStreamTrackProcessor` (WebView2 does), a clone of
the camera track is turned into a stream and handed to the worker. The worker owns the
timing, so a hidden window keeps its rate. Without it, a hidden video element is used,
and the worker asks the page for one `ImageBitmap` at a time.

**Watchdog:** with no frame result for 5 s, the worker is restarted once and the status
says "no reading (stalled)" until a result arrives. A second stall within 10 minutes
stops the camera, turns the choice off and tells the user. The self-test checks both.

**Stop:** every release (Off, a hold, a stall, withdrawal) stops the camera tracks first,
then closes the worker (ended after 600 ms if it does not answer), then reports
"stopped". If the page does not confirm within 1 s, the shell closes the window. The
stop time is measured (`CameraStats.last_stop_ms`), and the self-test requires 2 s or
less, confirmed by the page. A page that did not confirm is always destroyed, even when it
is on screen explaining a failure (only a page that confirmed its capture ended may stay
to explain). A start that comes while the previous capture is still stopping (a hold that
ends at once, or Off then On) waits: it starts on the same page once the stop is
confirmed, or on a new page once the old one has gone, never on a page mid-stop. The
self-test covers a visible page that hangs and ignores Stop.

**Frame rate:** the governor starts at 3 fps. It drops to 1 fps when the mean inference
time over 10 frames uses more than 6% of the PC's CPU, or exceeds 250 ms. It returns to
3 fps after 60 s under 4%.

### Only the camera page may use the camera

All pages share one origin (`http://tauri.localhost`), and WebView2 saves permission
answers per origin by default. So:

- Tauri's permission handler (`main.rs`) allows the camera only for the window labelled
  `camera`, and only while the machine wants capture. It refuses every other permission.
- Every window also gets a WebView2 `PermissionRequested` handler (`win32::guard_permissions`,
  installed in `windows.rs`). It gives the same answer and sets `SavesInProfile(false)`,
  so no answer is ever remembered. A runtime that cannot honour that gets a refusal.
- The camera page starts capture only after the guard is installed on its window
  (`camera_guarded`).

The self-test checks that the permission is refused before consent, that the guard is
installed, and that a refusal is not remembered: after a refused camera, allowing it
again reads frames.

### Nothing leaves the PC

- The model (`public/mediapipe/pose_landmarker_lite.task`) and the WebAssembly runtime
  (copied from the pinned npm package into `dist/mediapipe/wasm/` at build time) ship
  with the app. No CDN is used at run time.
- **Content policy:** the app's policy adds only `'wasm-unsafe-eval'` (WebAssembly
  compilation, not JavaScript eval), `worker-src blob:` and `'self'` in `connect-src`
  (the app's own files). The camera page adds its own stricter policy:
  `default-src 'none'`, with scripts, WebAssembly and fetches from the app only. The worker
  is an inline blob, so it inherits that policy.
- **Usage metrics:** `@mediapipe/tasks-vision` 1.1.1 contains a usage-metrics sender (to
  `odml.pa.googleapis.com/v1/log`; its README says the APIs "send metrics about the
  performance and utilization"). The content policy blocks it like any other request, and
  after its first failure the library stops trying. The self-test makes the worker try to
  reach the internet and requires the policy to refuse it (`camera_network_blocked`).
  Frames, images and landmarks are never part of those metrics, and no request leaves the
  PC.
- A vision LLM is never involved.

## Budgets and how they are measured

| Budget (4.5) | Check |
| --- | --- |
| Camera at most 10% of the PC's CPU | Self-test `camera_budget_cpu`: process-tree CPU time (Toolhelp + `GetProcessTimes`) over 10 s, on minus off. Informational on the CI runner (shared, no GPU, synthetic camera). Physical: `Test-HealthierYou-Camera.ps1`. |
| Camera at most 350 MB | Self-test `camera_budget_memory`: process-tree working set (`GetProcessMemoryInfo`), on minus off. Informational on CI. Physical: the same script. |
| Memory returned after off | Self-test `camera_memory_returned`: working set after off is within 60 MB of before (informational). |
| Off within 2 s | Self-test `camera_off_releases_within_2s` and `camera_presentation_releases` (required). |
| Inference time | Self-test `camera_inference_ms`: the mean model time per frame (batch 1 measured 37 ms). Informational. |

The CI step prints the report. It turns each failed required check into an error
annotation and each camera check into a notice, so the numbers can be read from the
checks API.

## Licence register

| Component | Version | Licence | Where |
| --- | --- | --- | --- |
| `@mediapipe/tasks-vision` (JS bundle and WebAssembly runtime, SIMD build) | 1.1.1, pinned exactly; integrity in `package-lock.json` | Apache-2.0 (package metadata) | npm; runtime copied to `dist/mediapipe/wasm/` |
| MediaPipe Pose Landmarker lite model (`pose_landmarker_lite.task`, float16, v1) | SHA-256 `59929e1d...690d574a`, 5,777,746 bytes | Apache-2.0, distributed by the MediaPipe project | `apps/companion/ui/public/mediapipe/` with `LICENSE` (Apache 2.0 text) and a provenance README |
| `webview2-com`, `windows-core` (Rust; already in the lockfile through Tauri) | 0.39.1, 0.62.2 | MIT | Windows only, `src/win32.rs` |

Open item for the release checklist: read the BlazePose GHUM model card from the
MediaPipe models page and confirm its licence line. The card is a Google Drive PDF, which
the build environment could not fetch.

## Tests

- `hy-core` `camera::tests` (19): consent and the off default, an old notice, resume after
  restart (with a hold given first), readings and report checks, stall then restart then
  recover, the second stall stops, stalls more than 10 minutes apart, the Off deadline
  with a forced close, holds, focus not being a hold, conflicts and retries, the start
  timeout, a page gone, a late start, withdrawal, the governor, statuses and serde.
- `hy-core` `coach_engine`: the view reports each hold. `hy-pet` `link`: the shell hears
  each change of the holds once.
- Companion: notices fit a bubble; page reports and commands have the page's shapes; the
  menu view carries the camera entry; the capability files list only registered commands
  (including `camera.json`); and the process tree walk.
- UI (`node --test`, `test/camera-frame.test.mjs`): the valid-frame rule on synthetic
  landmarks (eyes or ears, the visibility floor, out of frame, absent), shell-safe
  reports, the frame timing and timestamps, the consent text and the status words.
- Windows self-test (`src/selftest/camera.rs`, on Chromium's synthetic camera, with no
  image of anyone): consent first; started with the guard; frames read; no person gives no
  reading; the badge; the network refused; a hidden window keeps reading; the budgets
  (informational); stall, restart and recover; the second stall stops; a presentation
  releases and its end restarts; Off within 2 s with the badge gone; denied with the pet
  still running; and the refusal not remembered.

## Needs physical testing

Owner script: `scripts/windows/Test-HealthierYou-Camera.ps1` (run on the office PC as
the office user; it writes a text report and a CSV of measurements, with no names, paths
or pictures).

| Item | Slice | Script phase |
| --- | --- | --- |
| Reading at a real desk with a real person and room light; "no reading" when away or covered | V2-1 / V2-3 | `-Phase Session` |
| Camera budget on the office PC (CPU at most 10%, memory at most 350 MB, memory returned after off) | V2-1 | `-Phase Session` |
| Camera taken by Teams or Camera app; Windows camera access off and back on | V2-1 | `-Phase Session` |
| Meeting and presentation holds with real Teams/Zoom and PowerPoint; lock and sleep | V2-1 | `-Phase Session` |
| Webcam light off within 2 s of Off | V2-1 | `-Phase Session` |
| A second (USB) webcam unplugged and plugged back | V2-1 | `-Phase Session` |
| 2-hour soak: memory growth, no stop for "not responding", holds during normal work | V2-5 | `-Phase Soak -Minutes 120` |
| Calibration quality, floors, drift and the viewpoint cue | V2-2 / V2-3 | `-Phase Calibration` (see `docs/V2_Posture.md`) |
| Desk sessions: 5 setups x 2 sessions, raw-count quality report against the PRD V2 gate | V2-3 | `-Phase Desk`, then `-Phase DeskReport` (see `docs/V2_Quality.md`) |

## Deviations and decisions to confirm

1. **Coaching paused is a camera hold.** Architecture 4.5 lists lock, sleep, off hours,
   presentation and meetings. V2-1 also releases the camera while the user has paused
   coaching, because a paused companion should not be watching. Owner to confirm.
2. **A second stall turns the choice off.** After the camera is stopped for two stalls,
   the setting is off, so it does not restart by itself at the next sign-in. The user
   turns it on again (Try again, the menu or Settings).
3. **No posture events in V2-1.** Nothing about the camera is written to the event log
   yet. V2-2 adds the calibration and cue events, local only.
4. **SIMD runtime only.** Only the SIMD WebAssembly build is shipped (about 13.5 MB
   instead of 26 MB). The bundled WebView2 always supports SIMD; one without it would show
   "Posture model could not load".
5. **The library's usage metrics are blocked, not removed.** The vendor code is not
   patched. The content policy refuses the request, as described above.
