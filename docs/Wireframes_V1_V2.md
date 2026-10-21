# HealthierYou wireframes: V1 and V2

Status: **draft for design review**, revision 2 (21 Oct 2026). It answers Codex's review of PR #4. Revision 1 was 20 Oct 2026. For Codex review and then the owner's explicit approval. **Not approved. Product development is not approved.** Nothing here is product code.

- **Clickable prototype:** [`docs/prototype/index.html`](prototype/index.html). It is one self-contained file: open it from disk in any current browser. It makes no network calls, saves nothing to disk (changes live in memory only and are lost on reload), and never uses the camera. A banner on every screen says: *"Simulated prototype for design review. Not the product. No data is saved or sent."*
- **Screenshots** (1366x768): [`docs/prototype/screens/`](prototype/screens/).
- **Headless check:** [`docs/prototype/verify_prototype.cjs`](prototype/verify_prototype.cjs) (section 9).
- **Baseline:** the approved PRD 0.3 ([`PRD_0.3.md`](PRD_0.3.md)) and the architecture proposal, revision 3 ([`Architecture_V1_V2.md`](Architecture_V1_V2.md)) plus the recovery rule being added to its section 5.3.
- **Scope:** V1 and V2 only. V3 (hosted accounts, watches, avatars, groups and rankings) is deferred. It appears only as greyed "coming later" items: the hosted option in setup, a disabled settings tab and a disabled entry in the flow index. V4 sound, headphone and colour sessions are not shown.
- **Draft pending contract:** a parallel revision is still finalising the sharing, deletion, recovery, pairing and backup contracts. Every screen that depends on them carries a **Draft pending contract** label, both here and in the prototype. Their wording and fields can change without changing the layout. Revision 2 reconciles these screens with architecture revision 3 (full certificate fingerprint check is authoritative, six words are auxiliary; per-device summaries with no within-day timing; fail-closed server recovery) but they stay labelled Draft until the contract text is final.
- **Proposed defaults:** a few defaults come from the reviewer's recommendations. They are **not owner answers**. Each one is labelled **Proposed default, owner to confirm** in the prototype and listed in section 8.

## 1. Design rules

1. **The pet comes first.** The pet is a window of about 128 px (scalable from 75 to 150%). It is transparent, always on top and **never takes keyboard focus** (`WS_EX_NOACTIVATE`). Everyday actions (water, movement, focus, Pomodoro, snooze, phone note) happen in **one bubble** next to the pet. Ordinary windows are only for setup, settings, goals, sharing, backup and reviews.
2. **One bubble at a time.** Due needs are combined into one reminder that states its reasons (RHY-02). A snooze covers the whole reminder.
3. **The user is always in control.** Every prompt has a quiet way out (Snooze, Not now, Skip, Stop). A dismissal never counts as completion. There is no guilt, no sad pet and no streak punishment. After three dismissals in a day, the app offers fewer reminders.
4. **Honest data.** Labels such as *self-reported*, *estimate*, *no data* and *measured zero* are visible wherever figures appear. Missing days never look like zero.
5. **Status never depends on colour alone.** Every pet state has an icon badge and a text tag. Charts use the Okabe-Ito palette together with patterns (hatched means no data, dashed means partial or in progress).
6. **Calm office style.** Neutral greys, one teal accent, system font (Segoe UI), no sound, and slow motion that can be switched off.

## 2. Screen inventory

Surfaces: **P** = pet window, **B** = bubble beside the pet, **W** = HealthierYou window, **M** = modal dialog over a window, **A** = server PC page, **R** = reviewer-only aid (not part of the product).

