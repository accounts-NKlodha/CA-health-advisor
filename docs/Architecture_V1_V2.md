# HealthierYou architecture proposal: V1 and V2

Status: **proposal for review**, revision 4 (21 Oct 2026). It addresses Codex's three reviews of PR #3 and the owner's answers relayed on the PR. Revision 4 changes only the server recovery rule (5.3, with matching tests) and records the pairing implementation gates (5.2). Codex has given conditional design acceptance. That is not approval to build or merge, and the pairing security gate (5.2) is not yet signed off.

**Baseline:** the owner-approved PRD 0.3, published as [`docs/PRD_0.3.md`](PRD_0.3.md) (approved 18 Oct 2026; owner's name removed at the owner's instruction; source package SHA-256 `13224412c460fc0f260ebf8c84ef7e6e0715e263d231217ebad6c4d2a9fd962e`). It includes the recovery, focus and community addendum. This repository's `docs/PRD.md` (0.1) is superseded.

**Execution entry point:** current work is governed by the owner decisions recorded in `docs/Decisions.md` and by this architecture. Wherever they differ from `docs/PRD_0.3.md`, they take precedence. The status, approval, budget, calendar and deadline sections inside `docs/PRD_0.3.md` (for example the 5 Nov all-versions target, the 4 GB minimum PC and the gate states as of 18 Oct) are a historical snapshot, not current instructions. The PRD's requirements and acceptance criteria stay in force unless a recorded decision changes them.

**Other inputs:** delivery and feasibility packet revision 2; feasibility batch 1 (final build 6ef6eb3); the camera-vision note (20 Oct); and the `spike/batch1` code.

**Not approved:** the prototype and product development. This document contains no product code. The gates are unchanged: architecture review, then wireframes and a clickable prototype, then the owner's prototype approval, then code.

**Scope (owner, 20 Oct):** V1 and V2 first. V3 (hosted offering, wearables, rankings, one group per user) is deferred, not removed. Section 13 gives interfaces only; there is no wearable research.

Labels used in this document:

| Label | Meaning |
| --- | --- |
| **B1** | Proven by batch 1 on one 8 GB i5 Windows 11 laptop, offline, as a standard user |
| **Doc** | Vendor documentation or source read; not yet run by us |
| **A** | Assumption (checks are in section 14) |
| **P** | Specified by the PRD |
| **D** | The PRD is silent; this is a default chosen here, open to review |

## 1. Summary

- **Desktop companion.** Tauri 2 (a Rust core plus WebView2 pages), a bundled fixed WebView2 runtime and a per-user NSIS installer (all B1). One Rust coaching engine makes every timer and reminder decision. Data is local-first in SQLite. Every V1 and V2 feature works without a server, including update, rollback and full personal backup through signed files (section 8).
- **V2 posture.** MediaPipe pose lite runs on the CPU at 2 to 3 fps, in a camera page that exists only while the camera is on. Resource use is B1; accuracy is unproven. The output is drift from a quality-checked per-user calibration. Cues are suspended whenever the baseline is in doubt. No frames or landmarks leave memory.
- **LAN server.** One Rust Windows service with SQLite. The owner reports it will run on a Windows 11 PC with a 12th-gen i7 and 16 GB RAM.
  - **Pairing:** pinned TLS 1.3 plus an opaque single-use secret, both carried on a pairing slip. The full certificate fingerprint on the slip is authoritative.
  - **What it stores:** per-device daily summaries for the categories each user has opted into. Deleted date ranges can never be uploaded again. Every control change (policy, deletion, pairing, revocation) is committed in the same transaction as its effect and mirrored off the database volume. After a backup restore, no shared history is served until the mirror, a signed receipt from every paired device and the administrator together show the control log is current. Otherwise the server fails closed.
  - **Administrator:** sees no habit charts. Anyone with operating-system access to the server PC could technically read the unencrypted summaries, and the app says so.
- **Forecast, not a deadline.** If the prototype is approved on 30 Oct, the V1 pilot would start about 19 Nov (range 15 to 23 Nov) and the V2 pilot about 27 Nov (range 22 Nov to 1 Dec). These are forecasts pending the owner's agreement. They do not replace the recorded 5 Nov target, which this plan does not meet. Any change to scope or dates is the owner's decision (section 12).

## 2. Goals and non-goals

| Goals (V1, V2) | Non-goals |
| --- | --- |
| Pet-led reminders that never take keyboard focus or block input (P) | Clinical measurement, diagnosis, productivity or performance scoring (P) |
| Combined reminders; focus, Pomodoro, meeting and presentation suppression; no prompt bursts (P) | Forced breaks, input blocking, streak punishment, default sound (P) |
| Water, the six-day movement programme, recovery day, phone self-report, weekly and monthly review (P) | Manager dashboards or rankings; administrator views of personal charts (P) |
| Fully usable without a server; sharing optional per user and per category (P) | Cloud AI or any internet use at run time (P for V1; D for V2) |
| V2: calibrated drift cues with confidence, a visible camera state and an instant off switch (P) | Angle accuracy, emotion, drinking or exercise verification, phone-use detection (P) |
| V2: a simple text-guided meditation pause (P) | V4 sound, headphone and colour sessions (P) |
| On an 8 GB PC: idle at most 150 MB and 2% CPU; camera at most 350 MB and 10% CPU (P) | 4 GB PCs (owner, 20 Oct: the minimum is now 8 GB) |
| Install without administrator rights or prerequisites (P, B1) | Code signing, stores, hosting (no purchase approved) |

## 3. Context

```
                         Office LAN (one subnet, private network profile)
 +------------------------------------------+         +-----------------------------------+
 | Employee PC (Windows 10/11, 8 GB+)       |  HTTPS  | Server PC (Win 11, i7, 16 GB)     |
 |  HealthierYou companion (per user)       |  TLS1.3 |  HealthierYou server (service)    |
 |  +------------+   +-------------------+  |  pinned |  +-----------------------------+  |
 |  | Rust core  |<->| pet page (always) |  |<------->|  | API: pair, policy, sync,    |  |
 |  | engine,    |   | screens (on open) |  |  mDNS + |  |   delete, restore, updates  |  |
 |  | store, sync|   | camera page (V2,  |  |  slip   |  | SQLite + control log mirror|  |
 |  +-----+------+   |  only while on)   |  |  host   |  | Admin page: localhost only  |  |
 |  SQLite, signed-file update and backup   |         |  | Backups while running       |  |
 |  Windows signals: input, lock, sleep,    |         |  +-----------------------------+  |
 |  presentation mode           webcam (V2) |         +-----------------------------------+
 +------------------------------------------+
 No internet use at run time. No images, landmarks or keystroke contents leave the PC.
 V3 (deferred): a hosted service speaking the same /v1 protocol (section 13).
```

## 4. Desktop companion

### 4.1 Process and window model

