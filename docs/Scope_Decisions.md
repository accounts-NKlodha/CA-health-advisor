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

1. Apple Watch Series 8 + iPhone 13 reported; Fossil 'watch 5' (likely Gen 5) + Samsung Galaxy S24 FE; Samsung Fit3/4 uncertain + Samsung Galaxy A25. Exact Fossil/Samsung models and installed operating systems remain to verify.
2. Three pilot participants confirmed; office server specification, total rollout count and tester availability remain pending.
3. Shared repository owner/access and usable Claude cloud development environment.
4. V3 acceptance scope per supported device, feasibility results and store-distribution dependencies.