| ID | Surface and screen | Purpose | PRD IDs | Prototype flow (index no.) |
| --- | --- | --- | --- | --- |
| P0 | Pet window (eight states plus suppression states) | Always-present companion; status at a glance | PET-01 to PET-04, RHY-03 | 2, 3 |
| T1 | Tray menu | Show or hide the pet, pet panel, pause, settings, reviews | PET-04, RHY-03 | 2 (tray icon) |
| B1 | Pet menu (expanded bubble) | Hub for all everyday actions | PET-03, WAT-01, MOV-02, RHY-03, FOC-01, DIS-01, REC-01 | 2 |
| K1 | Pet panel (keyboard path, Ctrl+Alt+H) | Same as B1, but takes focus on purpose; Esc returns focus | PET-04 | 16 |
| B2 | Combined reminder card | One prompt with explicit reasons; done, water, snooze, dismiss, focus | RHY-02, RHY-03, WAT-02 | 4 |
| B3 | Water confirmation and water panel | Quick and custom log, undo, edit, remove | WAT-01, WAT-02 | 5 |
| B4 | Movement guide (intro, step, result) | Day's routine, seated or standing, step done, skip, stop, partial | MOV-01, MOV-02 | 6 |
| B5 | Focus chooser, pause chooser, meeting toggle | Suppress prompts on request | RHY-03, RHY-04 | 7 |
| B6 | Pomodoro chooser, controls, break card | User-started timer with combined break | FOC-01 | 9 |
| B7 | Deadline check-in, recovery invitation, recovery morning, reflection | Recovery-day journey | REC-01 | 8 |
| B8 | Phone interruption note | One-tap self-report with optional label and undo | DIS-01 | 10 |
| B9 | Weekly review offer | Optional review at the chosen time | REV-01 | 11 |
| B10 | Offline notice | Server unreachable; everything local continues | LAN-01, SHR-02 | 15 |
| B11 | Posture cue, "camera moved?", recalibrate request, camera conflict (V2) | Baseline-relative cue and camera states | CAM-01, CAM-02 | 18 |
| B12 | Meditation invitation and session (V2) | Text-guided pause | CAL-01 | 19 |
| W1 | Setup wizard (8 steps) | First run | First-use journey; PET-01/02, WAT-01, RHY-03, SHR-01 | 1 |
| W2 | Pairing (setup step 2; also opened from W8) **Draft pending contract** | Optional LAN pairing with a slip | LAN-01 | 1 |
| W3 | Settings: Pet | Ten-pet library; name, colour, size, edge, tone, motion; reset | PET-01, PET-02 | 2 |
| W4 | Settings: Reminders | Intensity, categories, quiet rules, meeting detection | RHY-02 to RHY-04 | 7 |
| W5 | Settings: Water and workweek (goals) | Water goal, serving buttons, workdays, hours, review time, time zone | WAT-01, WAT-02, MOV-01, REV-01 | 1, 16 |
| W6 | Settings: Movement programme | Monday to Saturday matrix (provisional content, pending content review), default version | MOV-01 | 6 |
| W7 | Settings: Focus and Pomodoro | Presets, auto-restart (off) | FOC-01, RHY-03 | 9 |
| W8 | Settings: Sharing and server **Draft pending contract** | Status, category toggles, preview, delete, operational data, sync, unpair | SHR-01, SHR-02, LAN-01 | 13, 15 |
| W9 | Settings: Backup and update **Draft pending contract** | Export, import, update from file, rollback | PRD section 9 | 14 |
| W10 | Settings: Posture check (V2) | Status, set up, recalibrate, off | CAM-01, CAM-02 | 18 |
| W11 | Settings: Meditation (V2) | Invitation time (off until set) | CAL-01 | 19 |
| W12 | Settings: Accessibility | Text size, reduced motion, contrast, keyboard map | PET-04 | 16 |
| W13 | Weekly review | Successes, missed plans, coverage, up to 3 suggestions, phone self-reports | REV-01, DIS-01 | 11 |
| W14 | Monthly progress | Monthly rates with coverage, baseline, day strip, table alternative | REV-02 | 12 |
| W15 | Recovery-day plan | Plan items; water unchanged; save, change date, cancel | REC-01 | 8 |
| W16 | Posture check consent and calibration window (V2) | Consent, access outcomes, calibration quality, saved | CAM-01 | 18 |
| M1 | Preview and start sharing **Draft pending contract** | Exact fields, today's record, audience wording | SHR-01 | 13 |
| M2 | Sharing stopped, delete what I shared, receipt, queued deletion **Draft pending contract** | Stop and delete as separate steps | SHR-02 | 13, 15 |
| M3 | Switched off on another PC (stale enable) **Draft pending contract** | No silent re-enable from a second PC | SHR-02 | 13 |
| M4 | Export, import, update from file, rollback **Draft pending contract** | Local lifecycle without a server | PRD section 9 | 14 |
| M5 | Unpair, revoke confirmations | Destructive confirmations | LAN-01 | 13, 17 |
| A1 | Server admin page (server PC, localhost only) **Draft pending contract** | Health, devices, slips, updates, audit log; no habit data | PRD section 10 flow 12, LAN-01 | 17 |
| R1 | Pet state gallery (reviewer aid) | All states side by side | PET-03 | 3 |
| R2 | Flow index and caption panel (reviewer aid) | Jump to flows, see the current step and PRD IDs | n/a | always |
| V3 | Greyed placeholders: hosted account option, "Watches and community" tab | Show deferral honestly | CON-01/02, COM-01/02 (deferred) | 20 (disabled) |

## 3. Screens: content, states and actions

Common states, which apply wherever relevant:

- **Empty:** explains what will appear and when. It never shows a zero chart.
- **Error:** says what happened, that nothing changed, and the next step.
- **Offline or server unreachable:** local features continue. A plug badge appears on the pet. The sharing screen shows the waiting count and the retry status.
- **Camera denied, busy or absent:** an explicit unavailable state. No score is inferred, and V1 features continue.

### P0 Pet window

- **Content:** the pet's art (a data-only pack; the prototype draws 4 placeholder SVG pets), a top-right **icon badge** and a bottom **text tag**. In V2, a "CAM" badge appears at top left while the camera is on.
- **Interaction:**
  - A click opens the bubble without taking focus.
  - A drag moves the pet. It snaps to the nearest left or right edge, or stays where dropped if the edge is set to "where I drop it".
  - The pet and all bubbles hide completely during presentations and full-screen apps, the tray stays, and the pet is restored quietly (**Proposed default, owner to confirm**).
- **States:** see section 4.
- **Error states:** if the pet's window would be off-screen after a monitor is unplugged, it is repositioned to the primary monitor's default edge (PET-04).

### B1 Pet menu and K1 pet panel

- **Content:**
  - greeting with the pet's name;
  - today's water against the user's own goal;
  - serving buttons, plus "Water..." for the full panel;
  - Today's movement, Focus..., Pomodoro...;
  - Phone interruption, I'm in a meeting, Pause...;
  - Deadline day, Posture check (V2, or "Camera off" when on), Reviews, Settings.
- **States:** paused gives a short "Resume coaching" bubble first. Focus shows the time left and "End focus". Offline shows B10 first, with "Open pet menu". A running Pomodoro shows the B6 controls.
- **K1:** Ctrl+Alt+H (configurable) opens the same content with keyboard focus on the first action. Tab moves between actions. Esc closes the panel and focus returns to the previous app.

### B2 Combined reminder card

- **Content:** a title and a reasons list, for example: "48 min of work since your last break", "Eye rest due, folded into this break", "Water: 140 ml behind your own pace (about 390 ml expected by now)". The water reason appears only when the logged amount is at least 100 ml below the amount expected by now (the user's own goal spread evenly over the user's own work hours). The prototype states its sample scenario in the caption: 250 ml logged at 11:15 against about 390 ml expected. A footnote says that one snooze covers all of them.
- **Actions:** Start 2-min movement (primary), Log 250 ml (strikes through the water reason), Already did it, Snooze 10 min, Focus..., Not now.
- **States:**
  - **No response for 10 min:** collapses to a "1" badge and is logged as *offered, no response*.
  - **After snooze:** the same single card returns once.
  - **Third dismissal:** offers to switch to Light.
  - **Category switched off:** that reason is never listed.