| Part | Lifetime | Notes |
| --- | --- | --- |
| Rust host | While the user is signed in | Engine, store, Windows signals, sync, updater, tray. Starts from the per-user Run key (D); single instance |
| WebView2 browser and GPU processes | Always | One environment, using the fixed runtime 155 in the install folder (B1) |
| Pet page | Always | Transparent, always on top, no taskbar entry, `WS_EX_NOACTIVATE \| WS_EX_TOOLWINDOW` (B1: P3 10/10). The window resizes for the compact or expanded bubble |
| Screens page | Created on open, destroyed on close | Setup, settings, goals, pet library, sharing, reviews, backup |
| Pet panel (keyboard path) | On demand | Opened from the tray or a hotkey (default Ctrl+Alt+H, configurable; D). Takes focus on purpose; Escape hands focus back (PET-04; A8) |
| Camera page (V2) | Only while the camera is on | Its own webview, so its memory is returned when it closes. Camera permission is granted only to this page |

Idle memory with the pet and a control window was 95 MB (B1). Real art and the UI framework still need to be measured (A7).

### 4.2 V1 components

| Component | Responsibility | Requirements |
| --- | --- | --- |
| Pet shell | Edge placement, drag and snap, scale 75 to 150% (D), per-monitor DPI, repositioning after a monitor is removed, hidden during presentations and full-screen apps (D), eight states, reduced motion, tray menu | PET-02 to PET-04 |
| Pet library | Ten data-only packs plus per-user edits and reset (4.4) | PET-01, PET-02 |
| Activity tracker | Polls input idle every 5 s; lock, unlock, sleep and resume events; session states (4.3) | RHY-01 |
| Coaching engine | A single deterministic state machine covering movement, eye rest, hydration, focus, Pomodoro, recovery day, meditation and posture cues (V2), snooze and cooldown. The only component that shows bubbles | RHY-02, RHY-03, FOC-01, REC-01, CAL-01, CAM-02 |
| Suppression | Pauses prompts for: the user's pause, lock, sleep, off-hours, focus, Pomodoro work, presentation or full-screen state (Windows notification-state API; Doc), and a manual "in a meeting" toggle. An experimental "camera or microphone in use" hint is off by default (A11) | RHY-04 |
| Hydration | Goal, serving sizes, quick and custom logging, undo and correction, pacing; no encouragement past the goal | WAT-01, WAT-02 |
| Movement programme | Bundled, versioned Monday to Saturday content with seated and standing variants, skip and stop, and partial versus complete labels; Sunday optional | MOV-01, MOV-02 |
| Recovery day | Marking a deadline day; a neutral check-in after a long active day (over 9 h, D) that the user confirms; one pending plan; a gentler day profile; one reflection | REC-01 |
| Pomodoro | Off until Start; 25/5, 50/10 or custom; precedence as set in the addendum | FOC-01 |
| Phone self-report | One action; work, personal or unsure label; editable | DIS-01 |
| Review engine | Weekly and monthly figures with coverage; at most three rule-based suggestions; text alternative for charts | REV-01, REV-02 |
| Screens, store, sync, backup, diagnostics | Sections 5, 6, 8 and 10 | Flows 1 to 8 and 11, SHR, LAN |

### 4.3 Session and timer semantics

The tracker never treats a lack of input as a confirmed break, because the PRD says input idle is not proof of movement (P). It uses these states (all D unless marked P):

| State | Entered when | Movement timer (45 min, P) | Reported as |
| --- | --- | --- | --- |
| Active | Input within the last 60 s | Accumulates | Work time |
| Quiet (unknown) | No input for 1 to 15 minutes while unlocked and awake. Could be reading, a call, or away from the desk | **Accumulates**: reading counts toward the 45 minutes | Work time, flagged as estimated |
| Long quiet | No input for more than 15 minutes while unlocked | **Pauses**: neither accumulates nor resets | Unknown; excluded from the longest work block |
| Confirmed break | Lock or sleep lasting 2 minutes or more; the user marks a break or movement done; V2 camera reports absence for 2 minutes or more (when on) | **Resets** | Break (never recorded as movement) |
| Short lock | Lock or sleep under 2 minutes | Pauses | Unknown |
| Off-hours or paused | Outside work hours, or the user paused coaching | Pauses, no prompts | Not counted |

**Reminder behaviour:**

- A cue that falls due during Quiet is still shown, so a reader sees it without touching the keyboard. It is shown once.
- After 10 minutes with no response, the cue collapses to a small badge and is logged as "offered, no response", not as a dismissal.
- After unlock or wake there is a 2-minute grace period, then at most one combined reminder (P: no burst).
- Pomodoro stays paused after a lock or sleep until the user resumes it (P). Focus runs on wall-clock time and ends silently if it expires while the PC is locked.
- Sessions that cross midnight are split by the user's time zone (P, default Asia/Kolkata).

**Default timings (P unless marked D):**

| Prompt | Default |
| --- | --- |
| Movement | After 45 minutes accumulated (as above) |
| Eye rest | 20 minutes, optional, folded into a nearby movement break |
| Hydration | Joins an eligible reminder when the user is behind pace. It appears alone only when 2 servings behind with no reminder for 30 minutes (D) |
| Spacing | 5-minute cooldown. A 10-minute snooze covers the whole combined reminder |
| Focus | 30, 60 or 90 minutes; no replay afterwards |
| Posture (V2) | 30-minute cooldown |
| Repeated dismissals | 3 dismissals in a day (D) trigger an offer to reduce intensity |

**Precedence (highest first):**

1. Pause, lock, sleep or off-hours (P)
2. Focus, presentation or meeting (P)
3. An active exercise, break or Pomodoro (P)
4. One combined reminder (P)

The engine runs on an injected clock, so it can be tested by replaying traces. Test cases T-R1 to T-R6 are in section 11.

### 4.4 Pet library and asset format

The ten pets share one behaviour set (P). Each pet is a **data-only pack** with no executable plugins (P):

| Part | Format |
| --- | --- |
| `pet.json` | id, version, name, licence and provenance, tone presets, palette slots, hitbox, and a state-to-animation map for the eight states. JSON-schema validated at build and load time |
| Art | SVG parts with CSS keyframes, so they stay sharp from 100 to 200% scaling. Palette variables give colour variants. A static pose per state gives reduced motion. Sprite sheets are the fallback if CPU use is too high (A) |
| Wording | A message catalogue keyed by tone and string id; English first, ready for translation (P) |
| User edits | Name, scale, colour, placement, motion and tone per user; "Reset to defaults" (P) |
| Animation budget | 12 fps while animating (B1: 1.1% idle CPU); paused when idle, in focus or hidden (D) |

### 4.5 V2 webcam posture

