# V2 posture check: calibration, drift and the cue (slice V2-2)

Status: built on top of V2-1 (the camera). Every threshold below is a D value. The V2-3
desk sessions tune them and freeze them before scoring (PRD V2 gate). This page covers
what V2-2 delivers, how it follows architecture 4.5, how it is tested, and what still
needs a person at a real desk.

## What the user sees

- **Camera page > Calibration:** **Calibrate** (later **Recalibrate**) while the camera is
  on. The user sits as they normally do for about 15 seconds while a progress bar counts
  clear frames. The result is either "Calibrated" or "Calibration not usable: <reason>".
  The reasons are:
  - not enough clear frames in 30 seconds
  - out of view
  - too far
  - too close
  - poor light or partly hidden
  - moved during it
  - the camera stopped

  A failed calibration gives no cues. **Cancel** keeps the previous state.
- **Settings > Posture check > Calibration:** the state and when the calibration was
  taken, with a button to the camera page.
- **The cue (pet card):** "Posture check". The text says that for about a minute the user
  has been sitting differently from their calibration and may settle back into a position
  that is comfortable for them, if they like. It offers **OK** and **Recalibrate** (which
  opens the camera page).
  - It compares with the user's own calibration only. It never judges posture and makes no
    health claim.
  - At most one cue every 30 minutes.
  - Dropped during focus, never queued.
  - Suppressed during holds, while another card, an exercise or a Pomodoro is on show, and
    in the calm gap after a card.
  - Closes quietly after 10 minutes. A hold withdraws it, including the end of work
    hours.