### B3 Water

- **Quick log:** a confirmation reading "Logged 250 ml, 750 / 2000 ml", with Undo and Details.
- **Panel:** a progress bar with a text value, serving buttons, a custom amount (1 to 2000 ml; validated) and today's entries with Edit and Remove. Corrections are stored as superseding entries, and totals recalculate.
- **States:**
  - **Empty:** "No entries yet today. That means nothing logged, not zero water."
  - **Goal reached:** "Your logging goal is reached; hydration reminders are paused." Manual logging stays available (serving buttons, custom amount, Undo and Details remain). There is no further encouragement (WAT-01).

### B4 Movement guide

- **Intro:**
  - the day and its body region, with "3 steps, about 2 min";
  - a notice that the content is provisional, pending content review;
  - a seating check ("if your chair has wheels, choose standing or lock them");
  - stop and skip wording;
  - buttons: Seated, Standing, Later, Skip today, plus links to other days.
- **Step:** a neutral human illustration with seated and standing variants, the step text, Done, Skip step and Stop.
- **Result:**
  - "Routine complete, self-reported", or
  - "Saved as partial: 2 of 3 steps, 1 skipped", which is never counted as complete. There is no catch-up.
- **Sunday:** "Sunday is optional", with a free choice of any day's routine.

### B5 Focus, pause and meeting

- **Focus:** 30, 60 or 90 min or custom. The pet's eyes close and its tag shows the time left. When focus ends, nothing is replayed.
- **Pause:** 30 min, 1 hour, rest of today or until I resume. Pause has the highest precedence.
- **Meeting:** a manual toggle. The pet dims and shows a mute badge with a "Meeting - paused" tag.
- **Presentation or full screen** (**Proposed default, owner to confirm**): detection hides the pet **and every bubble** completely. The tray icon stays, and the tray's "Show pet" ends the hidden state. When the presentation ends nothing is replayed: no burst of held reminders and no bubble. The prototype states that detection is imperfect.

### B6 Pomodoro

- **Chooser:** says the timer is off until Start. Presets 25/5, 50/10 or custom, and a visible "Auto-start next round" switch (off).
- **Guards:**
  - **Focus is on:** "Replace quiet focus?" with Replace or Cancel.
  - **Coaching is paused:** "Resume coaching for this session?"
- **Running:** the pet's tag shows "Work 24:59". A click gives Pause, Resume and End.
- **Break card:** look away, move, and water only if the user is genuinely behind their own pace for the time of day (a daily total below the goal is not enough); leaving the desk is encouraged. Actions: Start break, Skip break (recorded as declined), End.
- **After a lock or sleep:** "Pomodoro paused. Resume or End." No catch-up rounds.

### B7 Recovery day

- **Check-in** (**Proposed default, owner to confirm**: optional and configurable, 9 hours by default): "This PC has been active for about 9 hours today. That is an estimate of PC activity, not a sign of stress or tiredness. Would you like to mark today as a deadline-heavy day?" Options: Yes, No, Don't ask today. Settings, Reminders has a switch and an hours choice (8, 9 or 10). With the switch off, nothing is offered automatically and "Deadline day" on the pet menu still works. The app does not infer stress.
- **Invitation:** Tomorrow, Choose a date, Later, No thanks. It says the plan is not a leave request.
- **Recovery morning:** the day's plan, with "Your water goal stays 2000 ml".
- **End of day:** "Did this routine feel helpful?" Yes, Somewhat, No or Skip. The answer stays on this PC.

### B8 to B12

- **B8 Phone note:** "Phone interruption noted, 11:52", optional work, personal or unsure, and Undo. Corrections are also possible in W13.
- **B9 Review offer:** Open, Later (offered once on the next workday), Skip this week.
- **B10 Offline:** "Can't reach the office server since 10:05. Reminders, logging and reviews keep working. 3 summaries waiting; sent when it's back, only for categories you still share." Actions: Details (W8), OK.
- **B11 (V2):**
  - **Cue** (**Proposed default, owner to confirm**): "Your position has changed from your calibration. Would you like a short movement break?" with Thanks, Snooze 30 min, Turn camera off. It adds "compared with your calibration, not a health score". The wording is neutral by default. Directional detail (for example which way) is shown only after the detector passes a quality gate in desk testing.
  - **"Did the camera move?":** cues are paused; Recalibrate now, Later, Off.
  - **Recalibrate request:** asked once.
  - **Camera taken by another app:** posture check is paused and V1 continues.
- **B12 (V2):** the invitation offers 2, 5 or 10 min, or Not today. The session shows five short text parts, Next, Pause and Stop, and a link to switch between the breath-focused and the eyes-open, non-breath versions. There is no sound and no backlog.

### W1 Setup wizard (8 steps)

1. **Welcome:** what the app does and does not do. It works without a server, has no sound and never blocks input.
2. **Connect (optional):** Use on this PC only (default), Pair with the office server (W2), or Hosted account (V3, greyed: "coming later").
3. **Pet:** W3 content (the ten-pet library and edits).
4. **Workweek:** workdays (Monday to Saturday on; Sunday optional), hours, weekly review time and time zone (Asia/Kolkata).
5. **Water:** the user's own goal ("never suggested or raised") and serving buttons.
6. **Reminders:** W4 content.
7. **Sharing:** if paired, the categories, all off, with Preview (M1) and the audience text. If not paired: "nothing is shared".
8. **Try a reminder:** a sample bubble, then Finish. The pet appears at its edge and celebrates once.

**States:** the Back button keeps earlier choices, and every step can be completed without a server.