| Aspect | Design (thresholds are D: tuned in the desk test and frozen before scoring, as the PRD requires) |
| --- | --- |
| Opt-in and capture | Off by default. Uses `getUserMedia` at 640x480 on Media Foundation (B1; DirectShow failed C3 and C4 in batch 1) |
| Inference | MediaPipe tasks-vision 1.1.1 pose lite, CPU delegate, 2 to 3 fps (B1: 37 ms per frame, 4.9% CPU, 301 MB peak). Runs in a Web Worker (A9; the spike ran it on the main thread). Drops to 1 fps if over budget |
| Valid frame | Nose, both eyes or ears, and both shoulders have visibility of at least 0.5, and both shoulders are inside the frame |
| Features | Ear-to-shoulder gap divided by shoulder width (slouch); shoulder width against the baseline (lean-in); shoulder tilt; head roll. Hips and legs are ignored |
| Calibration quality (all required) | At least 40 valid frames out of at most 30 s, with at least 60% of frames valid; shoulder width between 15% and 70% of frame width; mean visibility at least 0.7; each feature's MAD at most 3 times its floor (the user held still). If any check fails, the app shows "Calibration not usable" with the reason, and no cues are given |
| Scale floor | z = (x − median) / max(1.4826 × MAD, floor). Floors: gap ratio 0.02, width ratio 0.02, tilt 0.015, head roll 2°. A perfectly still calibration can therefore never produce huge z-scores |
| Episode and cue | z is smoothed over 10 s. An episode starts after 60 s of drift with at least 70% valid frames, and the cue follows within 30 s (P gate) if the engine allows it. One episode produces one event |
| Baseline-suspect (viewpoint check) | Cues are suspended at once on any of: (a) a camera device, resolution or stream restart, or a resume from sleep; (b) a step change, completed within 2 s, of at least 15% of frame in shoulder midpoint position, or at least 20% in shoulder-width scale, held for 20 s, in landmark geometry that ignores the face; (c) the ratio of inter-ear to shoulder width moving outside the calibration band while posture features stay steady. Face or head motion alone never triggers it |
| Baseline-invalid | If the suspect state does not revert within 2 minutes, the app asks once to recalibrate. Nothing is scored against the old baseline until recalibration. When unsure it suspends cues rather than coach |
| No reading | Absent, occluded, low confidence, camera busy, denied or unplugged, or worker stalled. Shown explicitly; no score is inferred; V1 continues (P) |
| Worker watchdog | No result for 5 s while capturing gives "no reading (stalled)" and one worker restart. A second stall within 10 minutes stops the camera and tells the user |
| Restart | The stored calibration (numbers only) is reloaded as suspect. It is accepted if the first 20 s match within tolerance; otherwise it is invalid |
| Pausing and stop | Paused during lock, sleep, off-hours, presentation and meetings. During focus, cues are dropped rather than queued. Stop ends the track and destroys the page (B1: 0.35 s); taking longer than 2 s blocks release (P) |
| Stored | Derived `posture.observation` events and daily coverage only. Landmarks are kept in memory for 90 s or less. Frames are never written or sent |
| Quality report | For each setup and session it separates: planned check intervals; **no-reading** intervals by reason; **baseline-suspect or invalid** intervals; usable coverage %; **detector episodes** (matched to labels: true, false, missed); **cues delivered** versus suppressed (cooldown, focus, meeting, suspect); and threshold-to-cue latency. Thresholds per the PRD V2 gate |
| Harness | An operator marks staged drift, absence, occlusion and camera relocation with keys while the detector logs. No video is stored |

Test cases T-C1 to T-C6 are in section 11. The long soak and the second-webcam checks (V2-5) must finish **before V2 UAT**.

### 4.6 V2 meditation pause

One optional daily invitation at a time the user chooses. It stays off until a time is set (D). The session is text-guided, with eyes-open and non-breath alternatives, and lasts 2, 5 (default) or 10 minutes, with pause and stop. The invitation respects focus, meetings and off-hours. A missed invitation is skipped for that day, with no backlog (P, CAL-01). It never auto-starts or plays audio, and sessions are logged locally only.

## 5. LAN server

### 5.1 Service

| Function | Design |
| --- | --- |
| Host | The owner reports a Windows 11 PC with a 12th-gen i7 and 16 GB RAM. Still unconfirmed, and checked on install day (A1): administrator install access, same-LAN reach from every pilot PC, and power or sleep behaviour |
| Packaging | One Rust binary running as a Windows service, installed once by an administrator. Data lives in `%ProgramData%\HealthierYou\Server`, readable by SYSTEM and Administrators only. Firewall rule for TCP 8443, private and domain profiles only (D) |
| Users | Created by the administrator with a display alias only: no email, phone or employee number (D) |
| Authorisation | Each device has a token. The user id is taken from the token, never from the request body, and no endpoint returns another user's data (LAN-01) |
| Admin page | Served on localhost only, behind an Argon2id-hashed passphrase. It shows health, database size, last backup, devices, pairing slips, update channel and an audit log. It never shows habit data (P) |
| Backup | Runs while the service is up: daily at 13:00 IST, plus a catch-up at service start when the last backup is more than 24 h old. Overnight uptime is never assumed (D). Keeps 14 daily and 2 weekly copies, 30 days at most. Each backup records its control high-water sequence (5.3) |
| Load | 3 real clients plus 10 simulated clients (P), driven by a simulator built on the shared core crate |

### 5.2 Pairing: standard TLS with out-of-band fingerprint verification

Chosen route: pinned TLS 1.3 plus an opaque single-use secret, with no additional application handshake. A PAKE was rejected because the balanced-PAKE crates available for Rust state they are unaudited.

| Element | Contract |
| --- | --- |
| Server identity | At install, the server generates an ECDSA P-256 key and a self-signed certificate. The private key is protected with machine-scope DPAPI. The full SHA-256 fingerprint of the certificate is the server's identity |
| Pairing slip | The admin page creates one slip per user: `hy1-<host>-<port>-<fingerprint, base32, 52 chars>-<secret, 80 random bits, 16 base32 chars>`. It can be copied (for internal chat) or printed. Valid 24 h, single use |
| Initial TLS trust | The companion connects with TLS 1.3 only, with 0-RTT disabled. It accepts the connection only if the presented certificate's full SHA-256 fingerprint equals the one in the slip; there is no TOFU and no truncation. If they do not match, it aborts **before** sending the secret. The full fingerprint check is the only authoritative verification. As an auxiliary display, both sides also show six words derived from the start of the fingerprint. They are an aid for humans, not an encoding of the whole fingerprint |
| Proofs | The server proves possession of its key in the TLS 1.3 handshake, which is checked against the pinned fingerprint. The client proves possession of the slip by sending the opaque secret once, inside that pinned channel (`POST /v1/pair` with {secret, device label, app version}). There is no further application-level proof |
| KDF and salt | None needed. The secret is 80 random bits checked online only, and the server stores SHA-256(secret). There is no user-chosen password to stretch |
| Replay and consumption | One SQL transaction: `UPDATE slips SET used_at=now WHERE hash=? AND used_at IS NULL AND expires_at>now`. It must affect exactly 1 row, and the device row and its pairing entry in the control log (5.3) are created in the same transaction; otherwise everything rolls back. Per-IP and global rate limits apply. Expired or used slips are kept 30 days for the audit trail, then purged |
| Device credential | A 256-bit random token is returned once over the pinned channel. The companion stores it with user-scope DPAPI; the server stores SHA-256(token). Every later connection checks both the certificate pin and the token |
| Address changes | The companion finds the host by mDNS or host name. The pin, not the address, establishes trust |
| Recovery | Lost token, reinstall or new PC: the administrator revokes the old device and issues a new slip. Lost server key (disk failure with no backup of the key): every device re-pairs with a new slip. Planned key rotation means re-pairing too (D, acceptable for 3 users) |
| Usability trade-off | A roughly 80-character slip is pasted, or typed in groups from print. That is clumsier than a 6-digit PAKE code, but it uses only standard, audited TLS (rustls), with no custom cryptography |
| **Design gate (not yet signed off)** | Codex's security sign-off of this contract is required **before V1-5 implementation starts** (forecast 5 Nov). It is not a check at the end of V1-5. Codex's review of 21 Oct accepts this simplified pairing as a direction. That is not runtime security certification and not authorisation to implement |
| **Implementation gates** | The V1-5 build must still show, and Codex must review: certificate-pin enforcement on every connection (abort before the secret is sent); protected handling of the secret, token and server key (DPAPI, never logged); atomic single use of each slip; and failure tests (T-P1 to T-P6). Passing these is evidence for review, not a security certification |

