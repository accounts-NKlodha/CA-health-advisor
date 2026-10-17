# HealthierYou — Discovery & Product Requirements

Version 0.1 • 18 October 2026 • Draft for owner approval

**No product development is authorised by this document.** Rohit approves this PRD and later Claude's wireframes/clickable prototype. Claude owns implementation and fixes; Codex owns planning and independent review. The date target is **5 November 2026 for V1, V2 and V3**. Passing evidence, not a date or a mockup, determines readiness.

## 1. Product decision

Build a small, editable desktop pet that helps CA-firm employees maintain healthier workday habits while using Excel, Tally, PDFs and portals. Ordinary interactions happen through the pet, not an open dashboard. Screens exist for setup/settings, explicit actions requiring more space, and weekly/monthly reflection.

One office server provides LAN services. Each employee gets a self-contained Windows companion. A separate hosted deployment supports advanced users and wearable integration. The same behaviour definitions should be reused across deployments; Claude chooses the implementation after feasibility. A hosted browser page is not a substitute for the Windows pet.

The application helps people take breaks, log water against their own goal, complete short movement routines and understand personal progress. It does not measure clinical health or employee productivity.

## 2. Confirmed constraints and proposed defaults

### User decisions

- All three versions targeted before 5 November; no universal wearable support in this release.
- Windows 10/11 clients; slowest machine has 4 GB DDR3 RAM and 256 GB storage. Processor and server specifications remain unknown.
- One self-contained server installation and one self-contained companion installation per PC are allowed. No mandatory separately installed developer tools, free break apps or other end-user software in V1. Installer-managed bundled prerequisites must be disclosed and tested.
- V1 has no webcam/hardware dependency; V2 adds webcam only; V3 adds selected wearable/phone routes for hosted users.
- Initial library of ten editable pets; all reminder delivery uses the pet experience.
- Water target is user-selected. Monday–Saturday exercise coverage, weekly reflections and monthly charts are required.
- Each user decides which optional data reaches the server.
- Codex does not implement the product; Claude develops and fixes it. User can test locally.
- Budget: $200 Claude cloud-session credits. Hosting, certificates and store accounts are not assumed included or approved for purchase.
- No competitor research programme. Security testing remains part of acceptance.

### Planner defaults proposed for approval

- English first, with readable simple labels and room for later translations; no inferred Tamil/Hindi requirement.
- Individual history visible only to the individual. Administrator sees service/device health, pairing and update status, not personal habit charts. A manager health dashboard is out of scope unless explicitly requested later.
- No forced breaks, input blocking, always-on sound, streak punishment or rankings. Users may pause, snooze, dismiss or disable a category.
- Sunday is optional/recovery; Monday–Saturday is the scheduled movement sequence. Missed days do not create a catch-up debt.
- Core reminders and personal history keep working if the LAN/server/internet fails. Data sharing is independently controlled and may be off.
- Raw camera frames and raw keyboard content are not stored or sent. Webcam functionality sends only permitted derived summaries, if any.
- User-supplied public repository: accounts-NKlodha/CA-health-advisor. Publish project planning/source there; keep private conversation archives, credentials and actual employee records out. Code licence choice remains a release decision.

## 3. Version contract

| Version | Must be demonstrated | Explicit boundaries |
|---|---|---|
| V1 Workday Pet | Installer, LAN pairing, ten selectable/customisable pets, combined reminders, water logging, six-day exercise programme, focus/pause, weekly/monthly review, sharing controls, offline operation | No camera, phone, watch, cloud AI or third-party runtime manually installed by employee |
| V2 Webcam Pet | V1 plus personal calibration, confidence-aware presence/posture check, sustained-change cue, camera indicator/off switch and graceful unavailable-camera behaviour | No guaranteed angle accuracy, drinking volume, emotion diagnosis or exercise verification; phone-use detection is an optional experiment, not a substitute for core V2 |
| V3 Connected Pet | Hosted deployment, account/device pairing, a documented and physically tested integration for each of the three target watches, visible source/freshness, disconnect/revoke, pet review using imported data | Only named devices; exact per-device records and delivery route must be settled by feasibility. No claim that every watch accepts our app or provides live data. Public store approval is separately tracked |