### W2 Pairing **Draft pending contract**

- **Content:**
  - a field for the slip text, `hy1-<host>-<port>-<52-char fingerprint>-<16-char secret>` (the prototype shows sample values only);
  - an explanation: single use, valid 24 h, holds the full certificate fingerprint, so a fake server is refused before anything is sent;
  - after the server is found, the result states that the **full certificate fingerprint matched** the slip. That full check is **authoritative** and runs before the secret is sent. The **six words** ("amber, cedar, lotus, river, maple, quartz") are shown only as an **auxiliary aid** for people and are labelled "not the verification". Buttons: Pair this PC, and "The words differ - stop".
- **States:**
  - **ok:** paired. The message lists the operational data that is always sent and says health sharing happens only for chosen categories.
  - **Fingerprint mismatch:** "Stopped... the app ended the connection before sending the slip's secret, so nothing was sent... ask for a new slip."
  - **Used or expired:** asks for a new slip.
  - **Server unreachable:** retry, or finish setup and pair later.
  - **Malformed:** describes the expected format.
- **Actions:** Check slip, Skip pairing for now.

### W3 to W7, W10 to W12 Settings

- **W3 Pet:**
  - a 5x2 library grid; the 6 pets without art are marked "art pending". A note separates two decisions: **visual-family approval** (do the placeholders point the right way?) and **the ten finished pets** (4 drawn, 6 art pending). Approving the family does not approve any pet as finished;
  - name (up to 20 characters), colour variants (3 per pet), size 75 to 150%;
  - edge: left, right, or where I drop it;
  - tone: Gentle, Plain or Cheerful;
  - a Motion switch and Reset to defaults.
  - Edits apply instantly.
- **W4 Reminders:**
  - intensity: Light, Balanced or More often, with the Balanced values shown as "starting values to test, not medical advice";
  - category switches: movement, eye rest, water;
  - quiet rules: off-hours (always on, shown as a fixed switch), presentation and full screen (on, "may not detect every meeting"; **Proposed default, owner to confirm:** pet and bubbles hidden completely, tray stays, no replay), camera or microphone in use (experimental, off);
  - the afternoon reminder time (changed by a weekly-review suggestion) and the optional long-day check-in switch and hours (**Proposed default, owner to confirm**).
- **W5 Water and workweek:** goal, serving buttons, pacing switch, workdays, hours, review time and time zone. The hours and review time are bound to the in-memory state: reopening the tab shows the saved values. "Goal changes apply from today; past weeks are judged against the goal you had then."
- **W6 Movement:** the Monday to Saturday matrix (**provisional content, pending content review**, labelled so on the tab, in the bubble and in the coverage table), Sunday optional, and the default version (ask, seated or standing), which a weekly-review suggestion can change and Undo restores.
- **W7 Focus and Pomodoro:** a description of the presets and the auto-restart switch (off).
- **W10 Posture check (V2):** status (Off, On, On - no reading, Paused - camera moved?, Blocked by Windows, In use by another app), Set up, Recalibrate, Turn camera off, and the privacy note.
- **W11 Meditation (V2):** an invitation time selector that is off until set.
- **W12 Accessibility:** see section 6.

### W8 Sharing and server **Draft pending contract**

- **Status line:**
  - Connected, last sync 11:40, n summaries waiting; or
  - Can't reach the server since 10:05, retrying; or
  - Not paired, with "Pair with office server...".
- **Optional categories** (all off): hydration, reminder and exercise outcomes, work-session summary, phone self-report counts, and posture summary (V2). Each has a switch, Preview (M1) and Delete what I shared (M2). Work-session fields are `{date, origin device, estimated active minutes, longest block estimate, focus and Pomodoro counts}` with **no within-day timing**; there are no timing bitmaps or slot fields in any category.
- **Per device:** shared summaries are kept per device. With two PCs each sends its own, and durations are never added together across PCs.
- **Deleted dates:** a list of ranges the user deleted, each marked "never uploaded again", and "includes today, so nothing more is uploaded for today" where that applies.
- **After a server restore:** a banner says either that history was verified, or (fail closed) that shared history was discarded, this PC was disconnected, every category is off and the user must pair again (see A1).
- **Never sent:** meditation, recovery plans, reflections, individual events, camera images.
- **Always sent while paired (not health data):** device ID, alias, app version, Windows build, last seen, pairing state, policy sequence number.
- **Actions:** Sync now, Unpair this PC (M5), and a conflict example (M3).

### W9 Backup and update **Draft pending contract**

- Cards for Export my data, Import a backup, Install update from file, and Install previous version. Each card says what is included and what is not; pairing is excluded from backups.

### W13 Weekly review

- **Content:**
  - the week and its coverage ("Recorded on 6 of 6 workdays");
  - What went well;
  - Planned but missed;
  - For next week: up to 3 suggestions, each with Apply or Not for me. **Apply really changes the setting** in the prototype's state (afternoon reminder time, a 750 ml serving button, or the default movement version). The row then reads "Applied" with what changed, and **Undo restores the exact prior value**;
  - phone interruptions (self-reported, editable labels);
  - a "how these are worked out" definition line.
- **Variants:**
  - **Strong week**;
  - **Mixed week:** "On 2 days nothing was logged (no data, not zero)";
  - **Not enough data:** "Missing days are not counted as misses", and no suggestions;
  - **First week:** "Comparisons start after two full weeks".

### W14 Monthly progress

- **Metric chips:** water goal days, routines completed, break follow-through. Each is a rate per eligible day.
- **Bar chart:** each bar has a value and a "13 of 21 days" detail. Measured zero is drawn as an outline at 0%, no data as a hatched full-height bar marked "no data", and the month in progress as a dashed outline. A dashed orange baseline marks the user's first 4 weeks.
- **October day strip:** each day shows a symbol: goal met, some logged, 0 logged, no data, off, or future.
- **Table alternative:** "Show as table".
- **Empty variant:** "Not enough data yet. Your first monthly chart appears after two weeks of use."