Test cases T-P1 to T-P6 are in section 11.

### 5.3 Consent, policy and deletion convergence

| Element | Contract |
| --- | --- |
| Control operations | These are policy changes {category, enabled, base_seq, op_id}, deletions {category, date range, request_id}, device pairings and device revocations. The server gives each one a per-user `seq` that only increases. Operations are idempotent by op_id or request_id |
| Ordering | A **disable** is always accepted. An **enable** is accepted only if its `base_seq` is at least the seq of the latest disable or deletion for that category. Otherwise it is rejected as `stale_enable`: the device reverts its toggle and shows "Switched off on another PC on <date>. Switch on again?", and a new user action sends a fresh enable |
| `share_from` | Each accepted enable sets `share_from` = the enable date for that category. Dates before `share_from` are never uploaded, so there is no backfill after a disable or a re-enable (D) |
| Tombstones | A deletion records {category, date range} and deletes the matching rows. **Tombstoned dates are permanently non-uploadable** for that user and category, whatever seq, revision or refreshed policy a later record carries. No historical re-share is proposed; one would need a separately authorised feature. A deletion range that includes today blocks uploads for today, and sharing resumes the next day (D). Tombstones hold no health payload |
| Device side | The device keeps a copy of its user's tombstones and `share_from` dates and never builds outbox records for blocked dates. Queued records are **never re-stamped**: after a policy refresh, any queued record whose category is off, whose date is before `share_from`, or whose date is tombstoned is discarded, not resent. A disable purges the outbox for that category at once, even offline. An offline enable stays pending until the server accepts it |
| Upload acceptance (server) | A record {category, origin device, local_date, revision} is accepted only if the category is enabled now, local_date ≥ `share_from`, local_date is outside every tombstone range, and the device is not revoked. Otherwise it is rejected and the client discards it |
| Atomic ordering | The control operation, its `control_log` row (seq, op, previous-entry hash) and its effects (policy row, tombstone, deleted rows, revocation) are written in **one SQLite transaction** (WAL, `synchronous=FULL`). After the commit, the same `control_log` entry is appended and fsynced to `control.mirror`. Only then is the client acknowledged, with {seq, chain hash, server signature}. Every sync response also carries the user's current signed head {seq, chain hash}, and the device keeps the highest one it has received as its **receipt**. At startup the server re-appends to the mirror any committed entries the mirror lacks. So every acknowledged operation is in both the database and the mirror, and a client that saw no acknowledgement retries idempotently |
| Mirror location | `control.mirror` is a hash-chained, append-only file kept off the database volume: a second disk, or a network folder if the office has one. Both are deployment checks (A1). If neither exists it sits in a separate folder, and a disk failure then leads to fail-closed recovery |
| Recovery hold | Any start from a restored snapshot (including a rollback restore, section 8) enters **recovery hold**. The server replays mirror entries newer than the snapshot, but it serves no restored history and accepts no uploads or enables. It accepts only device check-ins, disables and deletions, because those can only reduce sharing. A mirror chain that verifies, with a tail at or above the snapshot's high-water, proves the log is consistent. It does not prove the log is current: a stale but valid mirror can lack later acknowledged operations. That check is necessary but never enough to release data. Companions keep working locally and show "Server recovering: sharing paused" |
| Release evidence | Restored shared history is released only on a **demonstrably current control log from outside the restored backup set**. In this design that means all three of these, within 7 days of the restore (D): (1) **Mirror integrity:** the off-volume mirror is never part of a backup and was not restored from a copy; its chain verifies from the start, and its tail is at or above the snapshot's high-water. (2) **Device witnesses:** every device that the mirror lists as paired and not revoked checks in over the pinned channel and presents its receipt. Each receipt's seq is at or below the mirror's tail for that user, and its chain hash matches the mirror entry at that seq. (3) **Administrator witness:** on the admin page, the administrator confirms that the listed devices are every device that was paired, and that no device was paired or revoked after the time of the mirror's last entry. Receipts serve only as evidence that nothing is missing. They are never replayed and never used to rebuild the log |
| Fail closed | The server goes straight to the discard path, serving no restored data first, if any of these happens: the mirror is missing, corrupt or ends before the snapshot's high-water; any device, with a known or unknown token, presents a validly signed receipt beyond the mirror's tail or with a chain hash that does not match; the administrator cannot confirm the device list; the server key was lost; or a listed device has not checked in when the 7-day window ends. **Discard path:** the server discards all restored shared history, revokes every device so they must re-pair, and sets every category off. Users re-enable sharing from the restore date. Personal data on each PC is unaffected. A receipt beyond the tail that appears after release is treated as an incident: the discard path runs at once and the owner is told |

Test cases T-D1 to T-D10 and T-J1 to T-J6 are in section 11.

### 5.4 Sync, offline operation, server-side history and several devices

- **Offline first.** The companion never waits on the server. Retries back off from 30 s to 15 minutes with jitter. On reconnect it pulls the policy, tombstones and `share_from` dates first, discards anything blocked, then pushes. The pet shows a small disconnected badge, and settings explain the status (P).
- **Origin identity.** Every summary row is keyed by {user, **origin device**, category, local_date} and upserted only when its revision is higher. The origin device is the PC that recorded the events. It is kept unchanged through export, import and restore, and a PC only ever uploads rows whose origin is itself. History imported from a backup or restored from the server is therefore never re-uploaded as a new device's totals.
- **Server history is summary history only.** "Restore from server" returns opted-in daily summaries as read-only `imported_summary` rows, keyed by origin. It does **not** restore settings, goals, pet edits or detailed events; full personal backup is the local export (section 8). If an import and a server restore cover the same {origin, category, date}, one row is kept, with the local import taking precedence. The deletion receipt says backups expire within 30 days (P).
- **Several devices: per-device summaries, no timing bitmaps.** Reviews show each device's summary separately:
  - Combined figures are given only where adding up is valid: water logged, phone self-reports, and reminders offered (each PC really offered its own).
  - "Exercise done today" and "water goal met" count once per day across devices.
  - Durations such as active minutes and longest block are **never summed or merged across devices**. They are shown per device and labelled as estimates.
  - No within-day timing data is shared in any category.

## 6. Data model, placement and privacy

### 6.1 Local store (`%LOCALAPPDATA%\HealthierYou\data\hy.db`, per Windows user)