V3 target combinations, as reported by the user:

1. Apple Watch Series 8 + iPhone 13.
2. Fossil, likely Gen 5 + Samsung Galaxy S24 FE.
3. Samsung Galaxy Fit, possibly Fit3 (user uncertain) + Samsung Galaxy A25.

**V3 scope is an unresolved feasibility gate, not permission to quietly drop a device.** For each device, the proposed minimum is a verified daily activity/step summary reaching the user's hosted account and appearing in their personal review. Reminder delivery to the watch is an additional tested capability. If this minimum is impossible for a device, escalate with evidence and a concrete alternative; the user must approve the changed acceptance scope. Simulator data, manual entry and screenshots of a vendor app do not pass an automated wearable integration criterion. A clearly labelled file import can be offered as an alternative but does not automatically count as the requested integration.

## 4. Users and core journeys

### First use

Install companion → choose/connect office server or hosted account → pair securely → select pet/name/size → set work hours, water goal and serving sizes → choose reminder intensity → choose sharing categories → preview a reminder. A user declining optional sharing can still use personal local features. Standard user install versus administrator-required install must be demonstrated on an office PC.

### Ordinary workday

The pet rests at a user-selected screen edge. It does not steal keyboard focus or cover critical application controls. At a suitable moment it offers one compact action such as a short movement break plus water. The user can mark completion, log water, snooze or enter focus mode. Starting a break pauses the current prompt; cancelling or returning does not create duplicate reminders.

### Guided movement

The pet introduces the day's short routine with an expandable illustration/instruction bubble. User chooses seated/standing adaptation if provided, completes or skips steps, and confirms completion. In V1 the record is self-reported. A screen reader and reduced-motion mode must expose the same instructions. No movement is imposed because a camera or watch inferred a condition.

### Weekly reflection

At the user's chosen week-end time, the pet offers a review; opening it is optional. Show successes, missed planned opportunities, data coverage, and at most three achievable next-week suggestions. If skipped, offer it on the next workday without repeated nagging. Month view shows consistent metrics over time. No-data periods look different from a measured zero.

### Server and phone outages

Pet/reminders/manual logs continue locally. A small settings indicator explains offline status. Reconnection syncs only currently allowed categories, with duplicate prevention. A phone's last-sync time is shown; yesterday's steps never appear as today's live data. A locked/sleeping PC does not queue a burst of reminders on wake.

## 5. Functional requirements

