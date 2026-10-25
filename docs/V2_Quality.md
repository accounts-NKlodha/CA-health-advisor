# V2 posture check: quality (slice V2-3)

Status: built on top of V2-2 (calibration, drift, viewpoint check and cue). V2-3 is the
"harness, labelled sessions, quality report" slice of architecture section 12, plus what
`docs/V2_Posture.md` left to it: tuning the provisional thresholds, deriving a day's
coverage from the records, and the desk-session harness that measures detection rate,
false cues per 30 minutes and latency.

**Everything here was checked on synthetic traces only.** The thresholds stay
**provisional** until the owner's real desk sessions (5 setups x 2 sessions) pass the PRD
V2 gate and the owner approves them (architecture A4: "any threshold change needs the
owner's approval"; PRD: "record and approve changes before using them to declare
success").

## What the user sees

- **Settings > Posture check > Calibration** gains one line: "Today: camera on N minutes;
  a usable reading for P% of it (no reading N minutes, paused or not calibrated N
  minutes); N cues shown." Without enough camera-on time it says so instead of showing a
  zero. It is counts, never a score, and it stays on this PC.
- Nothing else changes for the user. The cue, the camera page and the privacy rules are
  V2-2's.

## How it works (architecture 4.5 "Stored", "Quality report", "Harness")

| Part | Where | What |
| --- | --- | --- |
| Records | `crates/hy-core/src/posture.rs` | Two more `posture.observation` records, additive JSON (no migration): **camera-on time** (`kind: capture`) and **time with no usable calibration** (`kind: baseline_suspect`, `reason: no_calibration`: not calibrated, not usable, being taken, or invalid until the user recalibrates). Two additive fields: a drift episode holds **when its cue was actually asked** (`cue_after_s`, whole seconds after the record's start), and **no-reading spells under 5 s** are added up into one no-reading record (`spells`: how many; `duration_s`: their total; the reason that took most of the time) written once the spells span a minute (`BRIEF_EVERY`), and also when the camera stops, a segment ends, scoring starts or stops, or a long no-reading interval follows. Open intervals (camera on, no reading, suspect, no calibration) are written at least every 15 minutes (`SEGMENT_EVERY`) and start again, so a crash loses little and a day's records stay near their date. A record's time is when it ended (an episode ends when the calm began, 10 s before it is written; added-up spells end when the last one ended), so its start is that time less its length. |
| Thresholds | `posture.rs` `Thresholds` | Every tunable D value in one struct; `Thresholds::PROVISIONAL` is what ships. The architecture's own numbers (floors, 60 s, 70% valid, the step rule, 2 minutes, 20 s after a restart) are not in it: changing them is a design change. |
| A day's coverage | `posture::quality::coverage`, `Store::posture_day` | From that date's records alone: camera on = no reading + not scored + usable, with overlaps counted once. Short flickers count as no reading through the added-up records, so repeated 3 to 4 s dropouts can no longer show as near 100% usable. |
| Quality report | `posture::quality` | Matches the operator's labels with the records and counts, as raw numbers (below). `gate` adds a setup's sessions and checks the PRD V2 gate per setup. |
| Synthetic sessions | `posture::desk` | The desk protocol played through the real camera machine and detector with made-up numbers: 5 setups x 2 sessions, plus a second set of seeds the sweep never tuned on. |
| Sweep | `hy-sim posture --sweep` | The drift and viewpoint grids on both seed sets, one line per setting. |
| Desk command | `hy-companion.exe --hy-posture-report session ...` and `... gate ...` | Read-only on the store: a real session's report from the labels and the records, and the gate over all sessions. |
| Owner script | `scripts/windows/Test-HealthierYou-Camera.ps1 -Phase Desk` / `-Phase DeskReport` | Guides one session step by step and writes the labels; then asks the app for the counts. |

### The desk protocol (one session, about 38 minutes of timed steps)

Calibrate (30 s); comfortable reading (4 min); **forward drift** (90 s); typing (4 min);
second display (75 s); **sideways drift** (90 s); a reach (10 s); typing (2 min); a reach;
**away** (45 s); reading (3 min); **forward drift**; a **lock** (about a minute, which
releases and restarts the camera); typing (3 min); **covered** (30 s); second display;
**sideways drift**; reading (3 min); **a fifth drift** (sideways in session 1, forward in
session 2); typing (2 min); **camera moved** (160 s: pushed back in session 1, tilted in
session 2), then put back and recalibrated; typing (1 min). Each setup's two sessions give
10 staged drifts and about 50 minutes of comfortable work, as the PRD gate asks. Labels
describe movement relative to calibration, never posture quality.

### What the report counts (per session, then per setup)

- **Planned check intervals**: whole 60 s stretches of comfortable or drift labels; one
  has a usable reading when at least half of it had a reading against a calibration that
  holds. PRD: at least 70%.
- **No-reading** time by reason, and **suspect or not calibrated** time by reason.
- **Staged drifts**: detected (the cue, at the time the record says it was asked, within
  30 s of the sustained threshold, which is 60 s after the operator's mark), detected late, missed while suspect or not
  calibrated, missed with no reading, missed otherwise. PRD: at least 10 per setup and 80%
  detected.
- **Threshold-to-cue latency** of each detection, in seconds.
- **Detector episodes**: true (it began no more than 10 s before the operator's mark, and
  its cue was asked during the staged drift or up to 30 s after it), duplicate (a second
  one for the same drift), false (where it began). An episode already running before the
  drift began is false, and that drift is missed, whenever its cue came.
  Reported apart from cues, so the cooldown cannot hide a poor detector.
- **Cues** shown and suppressed by why (cooldown, focus, held, busy, no answer), and
  **false cues** (shown for a false or duplicate episode). PRD: at most 1 per 30 minutes
  of comfortable work.
- **Absence and occlusion** shown as no reading, and cues while unseen (must be 0; judged
  at the time the cue was actually asked, as are cues while moved).
- **Moved camera** paused within 25 s of the mark (with each delay), cues while moved
  (must be 0), and camera-moved or viewpoint suspicion during comfortable work (false
  suspensions, architecture A17).

## Tuning (synthetic only)

`hy-sim posture --sweep` tried z to start {2, 2.5, 3, 3.5, 4} x z to end {1.5, 2, 2.5} x
calm before the end {5, 10, 20 s} (39 settings), then the viewpoint values (head-ratio
band, turn tolerance, match tolerances; 27 settings), each on the tuning seeds and on the
check seeds.

**Rule:** pass the synthetic gate on both seed sets; then fewest false cues and false
episodes; then most detections; then the smallest change from V2-2's values.

| Setting (z start / z end / calm) | Tuning set: detected, false cues, false episodes, setups passing | Check set: same |
| --- | --- | --- |
| 2.0 / 1.5 / 10 s | 42/50, 1, 2, 5/5 | 38/50, 4, 4, 2/5 |
| 2.5 / 2.0 / 10 s | 45/50, 0, 0, 5/5 | 42/50, 2, 3, 4/5 |
| 3.0 / 1.5 / 10 s | 40/50, 0, 0, 3/5 | 37/50, 1, 1, 1/5 |
| 3.0 / 2.0 / 10 s (V2-2) | 45/50, 0, 0, 5/5 | 41/50, 1, 2, 4/5 |
| **3.0 / 2.5 / 10 s (V2-3)** | **46/50, 0, 0, 5/5** | **43/50, 1, 2, 5/5** |
| 3.5 / 2.5 / 10 s | 40/50, 0, 0, 4/5 | 39/50, 0, 0, 4/5 |
| 4.0 / 2.5 / 10 s | 35/50, 0, 0, 2/5 | 35/50, 0, 0, 1/5 |

Median latency was 6 s for the chosen setting on both seed sets. The calm before the end
(5, 10 or 20 s) made almost no difference, so it stays at 10 s.

**Re-run after the review fixes** (recorded cue time, the episode-onset rule, short
no-reading spells counted): every count in the table is unchanged on both seed sets, and
3.0 / 2.5 (any calm) is still the only drift setting that passes both; only median
latencies moved, by at most 1 s, because the report now uses when each cue was asked
instead of the start plus 60 s. The shipped provisional thresholds still pass; nothing was
changed.

**Result: one change, the end threshold from 2.0 to 2.5.** Why: after a drift a person
often settles about two floors from calibration; with 2.0 the episode outlasts the drift
and the next staged drift starts no new episode, so it is missed. With 2.5 episodes end
when the person is back in the band of comfortable stances. The start threshold stays at
3.0: lower values found more drifts on the tuning seeds but gave false cues on the check
seeds, and higher values missed too many.

**The viewpoint values did not move any count** on synthetic sessions (all 27 settings
gave the same numbers), because the synthetic moves are clear steps and the synthetic head
turns are clearly turned. They stay as shipped; only the real relocation trials (A17) can
tune them.

**What this does and does not show.** The synthetic sitting model (settling, slow
wandering, noise, glances, reaches, five setups) was written by hand. A pass shows that
the harness counts correctly and that the thresholds hang together on plausible traces.
It does not show that the check works for real people, cameras or light. The test
`posture::desk::tests` keeps the shipped values passing on both seed sets, and shows the
V2-2 end threshold failing the check set.

## Tests

- **`hy-core` `posture::quality::tests`** (14): labels read and refused; interval
  arithmetic; a day's coverage adds up with overlaps counted once, and says nothing
  without camera-on time; stored rows become intervals (other types and unknown kinds are
  skipped); an observation handed over late ends when it ended and keeps its recorded cue
  time (records from before it fall back to start + 60 s); **an episode already running
  before a staged drift is a false episode and the drift missed, with no negative
  latency**, while one that began a few seconds before the mark is its detection; the
  log read from a real
  store file (window and clipping); a hand-made session counted (detected, late, missed
  with its cause, duplicate and false episodes, cues and false cues, absence, occlusion,
  moved camera, false suspension); the gate per setup (coverage, detection, false-cue
  allowance, no reading, moved camera, incomplete sets) and what it refuses; JSON round
  trip; the command end to end (a store, labels, a session report, then the gate; bad
  arguments; a missing store).
- **`hy-core` `posture::desk::tests`** (5): the shipped thresholds pass the synthetic gate
  on both seed sets; V2-2's end threshold fails the check set; a session follows the
  protocol, repeats exactly, and its coverage adds up; **the cue time in a record is
  within 0.6 s of when the machine asked** at 3 fps and at 1 fps; the grids hold only
  valid thresholds.
- **`hy-core` `posture::tests`** (7 more): **a cue held back by 50 s without a reading
  (asked about 115 s after the drift began) is recorded at that time and counted late,
  not as a 9 s detection**; **short no-reading spells with alternating reasons are added
  up, written about once a minute, every second counted**; an episode that ends in calm
  is handed over with when it ended; open intervals are written every 15 minutes and start again; a
  suspect baseline too; time without a usable calibration is recorded until
  recalibration; thresholds are checked, bad ones fall back, and a higher start threshold
  ignores a drift.
- **`hy-core` `camera::tests`:** **ten minutes of 3 s dropouts (away and poorly visible
  in turn) and 2 s readings through the real machine**: at most one added-up record a
  minute, their total within 10 s of the time the machine showed no reading, no planned
  minute usable in the session report, and under 50% usable in the day's coverage read
  from a store.
- **`hy-core` store and schema:** a day's coverage from the store; capture and
  no-calibration records belong to their kinds; the cue time and spell count belong to
  their kinds. Camera and posture tests updated for the new records.
- **Companion:** the `--hy-posture-report` command is routed and refuses bad arguments;
  the Windows self-test checks that the time before a usable calibration is recorded and
  that the page reads today's coverage (`posture_coverage_from_records`).
- **UI `node --test`:** today's coverage in words (counts only, nothing without data).
- **Script:** `Test-HealthierYou-Camera.ps1 -SelfTest` (the protocol, the label format, no
  judgement in the instructions, and a report in a folder that does not exist yet: the
  folder is made and the files beside the report are written), on PowerShell 7 (Linux job) and Windows PowerShell 5.1.

## Needs physical testing

Owner script, one command per session (about 45 minutes with setup):

```powershell
.\Test-HealthierYou-Camera.ps1 -Phase Desk -Setup 1 -Session 1 -Report .\desk\hy-desk-1-1.txt
# ... setups 1 to 5, sessions 1 and 2 ...
.\Test-HealthierYou-Camera.ps1 -Phase DeskReport -Report .\desk\hy-desk-gate.txt
```

The script creates the report's folder (`.\desk` above) if it is not there. It records only the times of the steps (seconds and a word) and the counts HealthierYou
derives from its own records: never a frame, a landmark or a posture number, and no name,
path or device id. Read the files before sharing.

| Item | Gate |
| --- | --- |
| 5 setups x 2 sessions at real desks (laptop and external cameras, glasses, room light, one slow PC if available) | PRD V2 gate |
| Usable reading in at least 70% of planned check intervals per setup | PRD |
| At least 80% of staged 60 s drifts detected, with the cue within 30 s of the sustained threshold | PRD |
| At most 1 false cue per 30 minutes of comfortable work per setup; detector false episodes reported | PRD |
| Away and covered shown as no reading, with no cue | PRD |
| Moved camera paused within about 20 s, then recalibration asked after 2 minutes; false suspensions reported | Architecture T-C4, A17 |
| Today's coverage line reads sensibly after a working day | UAT |

If a setup fails, the counts show where (coverage, a drift kind, false episodes in a
label, suspensions). Any threshold change that follows needs the owner's approval, and
the values are frozen before the scoring sessions are run again.

## Deviations and decisions to confirm

1. **The end threshold changed from 2.0 to 2.5** on synthetic evidence only (above).
   It stays provisional; the owner approves it or another value before the scoring desk
   sessions.
2. **"Suspended within 20 s" is checked as within 25 s of the operator's mark.**
   Architecture 4.5 holds a step for 20 s, after a step of up to 2 s and the 1 s window
   that sees it, so a pause at 20 to 23 s after the mark is the design working. Each delay
   is reported, so the owner can judge.
3. **Time with no usable calibration is a `baseline_suspect` record with reason
   `no_calibration`.** Architecture 4.5 reports "baseline-suspect or invalid intervals";
   invalid (until recalibration), not calibrated and calibrating are counted there, so a
   day's coverage needs no new kind for them. Camera-on time is a new kind, `capture`.
   Both are additive JSON in the existing event log (migrations 0001 to 0006 unchanged),
   as are the two optional fields `cue_after_s` (drift episodes) and `spells` (added-up
   no-reading records). An older build ignores records it cannot read.
4. **A day's coverage is attributed to the date a record ended.** Intervals are written
   at least every 15 minutes, so at most 15 minutes of an interval that crosses midnight
   land on the next day.
5. **An episode belongs to a staged drift when it began no more than 10 s before the
   operator's mark and its cue was asked during the drift or up to 30 s after it.** The
   10 s allows for a mark that trails the movement. An episode already running before a
   drift began (it started from a comfortable stance) counts as a false episode where it
   began, and the drift it ran into as missed.
6. **Short no-reading spells (under 5 s) are written added up**, one record when they
   span a minute (or earlier, when capture stops, a segment ends or scoring starts or
   stops), placed so it ends when the last spell ended, with the reason that took most of
   the time. Coverage counts their full time; within a minute their position is
   approximate, so a check interval's share can be off by up to that minute's spells.
   Each spell does not become its own event.
7. **Check intervals are 60 s, usable at half or more**, and absence, occlusion, a moved
   camera, a lock and calibration are not planned check time (they test the no-reading,
   suspension and hold paths and are counted separately).
8. **The architecture's 5 setups x about 70 minutes** becomes two sessions of about 38
   minutes of timed steps each, to reach the PRD's 10 staged drifts per setup with about
   50 minutes of comfortable work.
