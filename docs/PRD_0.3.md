> **Approved baseline.** PRD revision 0.3, approved by the owner on 18 October 2026 and published here on 20 October 2026 with the owner's name removed (owner's instruction). It supersedes `docs/PRD.md` (revision 0.1). Later owner decisions recorded in `docs/Decisions.md` (for example DEC16 V1/V2-first sequence and DEC17 8 GB minimum PC) take precedence where they differ. Source package SHA-256 before redaction: 13224412c460fc0f260ebf8c84ef7e6e0715e263d231217ebad6c4d2a9fd962e.

# HealthierYou — Approved PRD and Claude Design Package
PRD 0.3 and Claude design brief approved by user on 18 October 2026. Prototype is not yet produced or approved. No product development authorised. Sound/colour deferred to V4, outside November 5 V1–V3 target. This file supersedes the older PRD 0.1 currently published on GitHub.

---
Source: docs/PRD.md

# HealthierYou — Discovery & Product Requirements

Version 0.3 • Approved by owner 18 October 2026; prototype gate pending

Scope extension: [Recovery, focus and community addendum](Recovery_Focus_Community_Addendum.md) specifies REC-01, FOC-01, CAL-01 and COM-01/02, including evidence boundaries and proposed defaults. Read it as part of this draft. Prototype approval remains pending.

Latest version decision: sound/headphone relaxation and colour sessions are V4 backlog, outside the November 5 V1–V3 target. V4 date and budget are uncommitted. Simple optional meditation remains proposed for V2. Normal pet colour customisation is unchanged.

**No product development is authorised by this document.** The owner approves this PRD and later Claude's wireframes/clickable prototype. Claude owns implementation and fixes; Codex owns planning and independent review. The date target is **5 November 2026 for V1, V2 and V3**. Passing evidence, not a date or a mockup, determines readiness.

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
- No forced breaks, input blocking, always-on sound or streak punishment. Users may pause, snooze, dismiss or disable a category. Optional hosted-community habit rankings are requested in revision 0.3; employer performance rankings remain excluded.
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

Revision 0.3 also requires the addendum's recovery-day, on-demand Pomodoro, simple V2 meditation, hosted-avatar and one-group/ranking flows. Proposed scoring rules and spirituality-related wording require explicit design review. No chakra alignment or clinical colour/sound therapy claim is an acceptance outcome.

## 11. Discovery exit and outstanding gates

This PRD can be reviewed now. These unresolved items do not justify inventing capabilities:

- Exact Fossil/Samsung models, phone/watch OS versions and permissions; see wearable research.
- Availability of a genuine Claude cloud coding environment with repository write/execution access; ordinary project file sync is insufficient proof.
- User-supplied public repository is accessible through the connector; Claude's access/write path remains to verify independently.
- Server RAM/CPU/OS, total rollout size and prompt test feedback; three pilot participants confirmed, ten-user simulated load is a proposed test target.
- Apple build/test distribution account and platform requirements, hosting credentials/budget and eventual store distribution approvals.
- Evidence-based feasibility of each V3 device and V2 camera quality. No store approval or health outcome guaranteed by November 5.

Discovery deliverables are this PRD, supporting findings, Claude design brief, gate/checkpoint protocol and a proposed schedule. Detailed implementation planning follows PRD approval. Product code follows approved prototype only.




---
Source: docs/Recovery_Focus_Community_Addendum.md

# Recovery, focus and community — proposed additions

18 October 2026 • Draft scope extension for PRD 0.3. No product development authorised.

The user requested advice and additions: a recovery day after deadline-heavy work, once-daily headphone meditation/sound in V2, colour/chakra ideas, on-demand Pomodoro, and hosted avatars/rankings with at most one joined group. Requested product behaviours are distinguished below from planner defaults and unproven health claims. Latest user decision: move sound and colour experiences to V4. A simple guided meditation pause remains proposed for V2; specialised audio, headphone playback and colour sessions are deferred. The existing PRD/prototype approval gates remain unchanged.

## 1. Evidence and positioning