| ID | Requirement | Acceptance evidence |
|---|---|---|
| PET-01 | Ten original or properly licensed selectable pets, one consistent interaction model | Demonstrate all ten; asset provenance recorded; no paid asset dependency hidden in package |
| PET-02 | Edit display name, scale, supported colour variants, placement, motion and tone; reset defaults | Save/restart preserves edits; all characters support the same core actions |
| PET-03 | Idle, cue, exercise, waiting, celebration, snoozed, focus and disconnected states | Prototype and implementation show each; no mandatory sound or keyboard-focus capture |
| PET-04 | Pet remains usable across display scaling, monitor changes and keyboard navigation | Tested at 100/150/200% scaling; reposition after disconnected monitor; escape closes expanded bubble |
| RHY-01 | Track computer activity/session time, lock, sleep and user-defined work hours | Recorded test traces include long typing, reading while idle, lock, wake, restart and midnight; UI never equates input idle with proven movement |
| RHY-02 | Combine due needs into one reminder with explicit reason | Simultaneous water/movement/eye needs yield one prompt; a snooze does not create three independent reminders |
| RHY-03 | Focus, pause, snooze, dismiss and category toggles | Immediate suppression; deterministic restart/resume behaviour; no prompt burst |
| RHY-04 | Meeting/presentation suppression with manual fallback | Demonstrate supported detections and limitations; no claim of knowing every meeting or portal submission |
| WAT-01 | Personal water goal and serving sizes; quick log, custom amount, undo and correction | Totals reconcile after edits, day changes and sync retries; exceeding target does not trigger encouragement to keep drinking |
| WAT-02 | Pace reminders within selected work hours | Reminders use chosen goal and logged amount; no automatic medical/seasonal target assignment |
| MOV-01 | Monday–Saturday short routines with broad body-region coverage | Content matrix, provenance/review, illustrated steps, alternatives and skip flow; Sunday optional |
| MOV-02 | Exercise session completion and interruption handling | Self-reported records labelled; partial completion, reschedule and skip do not count as full completion |
| REV-01 | Weekly successes, missed plans and next-week suggestions | Each statement reproducible from recorded events; insufficient data produces qualified/no conclusion |
| REV-02 | Month-by-month progress chart | Month boundaries/time zones tested; coverage visible; missing days and zero distinct; chart has text alternative |
| DIS-01 | Optional self-reported phone interruptions in V1 | One action creates an editable event; review says self-reported and allows work-related correction |
| DIS-02 | Automatic phone-use idea assessed separately | V2/V3 experiments report false positives/misses and limits; unavailable feature cannot imply zero distractions |
| SHR-01 | Select optional sharing by data category; preview fields and recipient | Before/after network test proves only allowed categories leave device; defaults off for optional personal data |
| SHR-02 | Stop future sharing and separately delete previously shared data | Toggle discards unsent optional queue for that category; remote deletion has acknowledgement/retry/status; revoke on one device does not silently permit a second device |
| LAN-01 | Office server discovery/manual address and secure device pairing | Two concurrent clients cannot see each other's personal records; server disconnect/restart/reconnect tested |
| CAM-01 | V2 calibration with confidence/no-reading state | Test actual desks, lighting, glasses, camera absence/occlusion and changed camera position; request recalibration when necessary |
| CAM-02 | Sustained baseline-relative posture cue and user-controlled camera | No per-frame nagging; visible camera state; off releases capture promptly; V1 features continue during camera conflict |
| CON-01 | V3 source-linked imports for each selected watch | Physical device proof: source record, phone sync, hosted ingestion, pet review; timestamps and retry/deduplication evidence |
| CON-02 | Hosted account/device isolation, revoke and stale-state handling | Separate users cannot access each other's events; revoked token stops sync; deleted account behaviour documented |

## 6. Reminder rules — proposed product defaults, not health prescriptions

Initial Balanced configuration: movement cue after 45 minutes of accumulated active work; optional brief eye-rest cue at 20 minutes; five-minute full-reminder cooldown; snooze ten minutes; focus 30/60/90 minutes. All intervals are configurable. These numbers are UI starting values to test for usefulness, not medical requirements.

Priority order: user pause/lock/sleep/off-hours → focus/presentation suppression → active exercise/break → combined due reminder. An eye cue due shortly before a movement break is folded into that break. Hydration joins an eligible reminder when behind the user's selected pacing. One nonurgent reminder is visible at a time. Repeated dismissals lead to a gentle offer to reduce intensity, never escalation or guilt.

Do not claim detection of file saves, GST submissions or natural task completion in V1 without an explicit supported signal and tested implementation. App switches or input pauses are imperfect proxies. No screen scraping, portal integration, keystroke contents or window-title collection is required by this PRD.

## 7. Pet library and editing boundary

Proposed starting characters: walking turtle, stretching cat, water-drop buddy, calm panda, cheerful dog, active rabbit, steady elephant, resting koala, balanced flamingo and helpful owl. Names/art are provisional for Claude's design exploration; user approves the visual family.

Ten pets does not mean ten different software behaviours. Reuse state/action definitions while varying artwork. V1 editing means built-in appearance/name/size/tone controls, not an animation-authoring suite, AI image generation or arbitrary executable plugins. Claude should propose an asset format that makes adding future pets inexpensive. No artwork is being created by Codex in this stage.