### W15 Recovery-day plan

- **Plan items, each with a switch:**
  - more short breaks;
  - optional 10 to 15 min walk at a chosen time, with optional repeat every 90 min;
  - lunch away from the screen;
  - a brief calming pause;
  - a finish time;
  - one end-of-day question.
- **Fixed note:** water stays at the user's own goal; today's routine is reused and nothing extra is owed.
- **Actions:** Save plan, Change date, Discard changes, and Cancel plan once a plan is saved. Only one pending plan is kept.
- **State:** the window edits a draft held in the prototype's state. The switches, the walk and finish times and the repeat box all bind to it. **Save plan** copies the draft into the saved plan, and reopening ("Edit plan" on the recovery morning) shows the saved values. **Discard changes** leaves the saved plan as it was. The recovery-morning bubble lists only the items that were saved.

### W16 Posture check window (V2)

- **Consent:** what it does and does not do: not medical, frames stay in memory and are never saved or sent, the CAM badge is shown while on, everything else works without it. Actions: Turn on camera, Not now.
- **Access outcomes:** allowed leads to calibration. The others are:
  - **Denied by Windows:** link to camera settings (simulated);
  - **Busy:** waits, and nothing is measured;
  - **No camera.**
- **Calibration:** a placeholder silhouette with a shoulder guide line, a "Camera on" label, "Simulated view - no camera is used in this prototype", a progress bar and an instruction to hold still for about 20 s.
  - **Usable:** "Saved as a few numbers... no images", with cue rules (about a minute of sustained change, at most every 30 min, quiet in focus, meetings and off-hours).
  - **Not usable:** the reason ("Both shoulders weren't fully in view"), Try again or Off. No cues are given without a usable calibration.

### M1 to M5

- **M1 Preview and start sharing:** destination, the field list, today's actual record, what is never sent, and the audience text (plus a line that summaries are kept per device, durations are never added across PCs, and no within-day timing is shared): *"Who can see this: you, in HealthierYou. The app has no manager charts, but someone with admin access to the server PC could technically open the unencrypted shared summaries."* It also notes that sharing starts from today and that deleting is a separate step.
- **M2 Sharing:**
  - **Sharing stopped:** "n unsent summaries discarded. What you already shared stays on the server until you delete it." Then Delete what I shared, or Keep it.
  - **Delete, step 1:** "All dates" or "A date range". Choosing a range shows **From** and **To** date inputs (no future dates; From after To is refused with a message). The text distinguishes the active store from backups, which expire within 30 days, and says that deleted dates can never be uploaded again.
  - **Delete, step 2 (confirmation):** names the category and the chosen dates (for example "12 Oct 2026 to 16 Oct 2026" or "all dates"), says it cannot be undone, and says whether today is included. Buttons: Yes, delete; Change dates; Cancel.
  - **Receipt:** "Receipt: Hydration summary, 12 Oct 2026 to 16 Oct 2026, removed from the active store today at hh:mm". "All dates" appears **only** when all dates were chosen. A range that includes today adds: "Today is included, so nothing more is uploaded for today; sharing continues from tomorrow if it is on."
  - **Deletion semantics kept visible:** deleted dates can never be uploaded again (permanent tombstones, architecture 5.3). A deletion that includes today prevents today's re-upload even if the user switches sharing off and on again: the preview says "Not uploaded: you deleted today for this category" and 0 summaries wait. The Sharing tab lists the deleted ranges.
  - **Offline:** "Deletion queued... waiting, retrying", with the chosen range named; uploads for those dates are already blocked on this PC.
- **M3 Stale enable:** "Switched off on your other PC on 19 Oct. Switch on again? Sharing restarts from today; earlier dates are not sent."
- **M4 Lifecycle:**
  - **Export:** password, contents, then "saved (simulated)".
  - **Import:** a "replaces, never merges" warning. Outcomes: ok, wrong password, newer version, damaged file; each error says "Nothing was changed".
  - **Update from file:** installer plus `.sig`; either "signature verified, installs at next quiet moment" or "refused".
  - **Rollback:** a one-release-back warning about data recorded since the update, with Export first.
- **M5 Unpair and revoke:** consequences stated, Cancel available.

### A1 Server admin page **Draft pending contract**

- **Banner:** "No habit data or charts are available on this page."
- **Content:**
  - service status, last backup and database size;
  - devices (alias, version, last seen, state, Revoke);
  - Create pairing slip: shows a sample slip with the full fingerprint (52 characters) and secret (16 characters), single use and 24 h validity, and Copy or Print. The 6 words are labelled auxiliary, "not the verification";
  - **Restore from backup (server recovery)**, labelled Draft: states the rule being added to architecture 5.3. Restored shared history is released **only if a demonstrably current control log that survived outside the restored backup set proves it is complete**. User PCs' copies are never used to release it. Otherwise the server **fails closed**: restored shared history is discarded at once, every device is revoked and must re-pair, and every category is off. The reviewer strip simulates both outcomes. The fail-closed result shows the devices as "Revoked: must re-pair", and the Sharing tab shows a banner saying so;
  - updates (channel, current release, previous release kept);
  - audit log;
  - the operating-system access disclosure.

## 4. Pet interaction states

Every pet supports the same states. Each has an icon badge and a text tag, so no state depends on colour. With reduced motion, animation is replaced by a still pose.