| Table | Contents | Retention |
| --- | --- | --- |
| `events` | Append-only: UUIDv7, schema version, type, UTC time, IANA zone, local date, source (manual, inferred or system; imported reserved for V3), measurement status, confidence, `supersedes` link, validated payload | 90 days (P) |
| `daily_summary`, `monthly_summary`, `imported_summary` | Per category and origin device, with coverage and a revision | Daily 90 days; monthly 24 months (P) |
| `goals_history` | Goal values with effective-from dates, so a week is judged against that week's goal (P) | 24 months |
| `settings`, `pet_profile` | Preferences and pet edits | Until changed |
| `sharing_policy`, `policy_ops`, `tombstones`, `consent_log` | Local copy of policy state, seq, `share_from` dates, tombstones, acknowledgements, the latest signed head receipt (5.3), and the notice version shown | 25 months (D) |
| `outbox`, `server_link` | Sync queue; certificate pin; DPAPI-protected token | Until delivered |

Corrections are new superseding events, so totals reconcile after edits, day changes and retries (WAT-01). A time-zone change never rewrites past local dates (P).

### 6.2 What stays on the PC and what can reach the server

| Data | On the PC | Can reach the server |
| --- | --- | --- |
| Camera frames, landmarks | Memory only | **Never** |
| Keystrokes, window titles, app names, screen contents | Never collected (P) | Never |
| Detailed events | 90 days | Never (D: summaries only, to minimise data) |
| Hydration summary (ml, goal, number of entries) | Yes | Only if switched on |
| Reminder and exercise outcomes (counts; routine done, partial or skipped) | Yes | Only if on |
| Work-session summary (active minutes, longest block estimate, focus and Pomodoro counts; no within-day timing) | Yes | Only if on |
| Phone self-reports (counts by label) | Yes | Only if on |
| V2 camera summary (camera-on minutes, coverage, episode counts) | Yes | Only if on |
| Meditation, recovery plans, reflections | Yes | Never in V1 or V2 (D) |
| Operational data (device id, alias, app version, Windows build, last seen, pairing state, policy seq) | Yes | Yes once paired; disclosed separately and not described as optional health sharing (P) |

All optional categories are off by default (P). Before a category is switched on, the preview shows the actual record and this audience text (SHR-01): "**Who can see this:** you, in HealthierYou. The app has no manager or administrator charts. Shared summaries are stored unencrypted on the office server PC, so a person with administrator access to that PC's operating system could technically open them." The same wording appears in the notice and in section 9.

### 6.3 Privacy design against the DPDP Act (legal applicability unverified)