## 8. Data, insights and sharing semantics

Minimum conceptual events: session/activity state, reminder offered, action/snooze/dismiss, manual water entry/correction, exercise step/session outcome, user-reported phone interruption, camera-derived observation, wearable import and sharing-policy change. Each needs an ID, event time, time zone, source, confidence/measurement status and schema version. Manual, inferred and imported values must remain distinguishable.

Proposed review metrics:

- Break follow-through = explicitly completed breaks / offered eligible breaks, with counts and coverage. Do not describe this as objectively measured physical activity.
- Longest observed work block = session estimate under a documented idle/reset rule; not proven uninterrupted sitting.
- Hydration = logged volume versus user-set target; no claim to measure total fluid intake.
- Exercise consistency = self-reported completed sessions / scheduled sessions on enabled workdays, with skipped/partial counts.
- Phone interruptions = user-reported events unless an optional verified integration explicitly provides another metric.
- V2 posture observations = accepted observations relative to calibration with camera-on coverage; no medical score.
- V3 steps/activity = source-specific imported records, timestamps and freshness. Never blindly add overlapping phone/watch totals.

Weekly suggestions are deterministic, explainable and limited to observed habits/preferences; an LLM is not required. Baseline comparisons require comparable coverage and show the chosen date range. Date boundaries use the individual's selected zone, initially Asia/Kolkata; changes must not duplicate events. Device clock differences and backfilled records must be handled.

Optional sharing groups: hydration; reminder/exercise outcomes; work-session summaries; phone-interruption summaries; camera-derived summaries; wearable summaries. Minimum operational traffic (device ID, pairing, application version, service reachability) is disclosed separately and cannot be described as optional health sharing. No secondary analytics upload is implied.

Proposed retention: local detailed events 90 days; personal monthly summaries 24 months; server keeps only opted-in categories with the same or shorter retention. Retention is a design default, configurable downward. Backups and remote deletion semantics must be designed before deployment. User-facing deletion must distinguish immediate active-store deletion from backup expiry.

## 9. Performance, reliability and security acceptance

These are proposed release targets to verify on the real slowest PC, not measured achievements:

- V1 companion idle working set at most 150 MB; average idle CPU at most 2% of total machine CPU over a 30-minute quiet period; no sustained idle spike over 5%. Record processor, sampling method and conditions.
- V2 combined companion/inference working set at most 350 MB during check-ins; aim at most 10% total CPU averaged during capture. If missed, use lower duty cycle/model adjustments; report results and seek any target change rather than silently ship.
- Settings/pet action responds within 500 ms on test PC; first usable companion launch within 10 seconds after login stabilises.
- No application crash or unbounded memory growth during three working-day pilot sessions. Performance must be measured alongside ordinary office applications.
- Three pilot participants are confirmed. Run with three real clients and ten simulated/concurrent clients for the initial LAN load test; total rollout size and server sizing remain deployment inputs, not assumed proven capacity.
- Per-user authentication/authorisation, safe pairing, secrets outside repository, transport protection appropriate to LAN/hosted route, validated imports, dependency/licence review, and no cross-account reads/writes.
- Server backup and restore demonstrated; previous compatible client/server release available for rollback; migrations tested on sample data. Release logs must exclude unnecessary personal payloads.
- Installer supports install/update/uninstall with clear personal-data handling. Do not promise unsigned installers will be warning-free or purchase signing certificates without approval.

## 10. Wireframe/prototype contract — Claude's next design deliverable

### V2 experimental quality gate proposed for feasibility

Before runtime implementation is accepted, Claude must define a repeatable labelled desk test and report raw counts, not a single unsupported accuracy percentage. Proposed minimum: five representative camera/desk setups, two sessions per setup, including comfortable reading, typing, staged sustained forward/lateral drift, looking at a second display, absence and occlusion. Labels describe observed movement relative to calibration, not clinical posture quality. Record manual labels and aggregate results; no stored raw video is required.