| State | Entered when | Badge and tag | Animation (off with reduced motion) | Click does |
| --- | --- | --- | --- | --- |
| Idle | Default | none | Slow 3 px bob, 4 s | Opens B1 |
| Nudge (cue) | Something is due or offered (review, check-in) | "!" | Small wiggle, 3 times, then still | Opens the offer |
| Reminder card | B2 is open | Bell | Wiggle | Closes or reopens the card |
| Exercise | Movement intro or steps | Figure, "Moving" | Gentle stretch loop | Shows the current step |
| Waiting | 10 min with no response | "1", "1 reminder waiting" | None | Reopens the same reminder |
| Celebration | Completion or goal reached | Star | Two small hops, then Idle | n/a |
| Snoozed | Snooze 10 min | "zz", "Snoozed 10 min" | Eyes closed | Reopens the reminder early |
| Focus | Focus is on | Target, "Focus 60 min" | Eyes closed, no motion | Time left, End focus |
| Pomodoro | Timer running | Clock, "Work 24:59" or "Break 04:10" | Eyes closed | Pause, Resume, End |
| Quiet hours | Outside work hours or on a non-workday | Moon, "Off hours" | Dimmed, eyes closed | B1 (logging still allowed) |
| Meeting or suppressed | Manual meeting toggle | Mute, "Meeting - paused" | Dimmed | Meeting over |
| Presentation or full screen | Detected | Pet and every bubble hidden; tray text shows "hidden: presentation" (**Proposed default, owner to confirm**) | n/a | Tray "Show pet"; nothing is replayed |
| Paused | User paused coaching | Pause, "Paused 1 h" | Dimmed, eyes closed | Resume coaching |
| Disconnected | Server unreachable (paired users only) | Plug, "Offline - still works" | Desaturated | B10, then B1 |
| Camera on (V2) | Posture check active | "CAM" plus dot (top left), "Camera on" | Idle | B1 (with "Camera off") |
| Camera: no reading (V2) | Away, dark, occluded, low confidence or suspect | "CAM", "Cam: no reading" or "Cam: cues paused" | Idle | Recalibrate request if suspect |

**Snooze and dismiss:** a snooze covers the whole combined reminder and returns it once. A dismissal is logged as dismissed, never as done. After three dismissals the app offers Light intensity.

**Precedence** (architecture 4.3):

1. pause, lock, sleep or off-hours;
2. focus, presentation or meeting;
3. an active exercise, break or Pomodoro;
4. one combined reminder.

## 5. Key flows

Each flow can be started from the prototype's flow index. Step references use the screen IDs above.

1. **First-run setup and optional pairing:**
   1. Welcome.
   2. Connect: keep "Use on this PC only", **or** choose Pair. Paste the slip and Check slip. The app checks the full certificate fingerprint (authoritative) before sending the secret; the 6 words are shown as an auxiliary aid only. Choose Pair this PC. The result is "Paired".
      - **Branches:** fingerprint mismatch (nothing sent), used or expired slip, unreachable server (retry or pair later), malformed slip.
   3. Choose a pet, name, size and colour.
   4. Workweek.
   5. Water goal and servings.
   6. Reminder intensity and categories.
   7. Sharing (if paired: all off; Preview shows M1).
   8. Try a reminder, then Finish. The pet appears at the right edge and celebrates once.
2. **Hydration log:** click the pet, then +250 ml. The confirmation shows the total and Undo. Water... opens the panel for a custom amount, Edit (correction) or Remove. At the goal: "Your logging goal is reached; hydration reminders are paused", and manual logging stays available.
3. **Movement session (six-day programme):**
   1. Pet menu, then Today's movement (or Start 2-min movement from B2).
   2. The intro shows the day's region and the seating check.
   3. Choose Seated or Standing.
   4. Each step: Done, Skip step or Stop.
   5. Result: complete (self-reported; celebration) or partial (counted separately, no catch-up). Skip today records a skip. Other days can be chosen from the intro, and Sunday is optional.
4. **Recovery-day plan:**
   1. A long day triggers a check-in, or the user taps "Deadline day".
   2. Yes leads to the invitation. Choosing Tomorrow or a date opens W15.
   3. Adjust the items, then Save. The pet confirms and offers Reschedule or Cancel plan.
   4. On the recovery day, the morning bubble lists the plan.
   5. At the end of the day, one reflection, which stays local.
   - **Decline path:** No or "Don't ask today" means nothing changes.
5. **User-started Pomodoro:**
   1. Pet menu, then Pomodoro..., then 25/5. If focus is on, confirm the replacement. If coaching is paused, confirm resuming for this session.
   2. The pet's tag counts down. A click gives Pause, Resume or End.
   3. At the break, the combined break card appears: Start break or Skip (declined).
   4. "Break over: start next round?" Auto-start stays off by default.
   5. A lock pauses the timer until Resume.
6. **Weekly and monthly review:**
   1. At Sat 17:00 the pet offers the review: Open, Later or Skip.
   2. W13 shows successes, misses, coverage and up to 3 suggestions (Apply or Not for me). Phone labels can be corrected there.
   3. Monthly progress (W14): metric chips, chart, baseline toggle, and Show as table.
   - **Empty variant:** "first chart after two weeks".
7. **Sharing preview, stop and delete** **Draft pending contract**:
   1. Settings, then Sharing and server. Switch Hydration on: M1 appears first, then Start sharing. The status shows "1 waiting", then Sync now.
   2. Switch it off: M2 "Sharing stopped" (unsent summaries discarded), then Delete what I shared, choose all dates or a range (From and To), review the confirmation, and Yes, delete. A receipt that matches the chosen dates follows, with backup expiry within 30 days.
   - **Offline:** deletion is queued with a retrying status.
   - **Second PC:** M3 asks before re-enabling, and sharing restarts from today.
8. **Local export and import** **Draft pending contract**:
   - Settings, then Backup and update, then Export... Set a password, then Save. The prototype shows a simulated file name.
   - Import... shows the "replaces, never merges" warning, then Replace my data. Outcomes: ok, wrong password, newer version, or damaged file (nothing changed).
   - Update from file: installer plus signature, then verified or refused.
   - Rollback: warning, with an Export first option.