Priority remains achievable movement, screen breaks, appropriate hydration, rest and optional mental recovery. Chakra themes can be an opt-in spiritual/reflection experience, not a measurable health outcome or the core clinical rationale. Cleveland Clinic describes chakras as a traditional framework not recognised by Western science, while noting possible personal meaning in associated reflection practices. No chakra score, energy scan, alignment diagnosis or promised healing. [Cleveland Clinic](https://health.clevelandclinic.org/chakras).

Meditation may support well-being, but effects vary and not everyone finds it comfortable. Provide stop/skip, eyes-open and non-breath-focused alternatives. [NCCIH meditation overview](https://www.nccih.nih.gov/health/meditation-and-mindfulness-effectiveness-and-safety).

Five minutes of gentle breathing is a reasonable starting design informed by NHS guidance; it is not a proven optimal meditation dose for CA employees. No forceful breathing or breath holds in the starter session. [NHS breathing exercise](https://www.nhs.uk/mental-health/self-help/guides-tools-and-activities/breathing-exercises-for-stress/).

Recorded nature sounds/music are relaxation audio. Music therapy is a professional therapeutic practice; a playback feature does not establish delivery of that therapy. Excessive volume can damage hearing. [NCCIH music and health](https://www.nccih.nih.gov/health/music-and-health-what-you-need-to-know).

Colour palettes are preference/accessibility features and optional visualisation aids. Do not promise a colour heals an organ, corrects chakras or treats stress. Screen colour changes are not equivalent to clinical light treatments, whose device/dose/indication matter. [Harvard light-therapy overview](https://www.health.harvard.edu/blog/light-therapy-not-just-for-seasonal-depression-202210282840).

Short, frequent changes away from screen work are generally preferable to relying only on long infrequent breaks. This supports keeping small pauses even if a user selects a longer walk. HSE is cited for ergonomics guidance, not as an assertion of Indian legal duties. [HSE work routine](https://www.hse.gov.uk/msd/dse/work-routine.htm).

Hydration needs vary. Preserve the individual's selected goal/restrictions rather than automatically increasing intake after a busy day. [NHS hydration guidance](https://www.nhs.uk/live-well/eat-well/food-guidelines-and-food-labels/water-drinks-nutrition/).

These are source-informed product proposals, not an expert-approved treatment programme. Claude authors scripts; independent content review checks source fidelity and safe wording. Qualified review is needed before any claim of a therapeutic service; no such service is in the current scope.

## 2. V1: deadline mode and recovery-day invitation

User can mark a deadline-heavy day. The pet may also offer a neutral check-in based on long work sessions, but the user confirms workload; do not infer exhaustion or stress from a face, watch or hours alone.

After confirmation, ask: "Would you like to plan a gentler recovery day?" Offer tomorrow, choose a date, later or no. Store one pending plan; reschedule/cancel freely. This is a personal routine, not approval of employment leave or a promise to reverse overwork. Do not suppress all useful breaks on deadline day or encourage overtime because recovery is planned.

Recovery-day plan proposed for prototype:

- More visible opportunities for short movement and screen-free breaks, with normal user control.
- An optional comfortable 10–15-minute walk at a chosen time; a user may choose a 15-minute walk at 90-minute intervals, but it is not mandatory, medically prescribed or a replacement for shorter pauses.
- Normal personal hydration pacing, no catch-up litres or increased target without user edit.
- Lunch away from the screen when practical, a brief calming pause, and a chosen work-shutdown time.
- Reuse the day's movement programme; do not add exercise debt for missed sessions.
- One optional end-of-day reflection: "Did this routine feel helpful?" The response stays personal unless explicitly shared.

Acceptance REC-01: choosing, rejecting, rescheduling and cancelling the date work; timezone/day rollover and missed-day handling tested; no automatic leave request, target increase, duplicate routine or claim of compensated health damage.

## 3. V1: user-started Pomodoro with pet-directed breaks

Off by default and only starts after the user presses Start. Proposed defaults: 25 minutes work / 5 minutes break; optional 50/10 and custom durations. These are focus presets, not a universal health prescription. This is distinct from existing 30/60/90-minute quiet-focus mode.

Pet shows compact time remaining. Pause/resume/end are always available. At the break, suggest a small sequence selected from current preferences: look away/blink naturally, stand or move comfortably, and water only if relevant to the user's goal. A one-minute screen-free eye-rest option is acceptable; do not prescribe eye-rolling routines or claim to improve eyesight. The remaining break should permit leaving the screen.

No automatic restart unless separately enabled by the user; show this setting clearly. Normal reminders are coalesced into the scheduled break where appropriate, not duplicated. A manual Pomodoro request is allowed during quiet focus after showing that it replaces/pauses that mode; lock/sleep/meeting handling must be explicit. Do not launch meditation audio when a timer ends.

Proposed timer precedence: an explicit Start may run outside normal work hours without changing those hours. If all coaching is paused, offer to resume coaching for this session; do not silently override the pause. Quiet focus is replaced only after confirmation. Lock/sleep pauses the timer and requires explicit resume; detected meetings/presentations pause prompts and the timer until user resumes. No catch-up cycles or automatic playback on return. Category-disabled activities stay excluded from break suggestions.

Acceptance FOC-01: disabled on first use; start/pause/resume/end/restart preference tested; no overlapping timers or double prompts after lock/restart; water advice cannot exceed/override the user's goal; declining a break does not become completion.

## 4. V2: simple daily meditation pause; V4: sound and colour experiences

One optional daily invitation at the user's chosen time. Starter duration five minutes, with two- and ten-minute alternatives. Increasing time is a preference, not progression required for a score.

V2 prototype: a simple text-guided meditation/grounding timer with no special sound or colour requirements. Acceptance CAL-01 covers optional invitation, chosen duration, pause/stop, readable instructions and no replay backlog. The content remains subject to review.

The following richer experience is V4 backlog only, not required for the November 5 release or current prototype. V4 proposed flow: settle comfortably, gentle natural breathing or external-sound awareness, brief body-awareness/grounding guidance, and a calm return to work. Exact script and timing are content-review deliverables. Offer silence, voice-only, nature sounds or gentle music. Use a secular default; a clearly labelled chakra-inspired visualisation may be offered as an optional tradition-based theme with no healing claim. It does not receive extra points or become a mandatory office practice.

V4 headphone use is optional. Provide an explicit output choice/preview and quiet starting volume. Connecting headphones never starts playback. Unplugging/disconnecting pauses playback rather than suddenly playing through speakers. Audio never enables the microphone or camera. Retain offline, properly licensed starter audio; no third-party streaming account required. Transcript and stop button remain available, with eyes-open and silent alternatives.

Colour experience: user chooses a comfortable static palette, with optional slow transition and reduced-motion mode. Do not force full-screen colour, high brightness, flashing or a particular colour-emotion association. Visual breathing cues are optional; offer screen-off/audio-only sessions so relaxation does not become more screen work.

Deferred V4 acceptance SND-01: once-daily invitation is optional, respects work/focus settings and does not replay a backlog; duration, stop, transcript and silent mode work. Test disconnect/output changes and initial volume. Content review and asset licensing recorded.

Deferred V4 acceptance COL-01: spiritual theme is off by default and never inferred from user identity; palettes work with accessibility preferences; no clinical/chakra/brainwave claims, scores or guaranteed benefit in UI/audio.

## 5. V3: hosted CA community, same avatar and one group

Requested behaviour: hosted profile uses the user's selected pet/avatar; user can discover/select a group and belong to **at most one active group**. Include both group ranking and an optional fraternity-wide ranking view. Participation/public display requires a separate opt-in; hosting an account alone must not publish private health data.

Proposed initial scope: curated groups, searchable directory, join/leave/switch, member avatar/alias and weekly habit-consistency board. Group creation, chat, cash prizes and employer appraisal exports are deferred. Leave old group before joining another; make the switch atomic server-side. Concurrent join requests, retries and two-device joins must never create two active memberships. Preserve personal history but do not duplicate group points on a switch. Late joiners are labelled; historical results stay attributed to the original scoring period/group rather than copied.

Same-avatar requirement means synchronising selected pet ID and supported appearance/name settings to the hosted profile when the user enables this capability; it does not require publishing local health logs. Allow separate community alias. Explain the display payload (alias, avatar, selected score, rank/group) separately from private server-sync categories; declining community sharing leaves personal use intact. Withdrawing ranking participation removes current public listing.

Planner's scoring proposal for design review: bounded habit consistency, not health, productivity or spirituality. Use a fixed published set of eligible habits, each credited at most once per eligible day. Normalise to scheduled days frozen for the scoring week, mark insufficient coverage provisional, cap at 100%, and treat ties equally. A completed break, suitable routine and logging against the personal water plan can contribute; never reward extra litres, exercise minutes, work hours, high biometric values or meditation/chakra claims. Wearable ownership must not add points. Label self-report honestly; this is a friendly board, not a verified health competition. Final weights, eligibility and tie rules must be specified in prototype before implementation.

Acceptance COM-01: same avatar across linked desktop/hosted profile; at most one active group under concurrent requests; join/leave/switch and retry tests; aliases/visibility preview.

Acceptance COM-02: global ranking visibility separately selectable; no score inflation by logging duplicates, exceeding water goal or owning a watch; empty/provisional/tie/withdrawal states shown; ranking exposes only opted-in fields.

This is an explicit update to the earlier no-ranking default: optional personal community rankings are now requested. Mandatory employer rankings and productivity scoring remain excluded.

## 6. Additional ideas — recommendations, not committed scope

- Two pet modes, "Help me focus" and "Help me unwind", with simple rules and no inferred emotional diagnosis.
- A private end-of-day shutdown note: one win, tomorrow's first task, chosen finish time.
- A recovery-friendly comeback message after missed days, without resetting a punitive streak.
- Cooperative group goals, such as members taking a screen-free break, alongside optional rankings; no raw health-data comparison.

## 7. Version and design impact

V1 adds REC-01 and FOC-01; V2 adds basic CAL-01; V3 adds COM-01/02. V4 defers sound/headphone sessions (SND-01), colour sessions and optional spiritual visualisation (COL-01). V4 has no committed delivery date or allocated budget and is outside the November 5 V1–V3 target. No previously requested watch is removed. The Nov 5 target and $200 allowance are unchanged, but these additions increase work; Claude must re-estimate before build approval rather than assume they are free. Use existing pet, scheduling and logging components; keep community features bounded as above.

Claude must add prototype journeys for recovery-day invitation/planning, deadline-day decline, Pomodoro controls and combined break, simple V2 meditation timer, avatar sync, group join/switch/leave, global ranking consent, and capped/provisional/tied scores. The user approves the experience and scoring before development. This revision is documentation only.


V4 headphone session/disconnect, optional spiritual theme and colour-accessibility journeys are future design tasks, not conditions of the current prototype gate. Ordinary pet colour customisation and accessibility settings remain in V1.



---
Source: docs/Claude_Design_Brief.md

# Claude standing design brief

18 October 2026 • For the user's Claude project/cloud environment

You own HealthierYou design artifacts and, after explicit approval, all product development, developer tests and fixes. Codex owns product planning, scope, acceptance criteria and independent review. The user owns PRD/prototype/go-live approval. Use multiple specialist agents with bounded tasks and a single integration owner.

## First assignment: read, challenge and design — no product code

Read PRD.md, Scope_Decisions.md and the supporting discovery/protocol files. Latest user decisions override the historical conversation and earlier Claude review. Do not restart competitor research or require a two-week third-party-app baseline. Do not rename versions: V1 computer-only, V2 webcam, V3 selected wearable/hosted integrations.

PRD 0.3 includes Recovery_Focus_Community_Addendum.md. Prototype its recovery-day invitation, user-started Pomodoro, simple optional V2 meditation pause; defer sound/headphone and colour sessions to V4, and V3 same-avatar community with at most one active group and optional global/group ranking. Chakra themes are optional tradition-based reflection, not a clinical priority or measured outcome. Re-estimate added scope within the existing deadline/budget; do not start product development.

Return one consolidated response containing:

1. Acknowledgement of repository branch/file access and what you can actually execute. Demonstrate capabilities rather than assuming a Claude.ai project connection gives Git write/build access.
2. A short list of material contradictions or feasibility blockers, with your recommended resolution. Handle routine choices yourself and record them; do not make the user relay tiny questions.
3. Architecture options and evidence-backed feasibility assessment for LAN + lightweight pet, target Windows PC, webcam and all three wearable combinations. Do not build executable spikes before the current approval hold is resolved; propose the spike, evidence needed and effort instead.
4. Two coherent pet visual directions, then a recommended direction using ten characters with shared interaction rules. Original/licensed art only. User can edit name, scale, supported colours and tone; no animation-authoring suite.
5. Complete wireframes and a clickable, simulated prototype satisfying PRD section 10. Prototype code/assets solely for interaction demonstration are allowed; no product scaffolding, production database, integrations, installers or deployment under the label of prototype.
6. A traceability table mapping every PRD requirement ID to prototype flow/state, intended validation and any unresolved limitation.
7. One checkpoint packet: changed files, decisions, test/review evidence, blockers, remaining credit balance if available and next action.

## Working rules

- V4 sound/headphone and colour experiences are future backlog only. Do not build them or require their wireframes for the current V1–V3 approval gate. V4 has no committed date or budget. Basic pet colour editing remains in V1.

- Do not begin product implementation until the owner approves the PRD and explicitly approves the prototype review gate. Record the approval in the repository.
- After that gate, you may independently execute accepted build slices; Codex reviews acceptance and you fix findings. Never ask Codex to implement fixes.
- V3 requires real device evidence for Apple Watch Series 8/iPhone 13, likely Fossil Gen 5/S24 FE, and uncertain Galaxy Fit/A25. Missing exact models are tracked, not guessed. All three remain scope unless user approves a change.
- Server sharing is user-selected; unshared personal information stays on the companion. Minimum connection metadata is explained separately. Shared-server deployment does not justify collecting every personal event.
- All ordinary reminders are pet-led. A large always-open health dashboard is contrary to the requested experience.
- Cellphone distraction starts with labelled self-report. Camera phone-use or phone statistics require evidence; neither implies knowledge of intent.
- Exercise content needs authoritative provenance, suitable adaptations and review; no diagnosis or claimed treatment. Weekly/monthly coaching must be traceable to actual coverage and events.
- Use the shared repository as the record, not chat memory. Give issues stable IDs, pin reviews to commits, batch nonurgent questions and escalate only meaningful blockers.
- Stop before new spending, store-account changes, irreversible actions or material scope changes requiring user authority. Do not expose secrets or personal employee logs in the repository.
- The $200 is a credit ceiling to track, not proof that the scope fits. Reserve capacity for fixes/testing; do not run unbounded agents or repeatedly reread the entire conversation.

## Definition of next handoff

Deliver a prototype the user can click, its source/artifacts, the requirement mapping and one concise design-review packet. Report what is simulated. The expected next event is **prototype review**, not an implementation announcement.



---
Source: docs/Approvals.md

# Approvals

| Gate | State | Evidence |
|---|---|---|
| Start Discovery & PRD | Approved | Owner approved in writing, 18 October 2026 |
| Publish planning package to public CA-health-advisor repository | Approved | After being asked explicitly about publishing the PRD, device details, schedule and $200 budget, the owner requested publication to the repository. This approves publication only. |
| Lightweight employee-PC companion | Approved | User explicitly answered yes in the scope discussion |
| Written PRD revision 0.3 | Approved | Owner approved in writing, 18 October 2026. Sound/colour deferred to V4. |
| Claude design brief | Approved | Same user message authorises the design assignment; no actual wireframes/prototype exist yet. |
| Executable feasibility spike before prototype | Not authorised | Research/design only under current hold |
| Claude wireframe/prototype revision | Pending | Not produced |
| Product implementation | Blocked | Requires PRD and explicit prototype gate approval |
| Pilot acceptance / go live | Pending | No release exists |

Update with exact document/commit/prototype revision and the user's approval evidence. An agent cannot approve a gate on the user's behalf. Codex may accept routine slices within approved scope, not override these gates.




---
Source: docs/Backlog.md

# Discovery and design backlog

Draft 18 October 2026. This is a gate/work register, not an authorised coding plan.

| ID | Work | Owner | State / dependency | Required evidence |
|---|---|---|---|---|
| D01 | Consolidate scope and PRD | Codex | Approved revision 0.3 | PRD with stable IDs and acceptance criteria |
| D02 | Research three device routes | Codex discovery agent | Desk research complete; hardware checks pending | Wearable_Feasibility.md; later physical model/record proof |
| D03 | Confirm execution/credit environment | User + Claude | Pending | Claude code execution/write smoke check, eligible credit balance |
| D04 | Publish planning to user-supplied repository | Codex | Revision 0.1 published; expanded revision 0.3 publication awaiting disclosure confirmation | Repository commit and verified document tree |
| D05 | Review and approve PRD | User | Approved 18 October 2026 | Approval recorded in Approvals.md |
| D06 | Architecture/feasibility proposal | Claude | Brief ready; no executable spike authorised | Evidence and explicit unknowns; resource and device plan |
| D07 | Wireframes and simulated clickable prototype | Claude | Design brief approved; ready for assignment | PRD section 10 flows, ten pets, requirement mapping |
| D08 | Independent design review | Codex agents | Requires prototype | One consolidated review and rubric result |
| D09 | Prototype approval | User | Requires resolved D08 | Explicit approval of named artifact revision |
| D10 | Detailed implementation plan/slices | Claude with Codex acceptance planning | After PRD; implementation after D09 | Dependencies, owners, test and budget allowances |
| D11 | Local deployment/test inventory | User | Three pilot people confirmed; other inputs pending | Server specification, rollout count, tester schedule, device OS/model |

No build task is active. V1, V2 and V3 remain required. Failed feasibility produces an explicit decision, not silent removal from scope.

PRD 0.3 adds REC-01/FOC-01/CAL-01/COM-01/02. Claude must include these in prototype traceability and re-estimate effort/credits before implementation approval. See Recovery_Focus_Community_Addendum.md; no hidden extension of deadline or budget.





---
Source: docs/Blockers.md

# Blockers and decisions needed

18 October 2026. Continue independent discovery/design while access is resolved.

| ID | Affected work | Evidence / issue | Next action | Latest useful time |
|---|---|---|---|---|
| B01 resolved | Public planning publication | User explicitly requested upload after the public-disclosure question naming payload and public repository | Publish through the normal approved connector route; record commit in repository history | Authorised publication |
| B02 | Claude execution and coordination | User named Claude.ai project; official docs distinguish context sync from Code cloud execution/write | Confirm Claude Code cloud access and $200 credit eligibility; perform docs-only write check | Before prototype dispatch / paid work |
| B03 | Apple integration | Native bridge build/signing/test route not arranged | Claude identifies authorised macOS/Xcode/signing/test distribution route and requests any missing user access | Feasibility gate |
| B04 | Fossil and Samsung integrations | Exact models/OS and existing watch-origin sync unknown | Inspect actual models, app versions and sample source records on user's phones | Feasibility gate |
| B05 | Pilot environment | Three pilot people confirmed; server spec, total rollout count and tester schedule unknown | User supplies remaining inventory; PRD uses three real and ten simulated clients as test planning default | Before UAT scheduling |

No approval to purchase services or store memberships is implied. No automated Codex–Claude relay is active.


---
Source: docs/Budget.md

# Budget control

18 October 2026. User states $200 Claude cloud-session credits. Account eligibility, actual current balance and expiry not yet verified. No Claude session has been dispatched by Codex.

Proposed envelopes: design/feasibility $30; V1 $45; V2 $25; V3 $45; fixes/packaging $30; reserve $25. Total $200. These are planning allowances, not a cost forecast or observed spend.

| Date | Phase/session | Balance before | Balance after | Spend | Evidence |
|---|---|---|---|---|---|
| 2026-10-18 | Setup pending | Unknown | Unknown | Unknown | User-stated allocation only |

Record balance evidence at phase checkpoints. Never invent dollar usage from token estimates. See Delivery_and_Claude_Protocol.md for stop/review thresholds. Other service costs require separate authority.


---
Source: docs/Decisions.md

# Decision log

18 October 2026.

| ID | Decision | Authority / state | Reason |
|---|---|---|---|
| DEC01 | All three versions targeted for Nov 5 | User confirmed | Fixed requested completion date |
| DEC02 | LAN server plus employee companion | User confirmed | Central access with desktop pet |
| DEC03 | Claude builds; Codex plans/reviews | User confirmed | Clear separation of ownership |
| DEC04 | No build before approved prototype | User confirmed | Visual quality and complete flows first |
| DEC05 | V3 limited to three owned watch/phone pairs | User confirmed; models partly uncertain | Bounded compatibility scope |
| DEC06 | Pet-first with settings and progress screens | User confirmed | Minimal daily screen interaction |
| DEC07 | User-selectable server sharing | User confirmed | Explicit user control |
| DEC08 | Per-device daily watch-origin steps as minimum V3 proposal | Planner proposal, pending PRD/feasibility | Testable contract instead of vague wearable support |
| DEC09 | Individual summaries; admin service status only | Planner proposal | Default audience while manager analytics not requested |
| DEC10 | Use public accounts-NKlodha/CA-health-advisor | User supplied repository; visibility verified | Planning/source shared there; no private transcript or employee data upload |
| DEC11 | Three-person pilot | User confirmed 18 October | Server and total rollout sizing still unknown |
| DEC12 | Recovery-day invitation and on-demand Pomodoro | User requested; detailed settings proposed in PRD 0.2 | Support deadline recovery without automatic increased water targets or health-debt claims |
| DEC13 | Sound and colour experiences deferred to V4 | User explicitly requested V4; supersedes earlier V2 proposal | Outside November 5 target; simple V2 meditation remains proposed; no clinical/chakra claims |
| DEC14 | Hosted same-avatar community, max one joined group, optional ranking | User requested; scoring/visibility defaults proposed | Overrides earlier blanket no-ranking proposal for voluntary community use only |

PRD.md is the consolidated proposal. Research notes may discuss alternatives; they do not override the approved requirement baseline.


---
Source: docs/Delivery_and_Claude_Protocol.md

# HealthierYou delivery and Claude coordination protocol

Status: proposed delivery baseline for PRD review, 18 October 2026. Dates use Asia/Kolkata. Target: all three agreed versions released by 5 November 2026. This target is not yet a validated delivery forecast.

## Authority and development hold

- User: approves PRD, wireframes/clickable prototype, pilot acceptance and go-live; supplies real Windows and wearable testing.
- Codex: project planning, scope control, requirements, architecture review, independent acceptance and security review. Codex does not develop the product.
- Claude: design artifacts, architecture proposal, implementation, developer tests, packaging, deployment instructions and fixes. Its coordinator owns integration of its agents' work.
- Product implementation is BLOCKED until explicit user approval of the wireframes and clickable prototype. Approval of discovery is not build approval. A filename or an agent's claim does not constitute approval.
- Before approval, Claude may research, document, produce wireframes and create an explicitly simulated presentation prototype. No production backend, desktop companion, wearable application or deployment. Experimental executable spikes need a separate explicit authorization while this hold applies; otherwise use documentation/device capability evidence and mark runtime feasibility unverified.
- Preserve synced `sources/` material unchanged. Treat reference documents and old chat suggestions as background, not authority over current requirements.

## The Claude environment must be verified

The ordinary Claude.ai project GitHub integration imports selected repository files as context. Its documentation says sync covers file names/contents on a branch, not commit history or pull-request metadata, and offers a manual refresh. That connection alone is insufficient evidence that Claude can run builds or write changes. [Anthropic: Use the GitHub integration](https://support.claude.com/en/articles/10167454-use-the-github-integration)

Claude Code cloud sessions can clone repositories and push branches with an authorised GitHub connection, run work in the cloud, use subagents and create pull requests. Cloud sessions share account usage limits. Official documentation describes review-triggered auto-fix and cloud session follow-up mechanisms. Availability and permissions must be tested on the user's account rather than inferred from a normal Claude.ai project. [Anthropic: Claude Code in the cloud](https://code.claude.com/docs/en/claude-code-on-the-web)

Recommended arrangement: retain the Claude.ai project for reference/discussion if desired; use an authorised Claude Code cloud workspace for execution. The user has stated $200 of cloud-session credit: record its actual eligibility, expiry and remaining balance before dispatching paid work. Do not assume it is API credit, that it funds every surface, or that $200 guarantees completion.

Setup exit criteria, all documented with links or evidence:

1. The user-supplied public accounts-NKlodha/CA-health-advisor repository is the shared record. Codex connector access is verified; Claude access must be tested. Keep private transcripts, credentials and employee records out.
2. Claude can read the approved document revision and create a documentation-only branch/commit. This is the write-access smoke check before any build permission.
3. The execution surface and available credit are confirmed. No credentials belong in the repository or chat documents.
4. There is a verified way to wake Claude when a review is ready and a verified way for Codex to inspect results. Until tested, coordination is manual at gates; a shared repository does not automatically start an agent.
5. Record repository URL, default branch, Claude session link, current document revision and integration status in `docs/STATUS.md`. A project/session link alone does not grant access.

No external setup or cross-agent message has been performed merely by writing this protocol.

## Work allocation

Codex uses separate review agents for product/acceptance, wearable feasibility, and delivery/coordination. After build approval, independent testers review completed artifacts without being the authors of those artifacts. They report reproducible failures to Claude; they do not quietly fix product code.

Claude uses a coordinator and a small number of bounded workers. Suggested initial split is desktop/pet/LAN, webcam, and mobile/wearable/hosted integration. A separate test pass validates completed slices. These are responsibilities, not a commitment to run four expensive sessions continuously. Start with two concurrent implementation workers after interfaces are agreed; expand only for genuinely independent tasks and within the budget. Avoid duplicate research, overlapping file ownership and repeated full-history prompts.

Each work item states an owner, requirement IDs, dependencies, files/components owned, acceptance evidence, spending allowance and stop condition. Claude's coordinator integrates shared contracts before parallel work and resolves routine merge conflicts. A worker may not change scope or declare another worker's untested integration complete.

## Target calendar and evidence gates

| Dates in 2026 | Work | Exit evidence |
|---|---|---|
| Oct 18–19 | Discovery, PRD, backlog and environment checks | User-approved PRD revision; known assumptions and device uncertainties listed |
| Oct 19–20 | Feasibility assessment and architecture proposal | Separate result for each watch/phone pair; LAN, desktop overlay and low-memory approach; unknowns explicitly marked |
| Oct 20–22 | Claude wireframes and clickable prototype | All three versions, ten pets, settings/sharing, Monday–Saturday movement, weekly/monthly progress and failure states demonstrated |
| Oct 22 | Design review gate | Codex consolidated review resolved; explicit user approval linked to prototype revision; production development unlocked only then |
| Oct 23–28 | Build in slices and Claude testing | Runnable incremental artifacts, automated checks and version-specific evidence; independent reviews begin as slices finish |
| Oct 29–31 | Independent testing, security review, fixes and user UAT | Tested 4 GB Windows machine, LAN server/client, camera cases and all three real wearable pairs; no unresolved release blockers |
| Nov 1–3 | Office pilot | Three confirmed pilot users; actual usage and issues recorded, fixes rechecked, missing coverage acknowledged |
| Nov 4 | Release readiness | Install/upgrade/rollback/backup evidence, release notes and user go-live decision |
| Nov 5 | Go live and initial review | Accepted release deployed; initial operational checks and issues recorded |
| Nov 12, proposed | Longer post-launch review | A week's real use, reminder burden, reliability and habit-engagement trends |

These dates require prompt gate decisions and access to physical devices. A delayed gate does not authorise skipping testing. Escalate the effect on dates and scope; only the user can approve changing either. Early November 5 review is operational, not a claim of long-term health improvement.

### Scope must remain visible

V1: self-contained office server and lightweight Windows employee companion, LAN access, no camera/wearable dependency. Pet-led reminders and logging, exercise routine, settings, sharing choices and personal progress.

V2: V1 plus webcam functions approved in the PRD and supported by evidence; phone-distraction inference must remain labelled as uncertain and user-correctable.

V3: hosted use plus separately validated integrations for Apple Watch Series 8/iPhone 13, probable Fossil Gen 5/Galaxy S24 FE, and Samsung Galaxy Fit of unconfirmed model/Galaxy A25. All three are required targets. A prototype, mock data, manual entry or unsupported status screen is not a completed integration. If any route proves unavailable, escalate immediately rather than silently removing that device. Public app-store approval must be treated as an external dependency if the chosen route requires it.

## Repository as the shared record

The following paths are proposed coordination records; their listing does not mean they already exist:

| Record | Purpose / writer |
|---|---|
| `docs/PRD.md` | Versioned requirements and acceptance criteria; Codex owns baseline |
| `docs/Backlog.md` | Work IDs, owner, dependencies, state and evidence links; coordinator updates |
| `docs/STATUS.md` | One current summary: gate, revision, completed work, next work, risks and credit balance |
| `docs/Decisions.md` | Decision, reason, alternatives, approver and affected requirements |
| `docs/Review_Queue.md` | Batch of completed slices ready for Codex, exact commit and test evidence |
| `docs/Blockers.md` | Only actionable blockers with attempted resolution, requested decision and deadline |
| `docs/Approvals.md` | PRD, prototype and release approvals with user message/date and artifact revision |
| `docs/Budget.md` | Actual credit observations and phase allocations; never invented token/dollar estimates |
| `design/` | Wireframes, prototype exports and design decisions |
| `evidence/` | Sanitised test reports, screenshots and device acceptance records |
| `releases/` | Release manifest and instructions; large installers can be repository release assets |

Before starting a slice, read current approvals, decisions and backlog. At completion, publish one evidence packet containing requirement IDs, exact commit/build, change summary, test environment, actual results, known limitations and next action. Link existing evidence rather than repeat conversations.

Review at stable slice boundaries. Batch nonurgent findings into one review. Claude fixes them and reports the new commit with rerun evidence. A review is stale if relevant code changed after its recorded commit. Codex routine acceptance authority does not include overriding user design or go-live gates.

## Low-noise communication

Once a reliable connection is configured, use repository records and supported execution triggers. Verify any trigger with one documentation-only test. Enable no unbounded polling or paid retry loop. Ordinary record updates should not notify the user; escalate a gate decision, missing access/device, infeasible requirement, material budget/date risk or reproducible blocker.

Use one blocker message containing: affected requirement; evidence; what was tried; two feasible choices if available; recommendation; impact if unanswered. Continue independent authorised work. Do not exchange new prompts for every small fix. No automated connector/trigger is claimed active in this draft, and no ongoing monitoring starts merely because this document describes it.

If only the normal Claude.ai project is available, use a single durable brief and batched handoffs at milestones. State that this fallback needs human transfer; do not pretend the two systems can communicate autonomously. Authentication/setup may need the user once. Do not ask for passwords or tokens in chat.

## Proposed $200 cloud-credit allocation

These amounts are management envelopes, not model pricing, a cost forecast or evidence of funds spent. Actual account credit rules prevail.

| Phase | Proposed cap |
|---|---:|
| Claude feasibility, architecture, wireframes and prototype | $30 |
| V1 implementation and developer tests | $45 |
| V2 implementation and developer tests | $25 |
| V3 implementation and device integration | $45 |
| Claude fixes from independent testing/UAT/pilot, packaging | $30 |
| Unallocated contingency | $25 |
| Total | $200 |

Log the visible credit balance before/after meaningful batches. If exact usage is unavailable, mark it unknown and request a balance check rather than assigning fabricated costs. At $100 spent, review remaining work and burn. At $150, stop optional polish and reconcile remaining acceptance work. At $175, obtain an explicit contingency decision before starting more paid work. Never silently exceed $200 or shift to another paid service. Reallocation inside the total is a documented planning decision; additional expenditure requires user approval. Hosting, Apple signing/membership, app-store costs and any paid services are not assumed funded or authorised by these credits.

## Release decision

Every required version and target device has an acceptance result against the approved scope, with no substitution of simulated evidence. Required checks include actual pet overlay and responsiveness on the slow PC, user-controlled data sharing, reminders under focus/sleep/restart, LAN disconnect/reconnect, camera opt-out, wearable sync delays/denial, upgrade and recovery. No critical unresolved security or data-loss defect may be hidden as a limitation. The user approves pilot and go-live with the remaining nonblocking limitations visible.

If evidence cannot be obtained by the deadline, report the affected acceptance item as incomplete. Completion means accepted working versions, not just merged code or a passing cloud test suite.




---
Source: docs/Discovery_Findings.md

# HealthierYou discovery findings

Date: 18 October 2026. Status: supporting discovery research; not approved architecture, clinical guidance or permission to develop. PRD.md consolidates the selected defaults and is authoritative over alternative suggestions in this research note. Claude owns product development and wireframes. No product code is authorised before the user's prototype gate.

## Product problem and outcomes

CA-firm employees need useful health-habit prompts while working in accounting applications, without maintaining a dashboard or buying hardware for V1. Success means an unobtrusive companion that makes recording and acting easy, survives ordinary office interruptions, and provides credible feedback. Habit logs are not evidence of medical improvement, productivity, employee performance or disease prevention.

Confirmed: a self-contained LAN server installation, self-contained employee companions, 10 editable pets, V1 computer-only, V2 existing webcam, V3 hosted wearable support for Apple Watch Series 8/iPhone 13, likely Fossil Gen 5/Samsung S24 FE, and uncertain Galaxy Fit model/Samsung A25. Confirm exact watch models during feasibility. All three versions are targeted for 5 November 2026, subject to evidence-based acceptance and external dependencies.

## Pet-first workflows and proposed defaults

1. First run: connect to server, create personal identity, choose a pet, set working days/hours and water goal, choose sharing categories, then return to work. Monday–Saturday is the proposed routine schedule; users may mark nonworking days.
2. During work: pet stays in a small movable desktop position. A single compact bubble gives one action. Actions include Done, Snooze, Skip, Log water and Focus. A dismissal never records completion.
3. Hydration: choose a personal daily goal and saved cup sizes; log with one action, edit or undo mistakes. No universal mandatory water amount. Zero logs means no recorded intake, not proof of dehydration.
4. Movement: show today's short routine in a pet bubble, with an expandable guide only when requested. Offer a seated alternative and skip. Completion is self-reported unless an explicitly validated future feature says otherwise.
5. Focus: PRD-selected 30/60/90-minute presets plus custom duration. Pause nonurgent prompts, resume without replaying a backlog, and allow early exit.
6. Weekly review: offer at the user's chosen end-of-week time; if missed, offer on the next scheduled workday, once only unless requested. Present strengths, gaps in recorded habits and at most three suggested adjustments. Do not interrupt immediately at login.
7. Settings and progress are ordinary screens. No full dashboard needs to remain open.

Proposed interruption rules: never steal keyboard focus; no sound by default; only one bubble visible; coalesce simultaneous reminders; respect explicit quiet/focus modes; pause on lock/sleep; recover after wake without a burst. Offer pause/hide and a reliable way to reopen the pet. Ensure keyboard navigation, readable scaling, reduced motion and a non-colour-only status indicator.

The 10-pet library requires actual selectable, recognisably different assets, not ten colour variants counted as different characters. Proposed editing scope: name, available colours/accessories, size, voice/personality preset and reminder wording. Arbitrary animation editing/generation is out of initial scope unless separately approved. All pets share the same functional states and actions. No persistent guilt, sad-pet punishment or competitive employee rankings.

## Six-day movement content framework

This is a proposed editorial rotation, not a prescribed exercise dose or an assertion that office breaks meet full exercise guidelines. WHO recommends activity across the week and muscle strengthening involving major groups on at least two days; brief mobility prompts are only a small contribution. Source: [WHO physical activity guidance](https://www.who.int/europe/news-room/fact-sheets/item/physical-activity).

| Day | Content emphasis | Candidate movement family |
|---|---|---|
| Monday | Neck, shoulders and upper back | Comfortable neck turns and shoulder/chest mobility |
| Tuesday | Hips and thighs | Seated marching; optional suitable standing alternative |
| Wednesday | Ankles, lower legs and movement break | Seated ankle movement and an optional short walk |
| Thursday | Chest and trunk | Gentle seated chest opening and comfortable upper-body turning |
| Friday | Hands, wrists and forearms | Comfortable finger opening/closing and supported active wrist movement; content review required |
| Saturday | Whole-body change of position and review | Adaptable seated/standing sequence repeating comfortable earlier movements; identify routines to keep |

NHS sitting exercises provide source examples for neck rotation, chest mobility, upper-body turning, hip marching and ankle movement. Their setup guidance uses a stable chair without wheels and encourages gradual progression. This matters in offices with wheeled chairs: the app must not instruct chair-dependent movements without checking appropriate seating. Source: [NHS sitting exercises](https://www.nhs.uk/live-well/exercise/sitting-exercises/).

The hands/wrists candidate draws on Cambridge University Hospitals' patient leaflet, which illustrates active wrist movement with forearm support and making a fist then straightening the fingers. These are movement examples for content review, not a general-office prescription or rehabilitation programme. Select gentle active movements only; do not import assisted stretching or treatment instructions into default prompts. No exact dose is proposed. Source: [CUH wrist range of movement exercises](https://www.cuh.nhs.uk/patient-information/wrist-range-of-movement-exercises/).

Before content approval, each routine needs original illustrations/text, source reference, intended body regions, accessibility alternative, setup requirements, stop/skip wording and a content reviewer. Do not copy NHS imagery without checking reuse terms. No forced range, ballistic stretching, complex balance tasks or automatic escalation based solely on a completion streak. Users who are unsure whether a routine is suitable should follow advice from their care professional. Discomfort triggers stop/skip, never a completion penalty. Specific strengthening content remains a coverage gap needing sourced review; do not claim every major region is fully trained or that this rotation is a complete exercise programme.

## Weekly and monthly insights

Use transparent definitions and expose denominator, source and coverage:

| Measure | Definition | Interpretation limit |
|---|---|---|
| Hydration goal days | Days with logged volume meeting that day's saved personal goal / eligible days with hydration logging | Self-report; show days without records separately |
| Movement sessions | Completed self-reported sessions, plus skipped and offered counts | Not measured exertion or posture improvement |
| Reminder response | Done / actionable prompts actually delivered, with snooze separate | Suppressed/offline prompts excluded; not adherence to all possible health needs |
| Habit consistency | Recorded days / configured working days | Holidays/leave excluded; partial coverage labelled |
| Phone events | Manual reports and any validated observations shown separately | Never a unified objective distraction score |
| Wearable measures | Source-specific supported values with last-sync time | Stale/missing values shown as unavailable, not zero |

Weekly copy should say “You logged water on 4 of 6 scheduled days” rather than “You drank enough on 4 days.” Suggestions are simple rules users can inspect: change prompt timing, reduce an unhelpful prompt frequency, or choose a more comfortable routine. No causal health conclusions. Chart monthly totals alongside per-eligible-day rates so longer months do not imply improvement. Retain historical goal values for fair comparisons, display incomplete months, and avoid percentage-change claims from a zero baseline. Allow corrections with recalculation and handle time-zone/day-boundary changes deterministically.

## Honest phone-distraction measurement

- V1: optional manual “phone interruption” log, undo and optional work/personal/unsure classification. No claim to read the phone.
- V2: feasibility candidate only—possible visible phone-use events. Hand-to-face movement, off-camera phone use, legitimate calls, occlusion and poor lighting are required test cases. A camera cannot establish distraction or intent. No reliability claim until a labelled test set demonstrates it.
- V3: device/platform-permitted usage information may supplement reporting. Wearing one of the supported watches does not establish access to phone unlocks, application use or pickups.
- Proposed review: display source-specific counts and coverage, let users dismiss/correct events, avoid individual managerial distraction rankings.

## Sharing, LAN and failure behaviour

Proposed default: optional behavioural categories off until selected, with a plain-language preview and visible audience. Local personal summaries remain available with optional sharing disabled. Minimum account/device/connection information is separately explained. Administration should cover installation, user access and service health; individual health summaries require a defined, explicitly authorised audience.

Acceptance cases: disable a category while offline; reconnect; verify queued records in that category are not uploaded. Distinguish stopping future sharing from requesting deletion of already shared records. Deletion scope must address server copies, sync queues and backup retention. Avoid claiming a browser/server application sends no data.

LAN disconnection/server shutdown must leave local reminders, settings and logging functional, show unobtrusive sync state, and resume without duplicate records. Reinstallation, multi-PC use, duplicate identities, clock changes, unexpected shutdown, full disk, malformed server address and server IP change need explicit recovery decisions. Shared-PC identities must not mix records. A failed wearable sync must not block V1 habits; absent/denied/busy camera must fall back to V1.

For the 4 GB Windows 10/11 client, performance gates must be measured with real office software open. Proposed targets for feasibility, not promises: idle resident memory at or below 150 MB, idle CPU average at or below 2% on the test PC, no persistent animation GPU load, no perceptible typing/focus interference, and pet-action feedback within 500 ms for local actions. Measure webcam mode separately, including thermal/CPU impact and a low-resource fallback. The server's specification, expected concurrent users and available network remain unknown; do not extrapolate client suitability to server capacity.

## Acceptance gaps to close before build gate

1. Three pilot volunteers confirmed. Server hardware, total rollout users, network access, tester schedule and installer rights still need verification.
2. Supported Windows builds, unattended updates, rollback and recovery policy.
3. Explicit data-sharing audience and default visibility for office administrators.
4. Water-goal configuration, day rollover and time-zone policy; no prescribed universal target.
5. Validated movement content covering requested regions and accessible alternatives.
6. Exact webcam feature list and quantified false-positive/coverage thresholds.
7. Exact wearable models, obtainable metrics, freshness and successful end-to-end acceptance per device; app-store acceptance cannot be assumed.
8. Final pet editor limits, asset rights and accessibility criteria.
9. Prototype coverage of empty, offline, disabled-sharing, camera-unavailable and stale-wearable states.
10. Pilot success criteria: proposed zero unresolved critical/high defects, no record mixing or unauthorised category upload, no focus-stealing, functioning rollback, and agreed client resource thresholds achieved on the slowest PC. User adoption targets should be agreed after pilot size is known.

## Reviewer recommendation

Approve a bounded PRD with these gaps recorded as named feasibility decisions. Do not disguise unvalidated phone detection or unavailable wearable data as delivered functionality. Keep the user's three-version deadline visible, while requiring a separately demonstrated working path for each V3 target and explicit user approval of any exception. Claude should present complete pet-centred wireframes and a labelled simulated prototype before the product build gate.




---
Source: docs/PRD_Review.md

# Independent PRD builder review

18 October 2026. A fresh agent read the PRD and supporting discovery documents without authoring them. Review scope: whether a builder can plan; no software was tested.

This review covered revision 0.1. Revision 0.2 adds Recovery_Focus_Community_Addendum.md and requires additional prototype/content/scoring review; do not treat the old verdict as approval of new features.

A separate read-only agent reviewed revision 0.2 additions for unsupported health claims, community membership and timer interactions. No material health-claim or one-group contradiction was found. Its timer precedence ambiguity was resolved in the addendum with explicit start/pause/lock/meeting behaviour; universal comfort wording was removed. This is document review, not clinical approval or runtime testing.

Verdict: sufficient to plan discovery and a simulated prototype; insufficient evidence to commit the implementation deadline.

| Finding | Disposition |
|---|---|
| Wrist reminders mandatory in research but optional in PRD | Resolved: per-device daily watch-origin activity summary is proposed minimum; wrist delivery is separately tested capability unless added at gate |
| Focus/performance defaults conflict | Resolved: PRD 30/60/90 min, idle <=2% CPU, action <=500 ms is canonical; supporting findings aligned |
| Weekly review timing conflict | Canonical PRD: chosen end-of-week time, next-workday fallback; research to match |
| V2 quality unquantified | Resolved in draft: proposed per-setup coverage, staged detection, false-cue and shutdown checks; runtime validation pending |
| Exercise matrix/content review incomplete | Named Claude author + independent content review + user experience approval; source coverage follow-up requested; no claim of clinical certification |
| Claude/Apple/hosted access unconfirmed | Remains evidence blocker; owner and Oct 20 checkpoint assigned |
| Actual hardware/pilot environments unknown | Remains deployment input; target inventory deadline Oct 22 |

No user approval or runtime feasibility has been inferred from this review. PRD changes remain proposals for user approval. Framework/storage/asset-format decisions are left to Claude's architecture proposal, avoiding unnecessary user technology questions.


---
Source: docs/Review_Queue.md

# Review queue

Current: PRD draft 0.1 independently checked; see PRD_Review.md for findings and dispositions. No product build exists.

For future items record: requirement IDs; exact commit/build; evidence links; tested environment; author; reviewer; findings; disposition; retest revision. Do not mark an unexecuted test passed or a simulated prototype as integrated functionality.



---
Source: docs/Scope_Decisions.md

# HealthierYou scope decisions

Updated: 18 October 2026. Status: discovery decision record. PRD.md is the consolidated draft, not yet approved.

## Delivery and ownership

- Target: V1, V2 and V3 completed by 5 November 2026. Feasibility and external approvals remain to be established; this is not a verified delivery commitment.
- Codex owns planning, acceptance criteria, independent review/testing coordination and release readiness. Codex does not develop the product.
- Claude cloud session owns development, developer testing and fixes, with multiple agents as appropriate. Available Claude cloud session credits: $200; exact billing and remaining allowance require verification.
- User approves PRD, wireframes/clickable prototype and go-live; Codex handles routine slice acceptance within approved scope.
- No product development before approval of Claude's detailed wireframes and clickable prototype. Prototype interactions may be simulated and must be labelled. Executable feasibility spikes before that gate require explicit approval.
- User supplied public repository https://github.com/accounts-NKlodha/CA-health-advisor on 18 October. Claude.ai project repository write access and execution/coordination mechanism are not yet verified.
- Full lifecycle: Discovery & PRD → Plan → Feasibility → Architecture → Wireframes/prototype → Review gate → Build in slices → Claude testing → Independent testing/security → UAT → Pilot → Go live → Post-launch review.

## Deployment

- One office computer/server hosts the LAN service.
- Lightweight self-contained companion installation on each employee PC is explicitly approved.
- Separate hosted offering for advanced users.
- Slowest target client: Windows 10 or 11, 4 GB DDR3 RAM, 256 GB storage. Server specification unknown.
- Competitor research is not a workstream. Ordinary security and functional data-sharing controls remain requirements.

## Versions

- V1: no dependency on camera, wearables, or separately installed third-party tools. Existing computer plus self-contained companion/server packages.
- V2: adds existing-webcam capabilities only; specific detections subject to feasibility and agreed acceptance criteria.
- V3: hosted-user wearable integration targeting the user's Apple Watch, Fossil smartwatch and small rectangular Samsung watch. Other wearables deferred. Exact models, operating systems and paired phones must be identified before promising capabilities or custom watch apps.
- Candidate V3 capabilities must distinguish reading health records, data freshness, notification delivery and interactive watch applications. Store submission and public approval are separate milestones.

## Pet-first interface

- Small desktop pet provides everyday reminders and interactions.
- Initial library of 10 health/lifestyle/helpful-colleague themed pets.
- Pets should be easily editable. Proposed controls: name, appearance, scale, reminder wording and personality; exact editor scope awaits PRD.
- Ordinary screens should be limited to setup/settings, goals, data-sharing controls and progress review. No requirement to keep a dashboard open throughout the workday.
- Water goal is user-configured; quick logging should be available through the pet.
- Wireframes must demonstrate pet reminders, exercise guidance, snooze/focus, logging, customization and weekly review, including interruption/error states.

## Movement programme and motivation

- Monday–Saturday daily exercises, collectively covering most major body regions/movement patterns over six days.
- Exact exercises, progression, adaptations and wording require evidence/content review during discovery; no medical treatment or injury-prevention promise.
- Proposed product behavior: short guided sessions, skip/adapt options and completion logging. In V1, completion is self-reported, not sensed.
- Weekly review: what went well, what was missed and achievable improvements for the following week.
- Month-by-month progress charts to motivate ongoing habits.
- Reviews distinguish logged actions, observed signals and unknown/unrecorded data. Missing data must not be labelled failure.

## Phone distraction

- User requests counts of cellphone distractions.
- Candidate V1 baseline: optional manual marking/check-in. No claim of automatic detection.
- Candidate V2 experiment: webcam-observed possible phone-use events, subject to accuracy, view and ambiguity tests. Phone use does not establish distraction or intent.
- Candidate V3 path: phone-originated usage information if supported and authorized; watch ownership alone does not establish access.
- Final scope and labels require feasibility. Do not silently turn unavailable evidence into automatic phone-distraction counts.

## User-controlled server sharing

- Each user selects which data categories are sent to the office server or hosted service.
- Proposed separate categories: reminder/exercise outcomes, hydration, work-session summaries, camera-derived summaries and wearable-derived summaries.
- Local personal insights should remain useful when optional sharing is disabled.
- PRD must define server audience, preview of shared data, permission changes, future-sync behavior and handling of previously shared records.
- Separate the minimum operational information needed for connection from optional health/behavior sharing. Do not describe server-based use as transmitting no information.

## Outstanding discovery inputs

Revision 0.3 requested additions are documented in Recovery_Focus_Community_Addendum.md: V1 recovery-day invitation and user-started Pomodoro; V2 simple optional meditation pause; V4 deferred sound/headphone and colour sessions; V3 same-avatar hosted community with at most one joined group and optional rankings. Chakra/colour healing claims are not accepted as measurable health requirements; spiritual visualisation is an optional proposed theme. User requested advice; detailed defaults/scoring remain proposals, not approved implementation.

1. Apple Watch Series 8 + iPhone 13 reported; Fossil 'watch 5' (likely Gen 5) + Samsung Galaxy S24 FE; Samsung Fit3/4 uncertain + Samsung Galaxy A25. Exact Fossil/Samsung models and installed operating systems remain to verify.
2. Three pilot participants confirmed; office server specification, total rollout count and tester availability remain pending.
3. Shared repository owner/access and usable Claude cloud development environment.
4. V3 acceptance scope per supported device, feasibility results and store-distribution dependencies.




---
Source: docs/STATUS.md

# Project status

Updated 18 October 2026.

- Phase: PRD 0.3 and Claude design brief approved; ready for Claude design/feasibility/prototype assignment. Sound/colour remain V4 backlog. No product implementation.
- Publication state for revision 0.3: local only. Automatic approval review rejected remote tree creation pending explicit public-disclosure confirmation for the expanded package. Published main still contains revision 0.1. No alternative upload route attempted.
- User PRD approval: received in writing on 18 October 2026.
- Prototype: not yet produced; Claude owns it.
- Product development: blocked by explicit user prototype gate.
- Remote repository: https://github.com/accounts-NKlodha/CA-health-advisor (public). Prior publication approval covered revision 0.1; publication of expanded revision 0.3 was confirmed by the owner on 20 October 2026, with the owner's name removed. PRD/design approval is recorded separately from publication.
- Claude handoff: prepared locally; not delivered to an external Claude session. Claude.ai project versus cloud coding access must be verified.
- Target deadline: 5 November 2026, all three versions; feasibility unresolved.
- Wearable model confirmation: Apple Watch Series 8/iPhone 13 confirmed by user; Fossil Gen 5 inferred from 'watch 5'; Galaxy Fit3/4 uncertain; Samsung phones S24 FE and A25 respectively.
- Deployment inputs: three pilot participants confirmed by user; server specification, total rollout count and tester schedule remain pending.
- No code, installer, hosted deployment, prototype approval or completed test run is claimed.




---
Source: docs/Wearable_Feasibility.md

# HealthierYou V3 — wearable feasibility discovery

Status: desk research only; no device tests or implementation. Official primary documentation accessed 18 October 2026. Deadline requested: 5 November 2026. Product development remains blocked pending the user's prototype approval.

## Decision proposed for the PRD

V3 targets exactly three user-owned combinations: Apple Watch Series 8 + iPhone 13; likely Fossil Gen 5 + Samsung Galaxy S24 FE; Samsung Galaxy Fit (possibly Fit3, model unconfirmed) + Galaxy A25. Do not advertise universal wearable support or identical features across them. Each must pass a real-device end-to-end acceptance test. A manual import or mock demonstration must not be relabelled a completed wearable integration.

Prefer phone-mediated daily activity summaries to continuous raw watch sensing. A hosted website needs a mobile bridge to these on-device data stores; no direct browser-to-watch integration has been established by this research. Upload only categories selected by the user to the hosted account. Desktop/LAN V1 and webcam V2 must remain independent of this path.

## Apple Watch Series 8 + iPhone 13

**Verified platform capability:** HealthKit supports authorised health-data access in native apps. Background delivery needs the entitlement; step-count background notifications on iOS have an hourly maximum frequency. This is not a live streaming feed. [HealthKit setup](https://developer.apple.com/documentation/Xcode/configuring-healthkit-access), [background delivery](https://developer.apple.com/documentation/healthkit/hkhealthstore/enablebackgrounddelivery(for:frequency:withcompletion:)).

**Proposed route, still to test:** watch activity synchronises into the iPhone health store; a small iOS companion reads permitted steps/activity summaries and sends selected summaries to the hosted service. A dedicated watch app is optional unless direct watch interactions are needed. Verify actual installed iOS/watchOS versions, available source data, duplicate-source handling and sync timing on the user's pair.

**Notifications:** mirrored iPhone notifications can reach the Watch, but normal routing depends on phone lock state and watch settings; do not promise simultaneous phone and watch alerts. [Apple notification behaviour](https://support.apple.com/en-ie/108369).

**Delivery dependency:** a compatible macOS/Xcode build environment and Apple signing/distribution route must be arranged. Windows-only testing does not validate iOS/watchOS builds. [Xcode system requirements](https://developer.apple.com/xcode/system-requirements). App-store acceptance is not established or guaranteed by the target date.

## Fossil (likely Gen 5) + Galaxy S24 FE

**Verified generic routes:** Google's Wear OS help describes watch/phone Google Fit synchronisation. Fit's Android app documents Health Connect integration. Health Connect provides an on-device Android data store, accessible through authorised app integration. [Wear OS fitness help](https://support.google.com/wearos/answer/6056874?hl=en), [Fit Health Connect settings](https://support.google.com/fit/faq/6108483?hl=en-GB), [Health Connect availability](https://developer.android.com/health-and-fitness/health-connect/availability).

**Candidate route, not verified for this particular watch:** Fossil → existing watch/phone fitness sync → Health Connect → HealthierYou Android bridge → hosted account. Check the watch model and installed OS, functioning pairing, which app owns its data, and whether actual watch-origin records arrive in Health Connect. Phone-generated steps alone do not prove watch support.

**Important limitation:** modern Health Services requires Wear OS 3 or later. Do not assume it works on a Gen 5; record the installed OS before choosing APIs. If the existing sync path fails, assess an OS-compatible watch sensor app as a separate high-risk route, with battery and background-operation tests. [Health Services requirements](https://developer.android.com/health-and-fitness/health-services).

Avoid starting a new integration around legacy Google Fit cloud APIs: the current migration guide says support ends at the end of 2026. Existing Fit app synchronisation and choosing Fit APIs for our own product are different decisions. [Official migration guide](https://developer.android.com/health-and-fitness/health-connect/migration/fit).

**Notifications:** Wear OS supports bridged phone notifications; actual delivery/actions must be tested on this older device. [Wear OS notifications](https://developer.android.com/training/wearables/notifications).

## Samsung Galaxy Fit (model uncertain) + Galaxy A25

**Model gate:** obtain Settings/About model number before committing. Fit3 findings below do not establish support for a different model or a presumed Fit4.

**Verified Fit3 capabilities:** Samsung documents health/activity tracking and phone notification mirroring. Its product specifications list FreeRTOS; it must not be treated as a Wear OS Galaxy Watch. No supported third-party installable Fit3 app route was established in this research. [Fit3 specifications](https://www.samsung.com/latin_en/watches/galaxy-fit/galaxy-fit3-gray-bluetooth-v5-3-sm-r390nzaalta/), [features](https://www.samsung.com/us/support/answer/ANS10004550/), [notification controls](https://www.samsung.com/ae/support/mobile-devices/how-to-manage-notifications-on-the-galaxy-fit3/).

**Candidate route:** Fit → Samsung Health on A25 → Health Connect → Android bridge → hosted account. Samsung confirms Health Connect synchronisation of activity, heart rate and sleep, but its FAQ's watch-specific examples do not prove every Fit model/data-type combination. Verify Fit-origin records and their freshness on the actual A25. [Samsung Health Connect FAQ](https://developer.samsung.com/health/health-connect-faq.html).

**Alternative:** Samsung Health Data SDK can read stored data and identify source devices including Galaxy Fit. Production distribution requires Samsung partnership registration; developer mode is only for testing, not a release workaround. Prefer evaluating Health Connect first to avoid adding this approval dependency. [SDK FAQ](https://developer.samsung.com/health/data/faq.html), [distribution process](https://developer.samsung.com/health/data/process.html).

## Acceptance evidence required before declaring V3 complete

1. Record exact hardware, OS and bridge/fitness-app versions for all three pairs.
2. For each pair, demonstrate actual watch-origin daily steps in the hosted account; preserve source and last-sync timestamp. Never silently substitute phone-only values.
3. Add other activity/heart-rate data only where the tested data source exposes it. Show unavailable or stale data explicitly. Do not manufacture a common metric across incompatible devices.
4. Demonstrate permission denial, selective sharing, revocation, disconnect/reconnect, retry without duplicates, time-zone/day-boundary handling and deletion semantics.
5. Assess reminders on each wrist under documented settings; distinguish notification receipt from acknowledgement or task completion. This is an additional capability to test, not the PRD's minimum wearable-data acceptance contract unless explicitly added at the feasibility gate.
6. Measure latency and battery impact over normal use. Background access requires platform permissions and scheduling: [Health Connect reading](https://developer.android.com/health-and-fitness/health-connect/read-data).
7. Document install/distribution instructions and tests on the user's physical devices. Store submission is a separate state from approved public availability.

## Deadline risks and decision ownership

Fossil legacy compatibility, uncertain Samsung model, iOS build/signing access and public distribution approvals are unresolved. The three-device requirement is retained. If a device route fails, Claude must provide evidence and alternatives to the planner/user; it must not quietly reduce scope, claim a mock as a completed integration, or promise unsupported hardware behaviour. The user approves any change to the November 5 acceptance scope.