- Supported-condition coverage: a usable reading in at least 70% of planned check intervals in ordinary tested lighting; report each setup separately and show low confidence outside supported conditions.
- Sustained-change detection: at least 80% of the deliberately staged 60-second drift episodes detected, with cue within 30 seconds after the sustained threshold; at least ten staged episodes per setup. Deduplicate one episode into one event.
- False-cue burden: no more than one incorrect posture cue per 30 minutes of labelled comfortable desk work per setup. Use a 30-minute posture-cue cooldown by default; report detector false events separately so cooldown cannot conceal poor detection.
- Camera off/denied/occluded: explicit unavailable/no-reading state, no posture score inferred from missing data, normal V1 continuation. Capture ends within two seconds of user off action where platform access permits; failures are release blockers.
- These are small-pilot engineering thresholds, not medical validation or general population performance. Claude may propose a better method/threshold before the prototype gate; record and approve changes before using them to declare success.

Movement-content acceptance owner: Claude prepares the sourced six-day matrix and original instructions; Codex's independent content-review agent checks source fidelity, body-region coverage, adaptations and stop/skip wording; user approves the experience. This is evidence/content review, not clinical certification. Unsuitable or unsupported routines are replaced before release. Any treatment-specific routine requires appropriately qualified review and is outside the initial general-habit scope.

Hosting and Apple build/distribution access are owned by the user for account authority and by Claude for the technical route. They must be resolved at the Oct 20 feasibility checkpoint; no new purchases are authorised by this PRD. Supported Windows builds, CPU inventory and server capacity must be recorded by Oct 22 for a credible test plan. Unknowns remain open rather than silently assumed satisfied.

### Required prototype flows

Provide a linked flow map and clickable prototype, not disconnected attractive images. Clearly label simulated data and unsupported integrations. At minimum include:

1. Installation/pairing: LAN, hosted, server unavailable, invalid pairing.
2. Initial setup: water goal/servings, workweek, reminder controls and ten-pet selector.
3. Desktop context: pet over a simulated spreadsheet, compact and expanded states, position/size editing.
4. Combined reminder: completion, water, snooze, dismiss and focus transitions.
5. Six-day exercise flow: instructions, adaptation, partial completion and skip.
6. Settings/data sharing: category toggles, payload preview, pending sync, revoke and delete.
7. Weekly reflection: strong week, mixed week, missing data, first week; realistic next-week actions.
8. Monthly progress: coverage, baseline, empty state and accessible chart alternative.
9. V2 calibration/check-in, camera conflict, low confidence and immediate off.
10. V3 exact three-device connection paths, pending permission, unsupported model, last sync and revocation.
11. Phone interruption self-report/correction, and visibly separate experimental states.
12. Administrator service-status/pairing/update view without individual health analytics.

Review rubric (100 points): core journey completeness 25; pet usability/non-interruption 20; settings/review clarity 15; failure/offline/accessibility states 15; version/device honesty 15; lightweight visual consistency 10. Proposed pass: at least 85/100, no missing critical journey, no blocking usability finding, and explicit user approval. A numerical pass alone does not authorise development.

## 11. Discovery exit and outstanding gates

This PRD can be reviewed now. These unresolved items do not justify inventing capabilities:

- Exact Fossil/Samsung models, phone/watch OS versions and permissions; see wearable research.
- Availability of a genuine Claude cloud coding environment with repository write/execution access; ordinary project file sync is insufficient proof.
- User-supplied public repository is accessible through the connector; Claude's access/write path remains to verify independently.
- Server RAM/CPU/OS, total rollout size and prompt test feedback; three pilot participants confirmed, ten-user simulated load is a proposed test target.
- Apple build/test distribution account and platform requirements, hosting credentials/budget and eventual store distribution approvals.
- Evidence-based feasibility of each V3 device and V2 camera quality. No store approval or health outcome guaranteed by November 5.

Discovery deliverables are this PRD, supporting findings, Claude design brief, gate/checkpoint protocol and a proposed schedule. Detailed implementation planning follows PRD approval. Product code follows approved prototype only.