9. **V2 camera:**
   1. Pet menu, then Posture check, which opens the consent screen.
   2. Turn on camera.
      - **Branches:** denied by Windows, camera busy, or no camera. Each gives an explicit state, and V1 continues.
   3. Calibration (simulated silhouette): usable gives "saved, numbers only"; not usable gives the reason and Try again.
   4. With the camera on, the pet shows the CAM badge and tag.
   5. A sustained change gives one cue (Thanks, Snooze 30 min, or Off). A 30-min cooldown follows.
   6. No reading: tagged "Cam: no reading" and no score.
   7. Camera moved: cues pause at once; after 2 min, one recalibrate request. Recalibrate returns to step 3.
   8. Another app takes the camera: posture check pauses.
   9. Off (from the pet menu, a cue or settings): "Camera off. Capture stopped." and the badge is removed at once.
10. **V2 meditation pause:**
    1. Settings, Meditation: set an invitation time (off until set).
    2. At that time the pet invites: 2, 5 or 10 min, or Not today (skipped, no backlog).
    3. Text session: Next, Pause or Resume, Stop, and an eyes-open, non-breath alternative.
    4. "Done. Back to work whenever you are ready." Logged locally only.

Further flows in the prototype:

- **Pet on the desktop:** menu, drag and snap, edits.
- **Combined reminder:** every action, no-response collapse, snooze return and the third-dismissal offer.
- **Focus, pause, meeting, presentation and quiet hours.**
- **Phone interruption** self-report and correction.
- **Server unreachable:** B10, waiting summaries, "back online, sent n".
- **Keyboard path.**
- **Admin status page.**
- **Pet state gallery.**

## 6. Accessibility

- **Keyboard:**
  - The pet window never takes focus, so typing in Excel, Tally or a portal is never interrupted.
  - A configurable hotkey (default Ctrl+Alt+H) and the tray menu open the **pet panel**, which takes focus on purpose.
  - Tab and Shift+Tab move between actions; Enter or Space chooses.
  - **Esc** closes the bubble or panel and returns focus to the previous app (PET-04). In the prototype the pet panel's Esc moves focus to the simulated work surface (the spreadsheet), not to nowhere. Real Windows focus return is a build acceptance test (architecture A8) and stays labelled so.
  - Windows have a logical tab order, visible focus rings (3 px) and Esc to close.
  - **Modal dialogs trap focus.** Focus moves into the dialog on open. Tab and Shift+Tab stay inside it and wrap. Esc or Cancel closes it and focus returns to the control that opened it (found again if the screen was redrawn).
- **Scaling:**
  - Bubble and screen text scales to 100, 125 or 150% in settings, on top of Windows display scaling.
  - Acceptance tests run at 100, 125, 150 and 200% (PRD PET-04; architecture section 11).
  - **Rule for build:** on a 1366x768 display at 150%, the logical work area is about 910x480. Windows therefore open at no more than 90% of the work area, with a minimum of 640x440 logical px. Content scrolls inside the window, and the settings navigation collapses to a list above the content below 760 px. Bubbles are at most 380 logical px wide and at most 60% of the work-area height, with internal scrolling. The prototype uses fixed 100% sizes for review only.
