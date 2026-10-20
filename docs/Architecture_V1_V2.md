# HealthierYou architecture proposal: V1 and V2

Status: **proposal for review** by Codex (independent review) and the owner. Drafted 20 Oct 2026 for the 22 Oct review.
Inputs: PRD 0.3 with the recovery, focus and community addendum (approved 18 Oct; this repository's `docs/PRD.md` still shows 0.1), delivery plan and feasibility packet revision 2, feasibility batch 1 results (five runs, final build 6ef6eb3), the camera-vision options note (20 Oct), and the batch 1 spike on branch `spike/batch1`.

**Not approved:** the prototype and product development. This document contains no product code. The gates are unchanged: architecture review, then wireframes and a clickable prototype, then the owner's prototype approval, then code.

**Scope (owner decision, 20 Oct):** build V1 and V2 first. V3 (hosted offering, wearables, community rankings, one group per user) is deferred, not removed. This document only shows where V3 would plug in (section 13) and contains no wearable research.

Labels used throughout:

| Label | Meaning |
| --- | --- |
| **B1** | Proven by feasibility batch 1 on one 8 GB i5 Windows 11 laptop, offline, as a standard user |
| **Doc** | Vendor documentation or source code read; not yet run by us |
| **A** | Assumption; how each one is checked is in section 14 |
| **P** | The PRD specifies it |
| **D** | The PRD is silent; this is a default chosen here, configurable where sensible, and open to review |

## 1. Summary

- **Desktop companion:** a Tauri 2 app per Windows user: a Rust core plus WebView2 pages, a bundled fixed WebView2 runtime and a per-user NSIS installer, all B1. One Rust "coaching engine" owns every timer and reminder decision. Pages only draw the pet and the screens. Data is local-first in SQLite, and every V1 and V2 feature works with no server.
- **V2 posture:** MediaPipe pose lite on the CPU at 2 to 3 fps. It runs in a separate camera page that exists only while the user has the camera on (resource use B1, accuracy unproven). The output is drift from a per-user calibration, not clinical angles. Frames and landmarks never leave memory.
- **LAN server:** one Rust binary running as a Windows service, with SQLite and HTTPS behind a self-signed certificate. Each device pins that certificate during one-time code pairing. The server receives only the daily summary categories each user switched on, and only that user can see them. The administrator sees device and service health, never habit data.
- **Schedule estimate:** V1 pilot-ready about **16 to 21 Nov 2026**, V2 about **25 Nov to 2 Dec 2026**, assuming the prototype is approved by 30 Oct. **The original 5 Nov date for all versions is not committed.** Section 12 has the detail.

## 2. Goals and non-goals

| Goals (V1 and V2) | Non-goals |
| --- | --- |
| Pet-led reminders that never take keyboard focus or block input (P) | Clinical measurement, diagnosis, productivity or performance scoring (P) |
| Combined reminders, focus, Pomodoro, meeting and presentation suppression, with no burst of prompts after lock or sleep (P) | Forced breaks, input blocking, streak punishment or default sound (P) |
| Water, six-day movement programme, recovery day, phone self-report, weekly and monthly review (P) | Manager dashboards, rankings, or admin access to personal charts (P) |
| Works fully offline; the server is optional per user and per category (P) | Cloud AI, any cloud service, or internet use at run time (P for V1; D for V2) |
| V2: calibrated drift cues with confidence, a visible camera state and an instant off switch (P) | Angle accuracy, emotion, drinking or exercise verification, phone-use detection (P) |
| V2: a simple text-guided meditation pause (P) | V4 sound, headphone and colour sessions (P) |
| Within budget on an 8 GB PC: idle at most 150 MB and 2% CPU; camera at most 350 MB and 10% CPU (P) | Supporting 4 GB PCs (owner, 20 Oct: the minimum is now 8 GB) |
| Install without an administrator; no prerequisites for the employee to install (P, B1) | Code signing, store distribution or hosted services (no purchase approved) |
| Leave room for V3 without building it now | Wearables, the hosted service, community features and avatar sync (V3, deferred) |

## 3. Context

```
                         Office LAN (one subnet, private network profile)
 +------------------------------------------+         +-----------------------------------+
 | Employee PC (Windows 10/11, 8 GB+)       |         | Office server PC (spec unknown)   |
 |                                          |  HTTPS  |                                   |
 |  HealthierYou companion (per user)       |  pinned |  HealthierYou server (service)    |
 |  +------------+   +-------------------+  |  cert   |  +-----------------------------+  |
 |  | Rust core  |<->| pet page (always) |  |<------->|  | API: pair, policy, sync,    |  |
 |  | engine,    |   | screens (on open) |  |  mDNS + |  |      delete, updates        |  |
 |  | store, sync|   | camera page (V2,  |  |  manual |  | SQLite: users, devices,     |  |
 |  +-----+------+   |  only while on)   |  | address |  |   opted-in daily summaries  |  |
 |        |          +---------+---------+  |         |  | Admin page: localhost only  |  |
 |  SQLite (%LOCALAPPDATA%)    | webcam     |         |  | Nightly backups             |  |
 |  Windows signals: input idle, lock,      |         |  +-----------------------------+  |
 |  sleep, presentation mode                |         +-----------------------------------+
 +------------------------------------------+                         ^
          ^                                                           | admin installs once,
          | employee: pet, settings, reviews                          | creates pairing codes,
                                                                      | uploads releases
 No internet use at run time. No images, landmarks or keystroke contents leave the PC.
 V3 (deferred): a hosted service speaking the same /v1 protocol (section 13).
```

## 4. Desktop companion

### 4.1 Process and window model

| Part | Lifetime | Notes |
| --- | --- | --- |
| Rust host process | Always, while signed in | Engine, store, Windows signals, sync, updater, tray icon. Autostart through the per-user Run key (D). Single instance per user |
| WebView2 browser and GPU processes | Always | One shared environment from the fixed runtime in the install folder (B1, version 155) |
| Pet page | Always | Transparent, frameless, always on top, skipped in the taskbar, `WS_EX_NOACTIVATE \| WS_EX_TOOLWINDOW` (B1: P3 kept focus 10 of 10). The window resizes to fit the compact or expanded bubble, so no second window is needed |
| Screens page | Created on open, destroyed on close | Setup, settings, goals, pet library, sharing, weekly and monthly review. A normal focusable window |
| Pet panel (keyboard path) | On demand | Opened from the tray icon or a global hotkey (default Ctrl+Alt+H, configurable or off; D). It takes focus on purpose and closes with Escape, which hands focus back (PET-04; A) |
| Camera page (V2) | Only while the camera is on | Its own webview, so its memory is returned when the camera stops. Camera permission is granted only to this page (section 9) |

Idle memory with the pet and a control window was 95 MB (B1). The product's idle state has fewer pages open, but uses real art and a UI framework (A, re-measured in slice V1-1).

### 4.2 V1 components

| Component | Responsibility | Requirements |
| --- | --- | --- |
| Pet shell | Placement at a user-chosen edge, drag and snap, scale 75 to 150% (D), per-monitor DPI, repositioning when a monitor is removed, hidden during presentation or full-screen apps (D), the eight pet states, reduced motion, tray menu (show, hide, pause coaching, quit) | PET-02 to PET-04 |
| Pet library | Ten data-only pet packs plus per-user edits and reset (4.4) | PET-01, PET-02 |
| Activity tracker | Polls input idle every 5 s; listens for lock, unlock, sleep and resume; builds sessions (4.3) | RHY-01 |
| Coaching engine | One deterministic state machine for movement, eye rest, hydration, focus, Pomodoro, recovery day, meditation (V2), posture cues (V2), snooze and cooldown. Makes the only "show a bubble" decisions | RHY-02, RHY-03, FOC-01, REC-01, CAL-01, CAM-02 |
| Suppression | Pauses coaching for: the user's pause, lock, sleep, off-hours, focus, Pomodoro work, presentation or full-screen mode (Windows notification state API; Doc), and a manual "in a meeting" toggle for 30, 60 or 90 minutes or until turned off. An optional "camera or microphone in use" hint is experimental, off by default and labelled that way (A) | RHY-04 |
| Hydration | Goal and serving sizes, quick log, custom amount, undo and correct, pacing across work hours, no encouragement after the goal is reached | WAT-01, WAT-02 |
| Movement programme | Bundled, versioned Monday to Saturday content with seated and standing variants, steps, skip and stop, partial and complete labels. Sunday is optional | MOV-01, MOV-02 |
| Recovery day | Marking a deadline day, an optional neutral check-in after a long active day (over 9 h, D) that the user confirms, one pending plan, and a day profile with more short-break opportunities, an optional walk, a lunch-away reminder, a shutdown time and one reflection | REC-01 |
| Pomodoro | Off until Start; 25/5, 50/10 or custom; pause, resume and end; auto-restart only when enabled; precedence as in the addendum | FOC-01 |
| Phone self-report | One-action event; work, personal or unsure label; editable | DIS-01 |
| Review engine | Weekly and monthly figures and up to three rule-based suggestions, all computed locally from events; coverage-aware; text alternative for charts | REV-01, REV-02 |
| Screens | First-run setup (PRD journey), settings, goals, pet library, sharing and privacy, review, connection status | Flows 1 to 8 and 11 |
| Store | SQLite with WAL, migrations, retention jobs (section 6) | PRD section 8 |
| Sync client | Pairing, policy, outbox, retry, deletion requests (section 5) | LAN-01, SHR-01, SHR-02 |
| Diagnostics | Rolling logs, resource self-sampler, diagnostics export the user reviews first (section 10) | PRD section 9 |

### 4.3 Coaching engine rules

Session model (D unless marked P): **active** means input within the last 60 s. **Reading** means 1 to 5 minutes without input while unlocked, and it still counts as work. **Away** means 5 minutes or more without input, or locked. Away resets the movement timer but is never recorded as movement, because the UI must not treat input idle as proof of movement (P). After unlock, wake, or an away spell of 30 minutes or more, there is a 2-minute grace period and then at most one combined reminder (P: no burst). Sessions that cross midnight are split for reporting by the user's chosen time zone (P, default Asia/Kolkata).

Default timings, all P unless marked D and all configurable:

| Rule | Default |
| --- | --- |
| Movement cue | After 45 minutes of accumulated active work |
| Eye-rest cue | Every 20 minutes (optional); folded into a movement break that is due soon |
| Hydration | Joins an eligible reminder when the user is behind their own pace. On its own only when 2 servings behind and no reminder in the last 30 minutes (D) |
| Cooldown, snooze | 5 minutes between full reminders; snooze 10 minutes. One snooze covers the whole combined reminder |
| Focus | 30, 60 or 90 minutes or custom; ends without replaying missed reminders |
| Pomodoro | 25/5, optional 50/10 or custom; lock or sleep pauses it until the user resumes it |
| Posture cue (V2) | 30-minute cooldown |
| Repeated dismissals | 3 dismissals in a day (D) lead to a gentle offer to reduce intensity |

Precedence, highest first: user pause, lock, sleep or off-hours (P) → focus, presentation or meeting (P) → active exercise, break or Pomodoro (P) → one combined reminder (P). Each evaluation produces at most one visible prompt with a stated reason. The engine runs on an injected clock, which keeps it testable by replaying recorded traces (section 11).

### 4.4 Pet library and asset format

Ten pets share one set of behaviours (P). Each pet is a **data-only pack**; no executable plugins are allowed (P).

| Part | Format (D) |
| --- | --- |
| `pet.json` | id, version, display name, licence and provenance, author, tone presets, palette slots, hitbox, and a map from each of the eight states (idle, cue, exercise, waiting, celebration, snoozed, focus, disconnected) to an animation. Validated against a JSON schema at build and at load |
| Art | SVG parts animated with CSS keyframes, so the pet stays sharp at 100 to 200% scaling. Colour variants come from CSS variables bound to palette slots. A static pose for each state serves reduced-motion mode. Sprite sheets at 1x and 2x are the fallback if an SVG pet costs too much CPU (A) |
| Wording | A message catalogue keyed by tone (for example cheerful, calm, brief; D) and by string id. English first, ready for translation (P) |
| User edits | Stored per user: name, scale, colour variant, placement, motion level, tone. "Reset to defaults" clears them (P) |
| Animation budget | 12 fps while animating (B1 measured 1.1% idle CPU); animation pauses during idle, focus and while hidden (D) |

Adding a pet means adding a folder and running the schema check. Art is original, created during the prototype phase after the owner chooses one visual direction (P).

### 4.5 V2 webcam posture

| Aspect | Design |
| --- | --- |
| Opt-in | The camera is off by default (D). Turning it on starts calibration. Turning it off is one click on the pet or in settings |
| Capture | `getUserMedia` at 640x480 in WebView2, on Windows' default Media Foundation path (B1; DirectShow was rejected because the light stayed on and Windows' Deny setting was ignored) |
| Inference | MediaPipe tasks-vision 1.1.1, pose lite, CPU delegate, 2 to 3 fps (B1: 37 ms per frame, 4.9% CPU, 301 MB peak). Moved into a Web Worker (A; the spike ran it on the page's main thread). Falls back to 1 fps when the CPU budget is exceeded (D) |
| Features | Measured only when the nose, eyes or ears, and both shoulders have visibility of at least 0.5 (D). Ear-to-shoulder gap divided by shoulder width (slouch), shoulder width compared with the baseline (leaning in), shoulder tilt, head roll, presence. Hip and leg points are ignored |
| Calibration | 20 to 30 s of comfortable posture with at least 40 valid frames (D). Stores the median and median absolute deviation per feature, plus the face position and size, so a moved camera can be noticed |
| Drift and cue | A robust z-score per feature, smoothed over 10 s. An episode starts when the drift has lasted 60 s with valid readings in at least 70% of that window. The cue shows within 30 s of the threshold (P gate) and only if the engine allows it. One episode produces one event. Thresholds are tuned in the desk test and frozen before scoring (P) |
| No reading | Absent, occluded, low confidence, camera busy, denied or unplugged each show an explicit no-reading state. No score is inferred from missing data, and V1 carries on (P) |
| Recalibration | Suggested, never forced, when the camera device changes or the face position or size moves persistently (more than 10 minutes, D) |
| Pausing | Capture stops during lock, sleep, off-hours, presentation and the manual meeting toggle. During focus, cues are dropped rather than queued (D) |
| Stop | Stops the track, destroys the camera page and logs the time; 0.35 s release measured (B1). A release longer than 2 s blocks the release (P) |
| Stored | Only derived `posture.observation` events (episode start and end, feature, size band, confidence, valid-frame coverage) and daily camera-on coverage. Landmarks live only in a rolling in-memory window of 90 s or less. Frames are never written or sent |
| Desk test harness | A labelling mode where an operator marks staged drift, absence and occlusion with keys while the detector logs. It produces raw counts per setup for the PRD's V2 gate (5 setups, 2 sessions each, at least 70% coverage, at least 80% detection, at most 1 false cue per 30 minutes). No video is stored |

### 4.6 V2 meditation pause

One optional daily invitation at a time the user picks (off until a time is set; D). Text-guided steps with eyes-open and non-breath-focused alternatives, lasting 2, 5 (default) or 10 minutes, with pause and stop. The invitation follows the focus, meeting and off-hours rules. If missed, it is skipped for that day, so nothing builds up (P, CAL-01). It never starts on its own and plays no audio (P). Sessions are logged locally only (D).

## 5. LAN server

| Function | Design |
| --- | --- |
| Packaging | One Rust binary running as a Windows service, with an administrator installer used once. Data lives in `%ProgramData%\HealthierYou\Server`, readable only by SYSTEM and Administrators. One inbound firewall rule on TCP 8443 (D), private and domain profiles only |
| Discovery | DNS-SD advertisement `_healthieryou._tcp` (Doc), with a typed address or host name as the fallback. A discovered server is only a candidate until pairing confirms it |
| Transport | HTTPS through rustls with a self-signed certificate created at install. Clients pin its fingerprint during pairing. A certificate change is announced by the old key, or the device pairs again |
| Users | The administrator creates a user with a display alias only: no email, phone or employee number (D) |
| Pairing | The administrator creates a single-use code for a user (12 base32 characters, about 60 bits; valid 24 h; 5 attempts; D). The companion proves it knows the code and the server proves it too, using an HMAC challenge keyed by an Argon2id hash of the code and bound to the server certificate's fingerprint and a client nonce. Then the device is issued a random 256-bit token, which the companion stores with Windows DPAPI and the server stores only as a hash. A PAKE (SPAKE2) is the alternative if security review prefers it (A; R20 in the packet) |
| Authorisation | Every request is tied to a user and device through its token. The user id always comes from the token, never from the request body. No endpoint returns another user's data (LAN-01) |
| Sharing policy | Per user and per category, versioned. A change made on a device is sent at once and also queued. If two changes conflict, the more restrictive one wins. The server refuses uploads for categories that are currently off, so turning a category off on one PC also stops a second PC (SHR-02) |
| Sync | The outbox pushes daily summary records keyed by user, device, category and local date, plus a revision number. The server upserts when the revision is higher, which makes retries idempotent and duplicates impossible. Retries back off from 30 s to 15 minutes, with jitter |
| Offline and reconnect | The companion never waits on the server. On reconnect it pulls the policy first, drops queued records for categories now off, then pushes. The pet shows a small disconnected badge, and settings explain the status (P) |
| Deletion | "Delete what I shared" (per category, all or a date range) creates a request with its own id. The server deletes from the active store and returns a receipt. The companion shows pending, done or failed with retries, and says that backups expire within 30 days (P: active store and backups are told apart) |
| Restore | When a paired user reinstalls or moves PC, shared summaries can be pulled back and are labelled with the device they came from (D). That continuity is the employee's reason to share at all |
| Admin | A web page served only on the server's localhost, behind an admin passphrase hashed with Argon2id. It shows service health, the database size, the last backup, users and devices (alias, version, last seen, revoked), pairing codes, the update channel, and an audit log of admin actions. It never shows habit data (P) |
| Backup | A nightly online SQLite backup, kept 14 days plus 2 weekly copies, 30 days at most (D). Restore runs from the admin page or the command line and is demonstrated before the pilot (P) |
| Load | Target: 3 real clients plus 10 simulated clients (P), driven by a simulator built from the same core crate |

Server memory and CPU are expected to be small (a single binary with SQLite), but the server PC's specification is unknown (A1).

## 6. Data model, placement and privacy

### 6.1 Local store (per Windows user, `%LOCALAPPDATA%\HealthierYou\data\hy.db`)

| Table | Contents | Retention |
| --- | --- | --- |
| `events` | Append-only. id (UUIDv7), schema version, type, occurred-at in UTC, IANA time zone, local date, source (manual, inferred, system; imported reserved for V3), measurement status (self-reported, inferred, measured), confidence, a `supersedes` link for corrections, and a schema-validated payload | 90 days (P) |
| `daily_summary`, `monthly_summary` | Derived figures per category with coverage, and a revision number | Daily 90 days; monthly 24 months (P) |
| `goals_history` | Goal values with an effective-from date, so a week is judged against the goal set at the time (P) | 24 months |
| `settings`, `pet_profile` | Preferences and pet edits | Until changed |
| `sharing_policy`, `consent_log` | Category states, policy version, notice version shown, timestamps | 24 months (D) |
| `outbox`, `deletion_requests`, `server_link` | Sync state. The device token is held as a DPAPI-protected blob | Until delivered |
| Bundled content | Movement routines, meditation script, pet packs, all versioned and read-only | Ships with each release |

Corrections and undo are new events that supersede old ones, so totals reconcile after edits, day changes and sync retries (WAT-01). Time-zone changes never rewrite past local dates (P).

### 6.2 What stays on the PC and what can reach the server

| Data | On the PC | Can reach the server |
| --- | --- | --- |
| Camera frames, landmarks | Memory only, never stored | **Never** |
| Keystrokes, window titles, app names, screen contents | Never collected (P) | Never |
| Detailed events (every log, prompt and step) | Yes, 90 days | Never (D: only daily summaries are shared, to collect as little as possible) |
| Hydration daily summary (ml logged, goal, number of entries) | Yes | Only if the user switches it on |
| Reminder and exercise outcomes (offered, done, snoozed, dismissed; routine done, partial or skipped) | Yes | Only if on |
| Work-session summary (active minutes, longest block estimate, focus and Pomodoro counts) | Yes | Only if on |
| Phone self-reports (counts by label) | Yes | Only if on |
| Camera summary, V2 (camera-on minutes, coverage, episode counts by type) | Yes | Only if on |
| Meditation, recovery plans, reflections | Yes | Never in V1 or V2 (D) |
| Operational data: device id, user alias, app version, Windows build, last seen, pairing state, policy version | Yes | Yes, once paired. Disclosed separately and never described as optional health sharing (P) |

All optional categories are off by default (P). Before a category is switched on, the screen shows a preview of the actual record and who can see it ("you; the administrator sees device status only") (SHR-01).

### 6.3 Consent and the DPDP Act

| Obligation (DPDP Act 2023 and Rules 2025) | How the design meets it |
| --- | --- |
| Notice in plain language: purpose, data, rights, contact | Shown at pairing and on each category screen. Purpose: the employee's own continuity and backup. The contact is a person the firm names (Q3) |
| Consent that is free, specific and given by a clear action | Separate opt-in per category. Pairing itself is optional, local use needs no server, and there is no manager view, which reduces employer pressure |
| Withdrawal as easy as giving | One toggle. Unsent data for that category is discarded at once. Deleting what was already shared is a separate, tracked action (P) |
| Purpose limitation and erasure | No aggregate or secondary use. Server retention is the same as the local retention or shorter (P), and backups last 30 days at most |
| Reasonable security safeguards and breach readiness | Section 9. The admin audit log and service logs carry no habit payloads (P) |
| Not relying on the "employment" legitimate-use ground for habit data | Habit categories rely on consent only; the employment ground is not used for them (camera-vision note) |

The main DPDP obligations are expected to apply from about May 2027 (from commentary, not verified), and the IT Act's sensitive personal data rules may still apply until then. Health data counts as sensitive under those rules. The design aims to satisfy both now, but this is not legal advice. The firm should confirm (A13).

## 7. Technology choices

| Area | Choice | Alternatives and why not | Status |
| --- | --- | --- | --- |
| Desktop shell | Tauri 2.12 (Rust and WebView2) | Electron: larger runtime and memory. Rust-native UI (egui): the fallback, but more UI work. WPF or WinUI on .NET: needs a .NET or Windows App SDK runtime, and those download hosts are blocked in the build environment | **B1** |
| Web runtime | Fixed WebView2 155 bundled in the installer (about +180 MB) | Evergreen system runtime: smaller, but it is not guaranteed offline or on Windows 10 and its version drifts | **B1** |
| Non-activating pet | `WS_EX_NOACTIVATE \| WS_EX_TOOLWINDOW` | Click-through windows hurt usability | **B1** |
| UI code | TypeScript, Vite, Preact for the screens, plain TypeScript for the pet | React (heavier), Svelte (equivalent). Kept small to protect the idle budget | A |
| Pet art | SVG with CSS animation and palette variables | Canvas (used in the spike, harder to author), Lottie (runtime size) | A |
| Core logic | Rust crate `hy-core` shared by companion, server and simulator | Engine in the page: harder to test and dies with the webview | Doc |
| Local store | SQLite through rusqlite (bundled), WAL mode | JSON files (no transactions); embedded key-value stores (weaker queries for reviews) | Doc |
| Activity signals | `GetLastInputInfo`, `WTSRegisterSessionNotification`, `WM_POWERBROADCAST`, `SHQueryUserNotificationState` | Keyboard hooks: invasive, not needed | Doc |
| Meeting hint | Windows camera and microphone in-use state, read-only, labelled as a hint | Calendar integration (out of scope); window titles (excluded by the PRD) | A (undocumented) |
| Camera | WebView2 `getUserMedia` on Media Foundation | DirectShow (failed C3 and C4 in batch 1); native capture (more code, unneeded) | **B1** (one webcam) |
| Pose | MediaPipe pose lite, CPU, 2 to 3 fps | Pose full (unmeasured, about 2.4 times slower); GPU delegate (driver and leak risk); cloud vision model (privacy, cost, needs internet) | **B1** resources; accuracy A |
| Server | Rust: axum, tokio, rusqlite, Windows service | Node, .NET or Python (runtime to install); PostgreSQL (administration overhead) | Doc |
| Transport | HTTPS through rustls, self-signed, pinned at pairing | Mutual TLS with client certificates (more moving parts); plain HTTP (unacceptable) | Doc |
| Discovery | DNS-SD plus a typed address | UDP broadcast (custom code, more firewall exceptions) | Doc |
| Pairing | One-time code, mutual HMAC with Argon2id | SPAKE2 (prevents offline guessing, but the crate is unaudited) | A |
| Secrets on the PC | Windows DPAPI, per user | A plain file (unprotected); Credential Manager (similar, more API) | Doc |
| Installers | NSIS per user for the companion; NSIS per machine for the server | MSI through WiX (only needed for Group Policy rollout) | Companion **B1**; server A |
| Updates | Tauri updater plugin pointed at the LAN server, packages signed with minisign | Manual reinstall (no rollback control); Windows Store (out of scope) | Doc |
| Builds | GitHub Actions Windows runner; Linux runner for core tests | Local Windows builds (none available) | **B1** |

## 8. Installation, update and uninstall

| Step | Companion | Server |
| --- | --- | --- |
| Distribution | Unsigned NSIS installer copied from a LAN shared folder (D). SmartScreen behaviour on a downloaded or shared-folder copy is untested (A5) | Installer run once by an administrator |
| Install | Per user, no administrator, offline, 27 s (B1). Installs to `%LOCALAPPDATA%\Programs`, registers autostart, then runs first-run setup | Service, firewall rule, certificate, admin passphrase, backup folder |
| Update | Checks the server's update manifest daily and on start (D), downloads, verifies the minisign signature, and installs in the background at the next quiet moment, never during a reminder or camera session. Releases bundle a newer fixed WebView2 when one exists, because the fixed runtime does not update itself (packet R9) | The administrator uploads a release on the admin page, or replaces the binary and restarts the service |
| Database migrations | Forward-only. The database is copied before each migration. Changes are expand-then-contract so that the previous release can still read the database (P: rollback) | Same, plus a backup before migrating |
| Rollback | The administrator pins the previous version on the server, and companions reinstall it. Migration compatibility is tested on sample data (P) | Reinstall the previous binary and restore the pre-migration backup |
| Uninstall | From Windows Settings. Program files and the uninstall registry entry are removed (batch 1 left the registry entry behind; it will be fixed and covered by a test). A check box offers to delete local data (D: unchecked, with its location shown). Settings also has "Delete all my data on this PC" | The uninstaller keeps the data folder unless the administrator ticks delete |

## 9. Security

| Threat | Control |
| --- | --- |
| Another employee on the LAN reads or alters someone's data | TLS with a pinned certificate, per-device tokens, user id taken from the token, tests that two clients are isolated (LAN-01) |
| A rogue server on the LAN | Pairing proves both sides know the code; discovery alone confers no trust |
| Guessing pairing codes | About 60-bit codes, 24 h expiry, 5 attempts, rate limiting, single use |
| A stolen or retired PC | The administrator revokes the device; the token is DPAPI-bound to the Windows user |
| The server administrator reading shared data | No admin endpoint returns habit data. An administrator with operating-system access to the server PC can still read the database file, so this is disclosed in the notice, and only daily summaries are shared. End-to-end encryption is an option (Q4) |
| Camera misuse | Camera permission is granted only to the camera page, through the WebView2 permission handler; others are refused. A visible indicator shows whenever capture is on. The camera page cannot reach the network (strict `connect-src` content policy) |
| Malicious content or IPC in a page | Strict content security policy (the spike had none), local assets only, no remote origins, Tauri capabilities set per window, validated command inputs, developer tools off in release |
| Tampered updates | minisign-signed packages. The signing key is a CI secret, never in the repository |
| Supply chain | Pinned Cargo and npm lockfiles, `cargo-deny`, `npm audit`, a licence register, a software bill of materials, hash-pinned MediaPipe wasm and model files, and only verified upstream sources (packet: the open-posture lookalike) |
| Unexpected outbound traffic | WebView2 SmartScreen and similar Edge services disabled (as in the spike). Release tests capture network traffic to show that only the paired server is contacted (A12) |
| Data at rest | Windows profile permissions locally, `%ProgramData%` permissions on the server, and BitLocker where enabled (not assumed) |

## 10. Observability

| Signal | Where | Personal data |
| --- | --- | --- |
| Structured logs (tracing, JSON lines) | `%LOCALAPPDATA%\HealthierYou\logs`, rotated at 7 days or 10 MB | Event types and counts only; no amounts, notes or aliases |
| Resource self-sampler (reused from the spike) | Hourly summary of private working set and CPU for the app tree | None |
| Crash and webview failures | Rust panic hook; WebView2 process-failed events recreate the page and log it | None |
| Diagnostics export | A zip the user opens and reviews before sending it by hand | The user sees exactly what is included |
| Server health | Admin page: uptime, database size, last backup, devices last seen, versions, error counts | Aliases only |
| Heartbeat | Included in sync: app version and queue length | Operational data (section 6.2) |

There is no third-party telemetry, no crash upload, and nothing goes to the internet.

## 11. Testing strategy

| Level | What | How | Requirements |
| --- | --- | --- | --- |
| Engine unit and property tests | No bursts, one prompt at a time, snooze covers a combined reminder, precedence, Pomodoro and focus transitions, day rollover | `cargo test` with an injected clock, plus property tests | RHY-02, RHY-03, FOC-01, REC-01, CAL-01 |
| Trace replay | Long typing, reading while idle, lock, wake, restart, midnight, time-zone change | Recorded and synthetic traces fed to the engine | RHY-01 |
| Store and review | Totals reconcile after corrections, missing days are distinct from zero, month boundaries, insufficient data | Fixture databases, golden outputs | WAT-01, REV-01, REV-02 |
| Sync and server | Idempotent retries, a category turned off while offline, revocation stopping a second device, deletion receipts, server restart, two-client isolation | Integration tests against a real server binary; fuzzing of API inputs | LAN-01, SHR-01, SHR-02 |
| Load | 3 real plus 10 simulated clients | Simulator on the core crate | PRD section 9 |
| UI and accessibility | Flows, keyboard-only path, Escape closes, reduced motion, chart text alternative | Browser tests against pages with simulated IPC; manual Narrator check | PET-03, PET-04, flows 1 to 9 |
| Windows acceptance | Focus kept, scaling at 100, 125, 150 and 200%, monitor unplugged, presentation mode, Windows 10 and 11 | The owner's PCs with scripted checks (the batch 1 approach) | PET-04, RHY-04 |
| Performance soak | 8-hour idle and 2-hour camera soak on the 8 GB laptop, then the three-day pilot | Self-sampler logs | PRD section 9 |
| Install lifecycle | Offline install, update, rollback, uninstall (registry key gone), SmartScreen on a downloaded and on a shared-folder copy | Scripted on Windows 10 and 11 | PRD section 9 |
| Privacy | Before-and-after network capture per category; confirm camera frames never touch disk or network | Windows packet capture (pktmon) plus file-activity check | SHR-01, CAM-02 |
| V2 quality gate | 5 setups x 2 sessions, raw counts | Desk-test harness (4.5) | CAM-01, CAM-02 |
| Security review | Pairing, authorisation, content policy, dependencies | Independent review by Codex before the pilot | PRD section 9 |

GitHub runners run Windows Server and only build; all acceptance happens on the owner's PCs (packet R19).

## 12. Implementation slices and schedule

These slices start only after the prototype is approved. Effort is in build days (one worker), as a range. **These are estimates, not commitments.** At most two workers run at once after V1-0 merges.

| Slice | Content | Depends on | Build days | Owner test time |
| --- | --- | --- | --- | --- |
| V1-0 | Repository scaffold, CI, `hy-core` event schema, store, migrations, clock and time zone | Approval | 1.5 to 2.5 | None |
| V1-1 | Pet shell, windows, DPI and monitors, tray, autostart, keyboard panel; pet pack format and ten pets with edits | V1-0, visual direction | 2.5 to 4 | 30 min |
| V1-2 | Activity tracker, coaching engine, suppression, focus, Pomodoro, meeting toggle | V1-0 | 2.5 to 3.5 | 30 min |
| V1-3 | Hydration, movement content and illustrations (content review by a Codex agent), recovery day, phone self-report | V1-2 | 2 to 3 | 30 min content check |
| V1-4 | Setup, settings, goals, sharing screens; weekly and monthly review | V1-2, V1-3 | 2 to 3 | 20 min |
| V1-5 | Server, discovery, pairing, policy, sync, deletion, admin page, backup and restore, load simulator | V1-0 | 3 to 4 | 1 h, two PCs and the server PC |
| V1-6 | Installers, updater, uninstall fix, rollback | V1-1, V1-5 | 1.5 to 2.5 | 30 min |
| V1-7 | Soak, network capture, fixes from the security review, performance | All V1 | 1.5 to 3 | Soak runs unattended |
| V2-1 | Camera page lifecycle, capture, worker inference, indicator, off switch, conflict and denied handling | V1-1, V1-2 | 1 to 2 | 20 min |
| V2-2 | Calibration, features, drift detector, cue, recalibration prompt | V2-1 | 2 to 3 | 30 min |
| V2-3 | Desk-test harness, labelled sessions, raw-count report | V2-2 | 1 to 2 | About 6 h across 5 setups (P) |
| V2-4 | Meditation pause | V1-2 | 0.5 to 1 | 10 min |
| V2-5 | 2-hour camera soak, Windows 10 and second-webcam checks, fixes | V2-1 to V2-4 | 1 to 2 | 1 h |
| **Total** | | | **V1: 16.5 to 25.5; V2: 5.5 to 10** | |

On credits: packet revision 2 put V1 at $54 to $108 and V2 at $16 to $30. Batch 1 came in above its own estimate ($21.2 against $11 to $19), so plan on the upper half. The budget is re-planned at the review against the ledger. Cloud credits expire on 5 Nov, and later work uses regular plan usage (Q8).

**Schedule estimate from 22 Oct (Asia/Kolkata). It assumes each gate closes the first time and the owner is available to test.**

| Dates (2026) | Step | Gate |
| --- | --- | --- |
| 22 to 24 Oct | Architecture review and revision | Codex review closed; owner accepts |
| 24 to 28 Oct | V1 and V2 wireframes and clickable prototype (V3 flows dropped from this gate) | Codex design review |
| 29 to 30 Oct | Owner's prototype review | Explicit approval recorded |
| 30 Oct to 12–15 Nov | V1 build (V1-0 to V1-7) | One evidence packet per slice |
| 10 to 18 Nov | V1 independent testing, security review, UAT | UAT signed |
| **16 to 21 Nov** | **V1 pilot (3 working days, 3 people)** | Owner's go or no-go |
| 6 to 20 Nov | V2 build (one worker, starts once V1-2 merges) | Slice packets |
| 19 to 26 Nov | V2 desk test and UAT | Raw counts meet the gate, or a decision is needed |
| **25 Nov to 2 Dec** | **V2 pilot** | Owner's go or no-go |

Risks to these dates: owner and colleague availability around Diwali (about 8 Nov) (Q5); the server PC not being ready for V1-5 (Q1); rework if the prototype review fails; the owner's test turnaround time. **The original 5 Nov target for V1, V2 and V3 is not committed.**

## 13. V3 extension points (interfaces only; nothing is built)

| Interface | Purpose in V3 | What V1 and V2 do now |
| --- | --- | --- |
| `SyncTarget` (base URL, trust, token) | The hosted service speaks the same `/v1` protocol as the LAN server | A single LAN implementation; the protocol is versioned from the start |
| Event `source = imported`, `source_ref`, `observed_at` against `received_at`, freshness | Wearable imports with "reported by" provenance and staleness | The fields exist in the schema; the value is rejected until V3 |
| Sharing category registry | Adds `wearable_summary` and a separate community display payload | Fixed list; adding a category is a data change plus a consent-notice version |
| `ProfilePublisher` (pet id, variant, display name, alias) | Same avatar on a hosted profile | Not implemented |
| `HabitCreditCalculator` (bounded daily credit per eligible habit) | Feeds bounded community scores | Daily summaries already hold the inputs; no score is computed or shown |
| Membership invariant | At most one active group, enforced on the server (unique partial index) | Not implemented |
| Account model: user → organisation | Hosted multi-office users | The LAN has one implicit organisation |

## 14. Unverified assumptions

| # | Assumption | Risk if wrong | How and when it is checked |
| --- | --- | --- | --- |
| A1 | A suitable server PC exists, stays on in office hours, shares the employees' subnet (no guest or isolated Wi-Fi), and an administrator can install one service and one firewall rule | No sync, pairing or backup; V1-5 is blocked | Owner answers Q1; install trial at the start of V1-5 |
| A2 | Memory and CPU stay flat over hours: idle at most 150 MB, camera at most 350 MB (301 MB peak over 10 minutes is 86% of the budget) | Budget breach or leak during the pilot | 8-hour idle and 2-hour camera soak (V1-7, V2-5); three-day pilot |
| A3 | WebView2 capture works on Windows 10 and on USB or second webcams (the only Windows 10 PC's webcam was not seen by Windows) | V2 unusable on some PCs | Test every pilot PC's webcam in V2-1 and V2-5; record the models |
| A4 | Drift from calibration meets the V2 gate (70% coverage, 80% detection, at most 1 false cue per 30 minutes) | V2 fails acceptance; threshold or feature change needs the owner's approval | Desk test V2-3 with raw counts; Face Landmarker added only if forward-head detection misses |
| A5 | Unsigned installers run on office PCs from a shared folder or download (SmartScreen, Smart App Control, antivirus, policy) | Install blocked or alarming warnings | Scripted I4 test in V1-6 on Windows 10 and 11; a signing decision follows if needed |
| A6 | Pet stays crisp and clickable at 150% and 200% scaling and after a monitor is unplugged (only 100% and 125% tested) | PET-04 fails | Windows acceptance in V1-1 |
| A7 | Idle memory stays within budget with real SVG art, Preact and the product pages | Memory over 150 MB | Re-run the batch 1 P1 script after V1-1 and V1-4 |
| A8 | The pet panel can take focus from the hotkey and hand it back on Escape | Keyboard path fails PET-04 | V1-1 manual test |
| A9 | Pose inference in a Web Worker in a background camera page keeps 2 to 3 fps without throttling | Detection latency misses the 30 s cue window | V2-1 measurement; fall back to a visible camera indicator page |
| A10 | The Windows notification-state API reports presentation and full-screen mode (and Do Not Disturb on Windows 11) reliably | Prompts during presentations | 20 scripted trials in V1-2; the manual toggle is always available |
| A11 | The camera and microphone in-use registry state is a usable meeting hint | False hints | Off by default; 20 trials with false-positive cases before enabling (packet D2) |
| A12 | With the chosen settings, WebView2 makes no outbound connections | A privacy claim would be false | Network capture in V1-7 |
| A13 | The DPDP interpretation (consent basis, timing) and the possible application of the IT Act sensitive data rules | Notice or consent rework | The firm confirms before the pilot (Q3) |
| A14 | The pairing protocol is sound | LAN impersonation | Codex security review in V1-5; SPAKE2 as the alternative |
| A15 | Offices allow per-user installs into `%LOCALAPPDATA%` and per-user autostart | Admin install needed per PC | First pilot PC install in V1-6 |
| A16 | Updates of about 180 MB or more (the bundled runtime) over the LAN are acceptable | Slow updates | Measure in V1-6; ship the runtime only when it changes |
| A17 | All office PCs are x64 | An ARM PC would need another build | Owner inventory (Q1) |
| A18 | Ten original pets can be drawn within the prototype and V1-1 effort | Schedule slips | Visual direction chosen at the prototype gate |

## 15. Open questions for the owner (one batch, short answers)

| # | Question | Needed by |
| --- | --- | --- |
| Q1 | Which PC will be the server? Its processor, RAM, Windows edition; is it on during office hours; can an administrator install a service once; are all office PCs x64 and on the same network? | 29 Oct |
| Q2 | Is "the employee's own backup and continuity, no manager view" the right stated purpose for the server? Does the firm want anything else from it? | 29 Oct |
| Q3 | Who at the firm is the named contact for privacy questions and deletion requests, and will the firm confirm the consent wording against the DPDP Act? | Before the pilot |
| Q4 | Is it acceptable that whoever administers the server PC could technically read shared summaries (disclosed in the notice)? Or do you want end-to-end encryption (about +2 to 3 build days, plus a recovery key per user)? | 29 Oct |
| Q5 | Office working days (Monday to Saturday?), Diwali closure dates, and when the 3 pilot users and about 6 hours of colleagues' webcam desk sessions are available | 29 Oct |
| Q6 | For the pilot, is an unsigned installer from a LAN shared folder still acceptable? | Before V1-6 |
| Q7 | When uninstalling, should "delete my local data" be ticked by default? (Proposed: unticked, with the location shown) | Before V1-6 |
| Q8 | Cloud credits expire on 5 Nov; build work after that runs on regular plan usage. Is that acceptable? | 22 Oct review |

## Appendix: requirement traceability (V1 and V2)

| Requirement | Component (section) | Slice |
| --- | --- | --- |
| PET-01, PET-02 | Pet library (4.4) | V1-1 |
| PET-03, PET-04 | Pet shell, pet panel (4.1, 4.2) | V1-1 |
| RHY-01 | Activity tracker, session model (4.3) | V1-2 |
| RHY-02, RHY-03 | Coaching engine (4.3) | V1-2 |
| RHY-04 | Suppression (4.2) | V1-2 |
| WAT-01, WAT-02 | Hydration, store corrections (4.2, 6.1) | V1-3 |
| MOV-01, MOV-02 | Movement programme (4.2) | V1-3 |
| REC-01 | Recovery day (4.2) | V1-3 |
| FOC-01 | Pomodoro, engine precedence (4.2, 4.3) | V1-2 |
| DIS-01 | Phone self-report (4.2) | V1-3 |
| REV-01, REV-02 | Review engine, screens (4.2) | V1-4 |
| SHR-01, SHR-02 | Sharing policy, sync, deletion (5, 6.2) | V1-4, V1-5 |
| LAN-01 | Server, pairing, authorisation (5, 9) | V1-5 |
| PRD section 9 (install, update, backup, rollback, performance) | Sections 8, 10, 11 | V1-5, V1-6, V1-7 |
| CAM-01, CAM-02, V2 gate | V2 posture (4.5) | V2-1 to V2-3, V2-5 |
| CAL-01 | Meditation pause (4.6) | V2-4 |
| DIS-02 | Not in V1 or V2 scope; stays in the backlog (packet) | None |
| CON-01, CON-02, COM-01, COM-02 | V3 extension points only (13) | Deferred |
