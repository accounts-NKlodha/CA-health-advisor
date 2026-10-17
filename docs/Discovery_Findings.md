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