This section states design intent. It is **not** a compliance claim or legal advice. Which law applies (DPDP Act 2023 and Rules 2025, commencement timing, and the IT Act's sensitive personal data rules in the meantime) is unverified, and the firm should confirm it (A13, Q3).

| Principle the design aims at | How |
| --- | --- |
| Notice in plain language | Shown at pairing and on each category: purpose (the employee's own summary history; no manager view, the default kept from Q2), data, audience text (6.2), withdrawal, and a contact the firm names (Q3) |
| Specific, affirmative, voluntary consent | Per-category opt-in. Pairing is optional, local use needs no server, and there is no manager view, to reduce pressure from the employer |
| Withdrawal as easy as consent | One toggle. Unsent data is purged at once. Deletion of shared data is a separate, tracked action, protected by tombstones and the mirrored control log (5.3) |
| Purpose and retention limits | No aggregate or secondary use. The server keeps data no longer than the device does (P), and backups for 30 days at most |
| Safeguards | Section 9. Logs carry no habit payloads (P) |
| Basis for habit data | Consent only; the employment legitimate-use ground is not relied on (camera-vision note) |

## 7. Technology choices

| Area | Choice | Alternatives and why not | Status |
| --- | --- | --- | --- |
| Desktop shell | Tauri 2.12 (Rust plus WebView2) | Electron: heavier. Rust-native egui: the fallback, more UI work. .NET WPF or WinUI: runtime downloads are blocked in the build environment | **B1** |
| Web runtime | Fixed WebView2 155, bundled (about +180 MB) | Evergreen: not guaranteed offline or on Windows 10 | **B1** |
| Non-activating pet window | `WS_EX_NOACTIVATE \| WS_EX_TOOLWINDOW` | Click-through: hurts usability | **B1** |
| UI | TypeScript, Vite, Preact for screens, plain TypeScript for the pet | React: heavier | A |
| Pet art | SVG with CSS animation | Canvas (used in the spike), Lottie | A |
| Core logic | Rust crate `hy-core` shared by companion, server and simulator | Engine in a page: dies with the webview | Doc |
| Store | SQLite (rusqlite, bundled), WAL | JSON files: no transactions | Doc |
| Activity signals | `GetLastInputInfo`, `WTSRegisterSessionNotification`, `WM_POWERBROADCAST`, `SHQueryUserNotificationState` | Keyboard hooks: invasive | Doc |
| Camera and pose | WebView2 `getUserMedia` (Media Foundation); MediaPipe pose lite on CPU | DirectShow (failed in B1); pose full (unmeasured); cloud vision (privacy, cost, internet) | **B1** resources; accuracy A |
| Server | Rust: axum, tokio, rusqlite, Windows service | Node, .NET or Python need a runtime; PostgreSQL needs administration | Doc |
| Pairing and transport | TLS 1.3 (rustls) with a full-fingerprint pin and an opaque single-use secret from a pairing slip | SPAKE2 or other PAKE crates (unaudited); TOFU (no protection on first contact); mutual TLS (more moving parts) | Doc |
| Secrets | DPAPI (user scope on clients, machine scope on the server) | Plain files | Doc |
| Installers | NSIS: per user for the companion, per machine for the server | WiX MSI: only needed for Group Policy | Companion **B1**; server A |
| Update and backup files | minisign-signed packages; `.hybackup` encrypted with AES-256-GCM, key from Argon2id | Unsigned installs; plain zip | Doc |
| Builds | GitHub Actions Windows and Linux runners | None locally | **B1** |

## 8. Installation, update, backup and uninstall

| Topic | Companion | Server |
| --- | --- | --- |
| Distribution | NSIS installer from a LAN shared folder or USB. It is unsigned, so the result on real office PCs must be accepted in an actual test (I4, Q6). Security controls are never bypassed: if SmartScreen, Smart App Control or antivirus blocks it, the result goes to the owner as a signing decision | Run once by an administrator |
| Install | Per user, no administrator, offline, 27 s (B1); autostart; first-run setup | Service, firewall rule, certificate, admin passphrase |
| Update (paired) | Manifest checked daily from the server; minisign signature verified; installs at a quiet moment, never during a reminder or camera session | Administrator uploads a release on the admin page |
| **Update with no server** | Settings → "Install update from file": choose the installer and its `.sig` from USB or a share. The signature is checked against the embedded public key before anything runs; a bad or missing signature is refused | Not applicable |
| Fixed runtime | Releases bundle a newer WebView2 when one exists (packet R9) | Not applicable |
| Migrations | Forward-only, expand then contract, so the previous release can still open the database. The database is copied before each migration | Same, plus a backup first |
| **Rollback, with or without a server** | "Install previous version from file", with the same signature check, works for releases marked rollback-compatible (one release back). If the older release finds a newer schema it cannot read, it restores the pre-migration copy (events since the upgrade are lost, and the app warns first). With a server, the administrator can also pin the previous version | Reinstall the previous binary; restore the pre-migration backup, which enters recovery hold: data is released only on complete evidence, otherwise the server fails closed (5.3) |
| **Full personal backup** | Settings → "Export my data" writes one `.hybackup` file containing settings, goals history, pet edits, sharing state, events (90 days), summaries (24 months) and the consent log. It is encrypted with a password the user chooses. Every row keeps its origin device id (5.4). The token and pin are excluded, so the device must re-pair. "Import" validates the schema version and **replaces** local data, never merging it, to prevent double counting | Backups in 5.1 |
| Uninstall | Program files and the uninstall registry entry are removed (the batch 1 leftover is fixed and tested). A box for "Delete my local data" is unticked by default, with the location shown (the Q7 default) | The data folder is kept unless deletion is ticked |

Test cases T-L1 to T-L7 are in section 11.

## 9. Security

| Threat | Control |
| --- | --- |
| A colleague on the LAN reads or alters data | Pinned TLS 1.3, per-device tokens, user id taken from the token, isolation tests (LAN-01) |
| A rogue server or MITM at pairing | Full fingerprint taken from the slip; the client aborts before sending the secret (5.2) |
| Guessing or replaying a slip | 80-bit secret sent only inside pinned TLS 1.3 without 0-RTT, 24 h validity, atomic single use, rate limits |
| A stolen or retired PC | Administrator revokes the device (a control operation, 5.3); the token is DPAPI-bound to the Windows user |
| The server administrator reads shared data | There is no admin endpoint for habit data, but someone with operating-system access to the server PC can technically open the unencrypted database. This is disclosed (6.2) and limited to daily summaries. Encrypting shared summaries end to end would be a scope change for the owner to decide (Q4); it is not proposed here |
| A deletion or withdrawal is undone | Permanently non-uploadable tombstoned dates, policy seq, one-transaction control log with a mirror, recovery hold that releases restored data only on device receipts plus administrator confirmation, otherwise fail closed (5.3) |
| Camera misuse | Permission only for the camera page; visible indicator; the camera page's content policy blocks all network access |
| Malicious page content or IPC | Strict CSP (the spike had none), local assets only, Tauri capabilities per window, validated commands, no devtools in release builds |
| Tampered update or backup files | minisign signatures; authenticated encryption for backups |
| Supply chain | Pinned lockfiles, `cargo-deny`, `npm audit`, licence register, SBOM, hash-pinned MediaPipe assets, verified upstreams only |
| Unexpected outbound traffic | Edge SmartScreen services disabled in the webview; network capture before release (A12) |

## 10. Observability

Local JSON-line logs are rotated at 7 days or 10 MB. They hold event types and counts, never amounts, notes or aliases. Also on the PC:

- an hourly resource summary of private working set and CPU (the spike's sampler);
- a panic hook, and recreation of a webview after it crashes;
- a diagnostics zip that the user reviews before sending it by hand.

The server's admin page shows uptime, database size, last backup, control high-water and mirror status, recovery-hold status (devices checked in and still awaited), devices last seen, versions and error counts. Sync carries an app-version and queue-length heartbeat. There is no third-party telemetry and no internet traffic.

## 11. Testing strategy

| Level | Coverage | Tool |
| --- | --- | --- |
| Engine | No bursts, one prompt, snooze, precedence, Pomodoro and focus, rollover | `cargo test` with an injected clock, property tests |
| Timers | **T-R1** 50 minutes of reading with no input, unlocked: cue at 45 minutes, once, no reset. **T-R2** away 20 minutes, unlocked: accumulation pauses after 15 minutes, no reset, no stacking. **T-R3** lock 3 minutes: reset; at most one reminder after a 2-minute grace. **T-R4** lock 1 minute: no reset. **T-R5** sleep 2 h then wake: no burst; Pomodoro waits for resume. **T-R6** midnight during Quiet: correct split | Trace replay |
| Store and review | Totals reconcile after corrections; missing day is not zero; month boundaries; insufficient data | Fixture databases |
| Consent and deletion | **T-D1** delayed upload: B is offline with queued hydration; the user deletes on A; B reconnects; the records are rejected and discarded, and the server holds none. **T-D2** offline withdrawal: the queue is purged at once, and network capture shows no payload after reconnect. **T-D3** A disables while B enables with a stale seq: B is rejected and asked again. **T-D4** re-enable after a deletion: only dates on or after `share_from` are sent. **T-D5** restore a backup taken before a deletion, with the mirror intact and complete evidence (every listed device checks in with a receipt at or below the tail, and the administrator confirms): the deleted rows are absent, the category is still off and the revoked device is still revoked; nothing is served until the last piece of evidence arrives. **T-D6** restore with the mirror missing, corrupt or ending before the snapshot's high-water: the server fails closed at once (history discarded, devices revoked, categories off). **T-D7** delete date D, refresh the policy, rebuild D from local events and retry: the device builds no record, and a forced upload is rejected; D is not resurrected. **T-D8** stale but valid mirror: the snapshot is at seq 10, the mirror verifies but ends at seq 12, and a deletion acknowledged at seq 15 is lost with the disk. Devices A and C check in with receipts at or below 12. The server stays in hold and serves no restored data. When B presents its receipt for seq 15, the server takes the discard path. Variant: B never checks in, and the discard path runs when the 7-day window ends. No restored data is served at any point. **T-D9** a receipt at or below the tail whose chain hash does not match the mirror, or a device with an unknown token presenting a signed receipt beyond the tail: discard path. **T-D10** during hold, restore reads, uploads and enables are refused while disables and deletions are accepted; if the administrator does not confirm the device list, the discard path runs | Integration tests against the real server binary |
| Control-log crash points | **T-J1** crash mid-transaction: nothing visible; the client retry applies once. **T-J2** crash after commit, before the mirror append: the mirror is repaired at startup; the retry is idempotent. **T-J3** crash after the mirror append, before acknowledgement: the retry returns the same seq with no duplicate. **T-J4** power loss during the mirror fsync: the chain verifies up to the last complete entry, and repair re-appends from the database. **T-J5** corrupt mirror entry: the chain check fails and restore fails closed. **T-J6** after each crash point the database and mirror high-waters are equal before uploads are accepted | Fault injection (process kill, simulated power loss) |
| Pairing | **T-P1** MITM certificate: aborts before the secret is sent. **T-P2** reused slip rejected. **T-P3** two simultaneous redemptions: exactly one device. **T-P4** expired slip. **T-P5** TLS below 1.3 or 0-RTT refused; a secret captured from one redemption cannot be used again. **T-P6** server IP change: reconnects by host name with the pin; a revoked token gets 401 and the app offers to re-pair | Integration tests plus Codex security review |
| Lifecycle | **T-L1** update from USB with no server. **T-L2** tampered package refused. **T-L3** rollback by one release, with fallback to the pre-migration copy. **T-L4** export, uninstall, reinstall, import: identical review output. **T-L5** two PCs on one day: water summed, exercise day counted once, durations shown per device and never summed. **T-L6** restoring from the server twice: no doubling. **T-L7** migrate, re-pair, re-sync: A shares 10 days; export; import on new PC B; B pairs and syncs, then also restores from the server. Server totals are unchanged, nothing is uploaded under B's id for A's dates, and B's review shows each day once | Scripted on Windows 10 and 11 |
| Camera | **T-C1** static calibration: the floor bounds z, so jitter gives no cue. **T-C2** low variance: floor applied, false cues within the gate. **T-C3** subject motion (reach, turn to a second display, stand): no cue unless the drift is sustained for 60 s. **T-C4** camera relocation (laptop pushed back, screen tilted): suspended within 20 s, recalibration asked, no cue against the old baseline. **T-C5** worker stall: no-reading within 5 s, restart, nothing fabricated. **T-C6** app restart: baseline revalidated or invalidated | Synthetic landmark traces plus desk test |
| V2 gate | 5 setups x 2 sessions, raw-count quality report (4.5) | Harness |
| Windows acceptance | Focus kept, scaling at 100, 125, 150 and 200%, monitor unplugged, presentation mode, Windows 10 and 11, installer I4 result | The owner's PCs, scripted |
| Soak | 8-hour idle (V1-7) **before V1 UAT**; 2-hour camera soak and second webcam (V2-5) **before V2 UAT** | Self-sampler |
| Privacy | Network capture before and after each category; frames never touch disk or network | pktmon, file-activity check |
| Load and UI | 3 real plus 10 simulated clients; keyboard path, reduced motion, chart text alternative, Narrator | Simulator; browser tests with simulated IPC |

GitHub runners only build; acceptance runs on the owner's PCs (packet R19).

## 12. Implementation slices and schedule

The slices start only after the prototype is approved. Effort is in build days for one worker. **These are estimates, not commitments.** At most two workers run at once, and only after V1-0 merges.

| Slice | Content | Depends on | Build days | Owner time |
| --- | --- | --- | --- | --- |
| V1-0 | Scaffold, CI, `hy-core` schema, store, migrations, clock and time zone | Approval | 1.5 to 2.5 | None |
| V1-1 | Pet shell, windows, DPI, tray, keyboard panel; pet packs, ten pets, edits | V1-0, visual direction | 2.5 to 4 | 30 min |
| V1-2 | Tracker (4.3), engine, suppression, focus, Pomodoro | V1-0 | 2.5 to 3.5 | 30 min |
| V1-3 | Hydration, movement content and illustrations, recovery day, phone self-report | V1-2 | 2 to 3 | 30 min |
| V1-4 | Setup, settings, sharing, reviews, export and import | V1-2, V1-3 | 2.5 to 3.5 | 20 min |
| V1-5 | Server, pairing (5.2), policy, tombstones, control log, mirror, receipts and recovery hold (5.3), sync, admin, backup, simulator | V1-0, **pairing gate closed** | 3.5 to 5 | 1 h, two PCs plus server |
| V1-6 | Installers, signed file update and rollback, uninstall fix | V1-1, V1-5 | 2 to 3 | 30 min plus I4 |
| V1-7 | 8-hour soak, network capture, security fixes | All V1 | 1.5 to 3 | Unattended |
| V2-1 | Camera page, capture, worker and watchdog, indicator, off, conflicts | V1-1, V1-2 | 1 to 2 | 20 min |
| V2-2 | Calibration quality, floors, drift, viewpoint check, cue | V2-1 | 2.5 to 3.5 | 30 min |
| V2-3 | Harness, labelled sessions, quality report | V2-2 | 1 to 2 | About 6 h of sessions (P) |
| V2-4 | Meditation pause | V1-2 | 0.5 to 1 | 10 min |
| V2-5 | 2-hour camera soak, Windows 10, second webcam, fixes | V2-1 to V2-4 | 1 to 2 | 1 h |
| **Total** | | | **V1 18 to 27.5; V2 6 to 10.5** | |

Plan to the upper half of the packet's credit ranges, because batch 1 ran above its estimate. After the credits expire, regular included plan usage applies. That is accepted by the owner, and no purchases or paid overages are authorised.

**Resource-loaded forecast.** This is not a deadline; it is pending the owner's agreement. It uses two worker chains with likely (mid-range) durations and adds half a day per slice for Codex review and fixes (a quarter day for V2-4). Days run on the calendar, and owner tests use the slots below. Day 0 is 30 Oct, the forecast prototype approval.

| Dates (2026) | Worker A | Worker B | Owner / reviewer |
| --- | --- | --- | --- |
| 22 to 29 Oct (before approval: design and content only) | Architecture closure; V1 and V2 wireframes and prototype | Pairing and convergence contracts for security review; movement-content matrix and sources | Codex design review 28 to 29 Oct; owner prototype review 29 to 30 Oct |
| 30 Oct to 1 Nov | V1-0 | Waits for V1-0 to merge | Review V1-0 |
| 1 to 4 Nov | V1-2 | V1-1 (1 to 5 Nov) | **Pairing gate sign-off needed by 5 Nov** |
| 5 to 7 Nov | V1-3 | V1-5 (5 to 9 Nov) | Slot 5 Nov 18:00 (pet, timers); 8 Nov 11:00 (movement content) |
| 8 to 11 Nov | V1-4 | V1-6 (10 to 12 Nov) | Slot 10 Nov 11:00 (server install, deployment checks, LAN pairing); 11 Nov 18:00 (screens); 12 Nov 18:00 (installer I4, updates) |
| 12 Nov | V2-4 | V1-6 finishes | Review |
| 13 to 15 Nov | V1-7: 8-hour soak overnight, network capture, security fixes | Half a day of V1-7 fixes, then V2-1 (13 to 15 Nov) | Codex security review of V1 |
| 16 to 18 Nov | V1 UAT fixes, frozen V1 release | V2-2 (15 to 18 Nov) | **V1 UAT** 16 and 17 Nov, 11:00 or 18:00 (2 x 90 min); 18 Nov 18:00 camera check |
| **19 to 21 Nov** | V1 pilot support | V2-3 harness | **V1 pilot, 3 working days** (start can move to 23 Nov if 21 Nov is not a working day) |
| 20 to 22 Nov | Helps with V2-3 report | V2-3 desk test | Desk-test slots 11:00, 15:00 and 19:00 (5 setups x about 70 min) |
| 23 to 24 Nov | V2-5 fixes | V2-5: 2-hour camera soak, Windows 10, second webcam | Slot 24 Nov 11:00 (1 h) |
| 25 to 26 Nov | V2 UAT fixes | V2 UAT fixes | **V2 UAT** |
| **From about 27 Nov** | Pilot support | Pilot support | **V2 pilot, 3 working days** |

**Critical path.** It runs: prototype approval → V1-0 → (V1-1 → V1-5 → V1-6 on worker B, alongside V1-2 → V1-3 → V1-4 on worker A) → V1-7 soak → V1 UAT → V1 pilot. The pairing gate sits on this path, because V1-5 cannot start without it. For V2 the path is V1-6 → V2-1 → V2-2 → V2-3 (owner desk sessions) → V2-5 → V2 UAT → V2 pilot. The V2 pilot starts after the V1 pilot ends, so the same three people are not running two pilots at once. With the low and high ends of every range, the V1 pilot start moves between about 15 and 23 Nov and the V2 pilot start between about 22 Nov and 1 Dec.

**Earliest credible milestones** with the full agreed V1 and V2 scope: V1 pilot from about 15 Nov and V2 pilot from about 22 Nov, low case. The likely starts are 19 Nov and 27 Nov. The recorded 5 Nov target is not replaced by this document. Whether to adopt these forecasts, or change scope instead, is the owner's decision.

**Why the dates moved from packet revision 2.** The packet had V1 to V3 in a 3-day pilot from 2 to 4 Nov, with a 6-day build window (26 to 31 Oct) for 16 slices, and the packet itself flagged that window as unproven (B3). Since then:
- the feasibility gate moved to 22 Oct;
- architecture took two review rounds;
- the prototype starts 24 Oct;
- the bottom-up V1 estimate is 18 to 27.5 build days, and the worker chains, review time and soak-before-UAT turn that into about 16 calendar days rather than the lower bound of total work divided by two;
- the consent-convergence, standalone-lifecycle and camera-validity work were added in review.

Deferring V3 does not shorten the V1 path.

The V1 pilot runs on a frozen V1 release while V2 work continues on a branch. **Any change to scope or dates is the owner's decision.**

## 13. V3 extension points (interfaces only; nothing is built)

| Interface | V3 purpose | Now |
| --- | --- | --- |
| `SyncTarget` (base URL, trust, token) | The hosted service speaks the same `/v1` protocol, policy log and tombstones | LAN implementation only; protocol versioned |
| Event `source = imported`, `source_ref`, `observed_at` against `received_at` | Wearable imports with provenance and freshness | Fields exist; the value is rejected |
| Category registry | `wearable_summary`; a separate community display payload | Fixed list; adding one is a data change plus a new notice version |
| `ProfilePublisher`, `HabitCreditCalculator` | Same avatar; bounded habit credit | Not implemented; daily summaries already hold the inputs |
| Membership invariant; user → organisation | At most one active group (unique partial index); multi-office hosting | Not implemented; one implicit organisation |

## 14. Unverified assumptions

| # | Assumption | How and when it is checked |
| --- | --- | --- |
| A1 | The server PC (Windows 11, 12th-gen i7, 16 GB, owner-reported) can be used: an administrator can install the service and firewall rule; every pilot PC reaches it on the same LAN (no guest or isolated Wi-Fi); its power and sleep settings keep it reachable in office hours. Backups do not assume overnight uptime (5.1) | Install-day deployment checks in the 10 Nov slot (V1-5); mirror location (second disk or network folder) recorded |
| A2 | Memory and CPU stay flat over hours (camera peak 301 MB is 86% of the 350 MB budget, measured over 10 minutes) | 8-hour idle soak (V1-7) and 2-hour camera soak (V2-5), both before UAT; then the pilot |
| A3 | Capture works on Windows 10 and on USB or second webcams (the one Windows 10 webcam was not detected) | Every pilot PC's webcam in V2-1 and V2-5, before UAT |
| A4 | Drift from calibration meets the V2 gate | Desk test V2-3 with raw counts; any threshold change needs the owner's approval |
| A5 | An unsigned installer is accepted on office PCs without bypassing any control | Actual I4 result in V1-6 on Windows 10 and 11; a block becomes a signing decision |
| A6 | Pet is correct at 150% and 200% and after a monitor is unplugged | V1-1 acceptance |
| A7 | Idle stays at or below 150 MB with real SVG art and the Preact UI | Repeat the P1 script after V1-1 and V1-4 |
| A8 | The hotkey panel takes focus and Escape returns it | V1-1 manual check |
| A9 | Inference in a worker in a background page keeps 2 to 3 fps; the watchdog catches stalls | V2-1 measurement; T-C5 |
| A10 | The notification-state API reports presentation and full-screen mode (and Do Not Disturb) | 20 scripted trials in V1-2; the manual toggle always works |
| A11 | The camera or microphone in-use state is a usable meeting hint | Off by default; 20 trials including false positives before enabling |
| A12 | WebView2 makes no outbound connections with the chosen settings | Network capture in V1-7 |
| A13 | Legal applicability (DPDP, its timing, the IT Act rules) | The firm confirms before the pilot (Q3) |
| A14 | Pasting or typing the roughly 80-character slip is acceptable to users | V1-5 slot; fallback is a printed slip |
| A15 | Per-user install and autostart are allowed by office policy | First install in V1-6 |
| A16 | About 180 MB runtime updates are acceptable on the LAN and by USB | Measured in V1-6; the runtime ships only when it changes |
| A17 | The viewpoint heuristics separate camera movement from posture without many false suspensions | Relocation trials in V2-3 (T-C4); suspended time reported separately |
| A18 | All office PCs are x64 and ten original pets fit the prototype and V1-1 effort | Inventory on install day; visual direction at the prototype gate |
| A19 | After a server restore, every paired device can check in within 7 days and the administrator can confirm the device list. If not, all three users lose restored server history and re-pair (5.3); their local data is kept | Accepted as the safe default for 3 users; T-D8 and T-D10 |

## 15. Owner questions

| # | Status | Question or recorded default |
| --- | --- | --- |
| Q1 | Partly answered (Windows 11, 12th-gen i7, 16 GB) | Remaining items become deployment checks (A1) on install day: administrator access, same-LAN reach, power and sleep settings. Also confirm all office PCs are x64 |
| Q2 | Default kept, not blocking | Server purpose: the employee's own summary history; no manager view |
| Q3 | Needed before the pilot | Named firm contact for privacy and deletion requests; the firm confirms the notice wording and legal applicability |
| Q4 | A choice, explained | Shared summaries are stored unencrypted, and the server's administrator could technically open them (disclosed). The default is disclosure only. End-to-end encryption would be a scope change for the owner and is not planned |
| Q5 | Availability answered (10:00 to 22:00 IST daily; all three users) | Please confirm the proposed slots (section 12), whether Saturdays count as pilot working days, and the forecast dates as a whole |
| Q6 | Needs an actual result | The I4 installer test on office PCs decides whether an unsigned build is acceptable; no security control is bypassed |
| Q7 | Default kept, not blocking | Uninstall keeps local data unless the user ticks delete |
| Q8 | Resolved | Regular included plan usage after the credits expire is acceptable; no purchases or paid overages. Not a gate |

## Appendix: requirement traceability

| Requirement | Section | Slice |
| --- | --- | --- |
| PET-01 to PET-04 | 4.1, 4.2, 4.4 | V1-1 |
| RHY-01 to RHY-04, FOC-01 | 4.2, 4.3 | V1-2 |
| WAT-01, WAT-02, MOV-01, MOV-02, REC-01, DIS-01 | 4.2, 6.1 | V1-3 |
| REV-01, REV-02 | 4.2, 5.4 (multi-device merge) | V1-4 |
| SHR-01, SHR-02, LAN-01 | 5.2, 5.3, 6.2, 9 | V1-4, V1-5 |
| PRD section 9 (install, update, backup, rollback, performance) | 5.1, 8, 10, 11 | V1-5 to V1-7 |
| CAM-01, CAM-02, V2 gate | 4.5 | V2-1 to V2-3, V2-5 |
| CAL-01 | 4.6 | V2-4 |
| DIS-02 | Backlog (packet) | None |
| CON-01, CON-02, COM-01, COM-02 | 13 (interfaces only) | Deferred |