- **Camera moved or tilted:** cues pause at once ("Camera position changed? Cues
  paused"). If the view does not match the calibration again within two minutes of
  reading, a notice beside the pet asks once to recalibrate. Nothing is scored until then.

Posture data stays on this PC. Nothing is shared with the office server, and the sharing
switch stays hidden.

## How it works (architecture 4.5)

| Part | Where | What |
| --- | --- | --- |
| Features | `ui/src/camera/frame.ts` (`postureFeatures`), in the pose worker | For a valid frame only:<ul><li>ear-to-shoulder gap / shoulder width</li><li>shoulder width / frame width</li><li>shoulder tilt</li><li>head roll (degrees)</li><li>shoulder midpoint</li><li>head-pair width / shoulder width</li><li>head turn</li></ul>The ears are used when both are seen, else the eyes. Heights are in frame-width units. Hips and legs are ignored. These 9 numbers are all that leave the worker. |
| Detector | `crates/hy-core/src/posture.rs` | Pure rules on synthetic times: calibration quality, floors, smoothing, episodes, the viewpoint check, restart revalidation and records. Features are kept in memory for at most 90 s. |
| Machine | `crates/hy-core/src/camera.rs` | Owns the detector:<ul><li>feeds it frames, stream starts and ends, and reading changes</li><li>runs the page at 3 fps while calibrating</li><li>stores the calibration (numbers only) in the camera setting</li></ul> |
| Cue | `coach/engine/posture.rs` (`Input::PostureCue`) | The engine decides whether the cue shows, as above. The shell asks on the engine thread and hears the answer back (`CueOutcome`). |
| Records | `posture.observation` events (source inferred) | <ul><li>Drift episodes, with the cue outcome: shown, cooldown, focus, held, busy or no answer.</li><li>No-reading intervals (5 s or more), with the reason: absent, low confidence, out of frame or stalled.</li><li>Suspect intervals, with the reason: app restart, camera restart, camera moved or viewpoint changed.</li></ul>All local. |

**Calibration quality (all required):**
- 40 valid frames within 30 s, with at least 60% of frames valid.
- Shoulder width between 15% and 70% of the frame width.
- Mean visibility at least 0.7.
- Each feature's MAD at most 3 times its floor.

**Scale floor:**
- z = (x − median) / max(1.4826 × MAD, floor).
- Floors: gap 0.02, width ratio 0.02, tilt 0.015, roll 2°.
- Width is scored as the ratio to its calibrated median.

**Episode:**
- The features are averaged over 10 s.
- Drift is the largest |z| at or above 3.0 (D).
- An episode starts after 60 s of drift with at least 70% valid frames, and asks for the
  cue at once (within the 30 s gate).
- It ends after |z| stays below 2.0 for 10 s (D), or when capture stops.
- One episode is one event.

**Baseline-suspect (cues paused at once):**
- **(a)** Any camera stream start: after a hold, sleep, a conflict or an app restart.
- **(b)** A step in the shoulders' midpoint of at least 15% of the frame, or in shoulder
  width of at least 20%, completed within 2 s and held for 20 s. Only shoulder geometry
  is used.
- **(c)** The head-to-shoulder width ratio outside its calibration band for 20 s, while
  gap, tilt and roll stay steady and the head faces as at calibration. A turned head
  shows in the turn feature, so head motion alone does not trigger it.

**Leaving suspect:**
- **Revert:** the suspect state clears after 20 s of continuous geometry match (midpoint
  within 7.5% of the frame and width within 10%, D). Only matching frames count: the 20 s
  run is measured from its first to its latest matching frame, a frame that does not match
  ends it, and more than 2 s without a matching frame (no person, invalid or unusable
  frames) starts it again.
- **Invalid:**
  - After 2 minutes of reading without a match, the baseline is invalid. The calibration
    is dropped, and the user is asked once to recalibrate.
  - After an app restart, the stored calibration is accepted only if 80% of the first
    20 s of reading match. Otherwise it is invalid at once.

## Tests

- **`hy-core` `posture::tests`** (23, synthetic traces):
  - Calibration:
    - a usable calibration from a still person
    - each failed check with its reason, including 1 fps for 30 s
    - a failed calibration drops the old one
    - interrupted, and Cancel
  - Floors and drift:
    - **T-C1** a perfectly still calibration bounded by the floors, with no cue in 10 min
      of jitter
    - **T-C2** 30 min of fidgeting with no false cue
    - sustained drift gives one cue and one episode with its outcome, and a later episode
      cues again
    - lean-in, tilt and roll each count as drift
    - **T-C3** reaching, turning to a second display, standing and short lean-ins, ten
      times over, never cue
    - drift with under 70% valid frames is not an episode
  - Viewpoint check:
    - **T-C4** a moved camera is suspect within 20 s, gives no cue, becomes invalid
      after 2 min with one request, then recalibration resumes
    - moved back restores the baseline
    - a step that does not hold, and a slow slide, are not moves
    - a viewpoint change is suspect, but a head turn is not
  - Restarts:
    - **T-C6** a stored calibration is accepted when the first 20 s match and invalid
      when they do not; unusable stored numbers are ignored
    - a camera restart is suspect until the view matches
    - a suspect baseline is restored only by 20 s of continuous match: one match then
      invalid frames, or matches broken by long dropouts, do not restore it
  - Records and limits:
    - stopping records the episode (no answer)
    - no-reading intervals with their reasons
    - at most 90 s of numbers kept
    - features from the page are checked
    - the engine's answers map to cue outcomes
- **`hy-core` camera:** calibration needs capture, runs at the full rate and is stored;
  drift asks for the cue and a hold records the episode; features only on valid frames;
  no-reading records.
- **`hy-core` engine:** the cue shows, then waits out its 30-minute cooldown and closes
  after 10 minutes; focus drops it; a meeting, off hours, a Pomodoro or a reminder
  suppress it; a hold (including the end of work hours) withdraws it, and it is never
  replayed.
- **`hy-core` schema:** reasons and cue outcomes belong to their kinds.
- **`hy-pet` link:** the cue card's words and actions route.
- **UI `node --test`:** features from synthetic landmarks (units, slump, lean, tilt, roll,
  turn, mirror, eyes for ears, nothing else rides along), and the calibration words.
- **Windows self-test** (synthetic camera with no one in view, then synthetic numbers
  with the worker frozen; never an image):
  - a calibration on the real page is refused as out of view, at the full rate
  - a synthetic calibration is usable and stored
  - a sustained drift asks the engine once and records one episode with its outcome
  - a moved camera suspends cues

## Needs physical testing

Owner script: `scripts/windows/Test-HealthierYou-Camera.ps1 -Phase Calibration` (about
15 minutes). The V2-3 desk sessions (5 setups × 2 sessions, raw counts) validate quality.

| Item | Slice |
| --- | --- |
| Calibration usable at real desks: glasses, room light, a laptop camera and an external one; the reasons when not | V2-2 / V2-3 |
| No cue in calm desk work; a cue within about 30 s of a sustained minute of slumping or leaning | V2-2 / V2-3 |
| Laptop pushed back or screen tilted: cues paused within 20 s; recalibration asked after 2 min | V2-2 / V2-3 |
| App restart at the same desk accepted; at a moved desk invalid | V2-2 |
| Detection rate (at least 80% of staged 60 s drifts), false cues (at most 1 per 30 min) and latency, as raw counts | V2-3 (harness) |
| Cue wording and the card's readability and accessibility | V2-2 / UAT |

## Deviations and decisions to confirm

1. **Thresholds are D values**, set from synthetic traces only: z 3.0 to start, 2.0 to
   end, the end after 10 s, the band and match tolerances. V2-3 tunes them.
2. **A fast lean-in held for 20 s can look like a moved camera** (rule (b) uses shoulder
   scale). Cues then pause rather than coach, as the architecture prefers when unsure. If
   the desk sessions show this often, the scale threshold or hold changes.
3. **Daily coverage is not stored separately.** It can be derived from the no-reading and
   suspect intervals plus camera-on time. V2-3's report computes it.
4. **Calibration runs at 3 fps even when the governor chose 1 fps**, for at most 30 s,
   because 1 fps cannot give 40 frames in 30 s.
5. **The cue is suppressed during the calm gap** after another card (no bursts), and
   counted as busy.
6. **Recalibrate opens the camera page.** The user presses Calibrate there and can see
   their progress, so a card click never starts a calibration unseen.