- **Colour-blind-safe status:**
  - The Okabe-Ito palette (blue #0072B2, orange #E69F00, vermilion #D55E00, bluish green #009E73).
  - Every status pairs colour with an icon or a word.
  - Charts: hatched means no data, an outline means measured zero, dashed means partial or in progress, a dashed line marks the baseline. The day strip has a symbol on every cell.
  - Red and green are never the only distinction.
  - A higher-contrast option is available.
- **Reduced motion:** follows the Windows "Show animations" setting (`prefers-reduced-motion`) plus an in-app switch and the pet's Motion switch. Each pet has a still pose per state. Progress bars have no animated stripes. The movement guide uses still illustrations with text.
- **Screen readers:**
  - Bubbles announce through a polite live region without moving focus.
  - The pet's accessible name includes its state and tag, for example "Shelly the pet, Focus mode, Focus 60 min".
  - Illustrations have equivalent text, and charts have a table view.
- **Language:** plain English and short labels; strings are keyed for later translation.

## 7. PRD section 10 coverage

| PRD flow | Covered by | Notes |
| --- | --- | --- |
| 1. Installation and pairing: LAN, hosted, server unavailable, invalid pairing | W1 step 2, W2 | Hosted is greyed (V3 deferred). The installer itself is not simulated |
| 2. Initial setup: water, workweek, reminders, ten-pet selector | W1, W3 | 4 drawn pets plus 6 "art pending" placeholders. Visual-family approval is a separate decision from the ten finished pets |
| 3. Desktop context: pet over spreadsheet, compact and expanded, position and size | P0, B1, W3 | Fake spreadsheet window behind |
| 4. Combined reminder: completion, water, snooze, dismiss, focus | B2 | Includes the no-response and third-dismissal states |
| 5. Six-day exercise: instructions, adaptation, partial, skip | B4, W6 | Content is provisional, pending content review |
| 6. Settings and sharing: toggles, preview, pending sync, revoke, delete | W8, M1 to M3 | Draft pending contract |
| 7. Weekly reflection: strong, mixed, missing data, first week | W13 | Four variants |
| 8. Monthly progress: coverage, baseline, empty, accessible alternative | W14 | |
| 9. V2 calibration and check-in, camera conflict, low confidence, immediate off | W16, B11 | Simulated silhouette only |
| 10. V3 device paths | Not prototyped | V3 deferred by the owner (20 Oct); greyed placeholder only |
| 11. Phone self-report and correction; experimental states visibly separate | B8, W13 | No automatic phone detection is shown (DIS-02 stays an experiment) |
| 12. Administrator status, pairing and update view without health analytics | A1 | Draft pending contract |
| Addendum: recovery-day invitation and planning, deadline-day decline | B7, W15 | |
| Addendum: Pomodoro controls and combined break | B6, W7 | |
| Addendum: simple V2 meditation timer | B12, W11 | |
| Addendum: avatar sync, groups, rankings, scores | Not prototyped | V3, deferred |

## 8. Open design questions

**Proposed defaults.** Items 1, 3, 7, 9 and 10 below come with a proposed default from the review. Each is labelled **Proposed default, owner to confirm** in the prototype. **They are not owner answers.**

1. **Pet visual family:** do the flat two-colour SVG placeholders point in the right direction? This is the *visual-family approval*, and it is separate from the *ten finished pets* (4 drawn, 6 art pending). Approving the family does not approve any pet as finished. The remaining six need art once the direction is approved.
2. **Hotkey:** Ctrl+Alt+H can clash with AltGr keyboard layouts and with other office tools. Confirm the default, or ship with no default and set it in setup.
3. **Presentations:** should the pet hide completely (as prototyped) or shrink to a tray badge? *Proposed default, owner to confirm:* the pet and all bubbles are hidden completely, the tray icon stays, and nothing is replayed afterwards.
4. **No-response collapse:** after 10 minutes, is a "1" badge on the pet enough, or should the tray icon also show it?
5. **Pairing entry:** pasting an 80-character slip versus typing it in groups from print (architecture A14). The final format follows the pairing contract.
6. **Deletion scope:** "all dates" plus a date range, or per-month choices only? This follows the sharing contract.
7. **Recovery check-in:** confirm the 9-hour active threshold and the check-in wording. *Proposed default, owner to confirm:* the check-in is optional and configurable (switch plus 8, 9 or 10 hours), and PC activity time is labelled an estimate, not evidence of stress.
8. **Weekly review:** hide the phone section entirely for users who never log phone interruptions? The proposal is to hide it.
9. **Movement illustrations:** a neutral human figure (as prototyped) or the pet demonstrating? The proposal is the human figure, for clarity. All movement content is **provisional, pending the content review**.
10. **Posture cue wording:** name the direction ("leaning further forward") or stay neutral? *Proposed default, owner to confirm:* neutral, "Your position has changed from your calibration. Would you like a short movement break?" Directional detail only after the detector passes a quality gate in the desk test.
11. **Water wording at the goal:** "Your logging goal is reached; hydration reminders are paused", with manual logging still available. Confirm the wording.
12. **Pace guide:** the prototype spreads the user's own goal evenly over the user's own work hours and calls a user "behind" only when the logged amount is at least 100 ml under the amount expected by now. Confirm this sample rule, or choose another.
13. **Server recovery wording (Draft):** the recovery rule (release restored history only on proof from a surviving control log outside the restored backup set; otherwise fail closed) is being added to architecture 5.3. The admin and sharing wording follows it and will be reconciled with the final text.

## 9. Prototype notes and verification

- **Simulated:** all data, the clock, the server, pairing results, sharing, deletion receipts, files, updates and the camera. The camera view is a drawn silhouette and `getUserMedia` is never called. Pomodoro and calibration times are shortened, and some steps are triggered from "Reviewer" strips or caption buttons. A browser page cannot reproduce a non-activating window: the prototype prevents focus on pet clicks and returns focus on Esc, but real focus behaviour is a build acceptance test (architecture A8).
- **Structure:** one HTML file of about 1,300 lines with inline CSS and JS:
  - a state object `S` (all settings, the recovery plan and its draft, applied suggestions, deletion tombstones and restore outcome live here, in memory only);
  - `petSVG` for the four placeholder pets;
  - `PET_STATES`, the single table behind both the pet visuals and the gallery;
  - `say` for bubbles, `openWin` for windows, `modal` for dialogs (focus trap, Esc and Cancel restore focus to the invoker);
  - flows `F.*` and actions `A.*`, wired through `data-a` attributes.
  - No external assets, fonts or network calls.
- **Verification** (21 Oct 2026, headless Chromium through Playwright): `verify_prototype.cjs` covers the following.
  - **Flows:** opens the file from `file://` and starts each of the 19 enabled flows from the index.
  - **Clicks:** clicks every visible action on each flow's first screen, restarting the flow before each one. It also clicks every action that appears one level deeper and then walks the primary buttons. That was 412 action clicks in total.
  - **Keyboard and drag:** checks that Ctrl+Alt+H moves focus into the pet panel and that Esc closes it, and drags the pet to the left edge.
  - **Targeted state assertions (87):** the script reads the state object `S` or DOM values. Groups: persistence and undo (16), range deletion and receipts (19), focus (14), contract wording (14), water copy and pace (12), proposed defaults and labels (12). Examples: recovery switches, times and repeat are in `S` after Save and shown again on reopen; Undo restores the prior value; the range receipt shows the chosen dates and only an all-dates deletion says "all dates"; deleting today blocks today's re-upload; Tab and Shift+Tab stay inside a modal and Esc and Cancel restore focus to the invoker; the pet panel's Esc focuses the work surface; no timing-bitmap or slot text exists; the new water copy is present; a daily total below the goal is not called "behind pace".
  - **Result:** **0 console errors or warnings, 0 page errors, 0 network requests and 87 of 87 assertions passed.**
  - **Screenshots:** saved in `prototype/screens/`. Revision 2 refreshed 01, 02, 03 and 05, and added 07 and 08:
    - `01-combined-reminder.png`
    - `02-setup-pairing.png`
    - `03-weekly-review-mixed.png`
    - `04-monthly-progress.png` (unchanged)
    - `05-sharing-preview.png`
    - `06-camera-calibration.png` (unchanged)
    - `07-delete-range-receipt.png` (range deletion receipt)
    - `08-admin-restore-fail-closed.png` (server recovery, fail closed)
