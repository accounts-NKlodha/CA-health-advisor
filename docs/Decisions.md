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
| DEC12 | PRD 0.3 package is the baseline; this repository's PRD.md (0.1) is superseded until replaced | Owner approval 18 October | Repository PRD to be refreshed by a later docs PR |
| DEC13 | Technology choices (Tauri 2, Rust server, MediaPipe, Health Connect, TestFlight) are provisional until the feasibility review | Owner instruction 18 October | Confirmed or replaced at the 22 October review |
| DEC14 | Public repository holds only project documents and synthetic test evidence | Owner instruction 18 October | No billing screenshots, device identifiers, health records or signing credentials |
| DEC15 | Routine decisions recorded here through pull requests; only blockers, decisions and gates go to the owner | Packet revision 2 | Codex reviews on GitHub |
| DEC16 | Build V1 and V2 first, then wearable feasibility; V3 (hosted, wearables, rankings, groups) deferred, not removed. Wearable feasibility no longer blocks V1/V2 architecture. 5 Nov for V1+V2+V3 is no longer committed; V1/V2 dates to be estimated in the architecture proposal | Owner confirmed 20 Oct (decision card) | Supersedes DEC01 and the all-three-by-5-Nov plan; honest schedule |
| DEC17 | Minimum PC target is 8 GB (was 4 GB) | Owner decision 20 Oct (decision card) | No 4 GB PC available; 4 GB not validated |
| DEC18 | Retain default Media Foundation camera capture; forced DirectShow rejected | Batch 1 finding, 20 Oct | DirectShow kept the camera on after stop and ignored Windows camera denial |
| DEC19 | Batch 1 provisionally accepted by Codex on the owner-approved 8 GB Windows 11 laptop, with qualifications: not independently verified, not 4 GB validated, Windows 10 camera unproven, open items in Blockers.md | Codex review 20 Oct | Architecture may proceed; soak test and second webcam before UAT/pilot |

PRD.md is the consolidated proposal. Research notes may discuss alternatives; they do not override the approved requirement baseline.

## Update 21 October 2026

| ID | Decision | Authority / state | Reason |
|---|---|---|---|
| DEC20 | Server restore resets only the absent user; maximum wait 7 days; time never counts as proof (architecture Q9) | Owner decision 21 Oct | Per-user restore rule in architecture revision 5 |
| DEC21 | Prototype approved with three defaults: hide the pet during presentations, optional nine-hour check-in, neutral posture wording | Owner approval 21 Oct | Prototype PR #4 after three revisions |
| DEC22 | Prototype verification gate closed by Codex (independent 24/24 harness on routine logic); V1/V2 build authorised in reviewed slices, starting with V1-0 | Codex review 21 Oct, within owner-approved scope | Pairing/security sign-off still required before V1-5 |
| DEC23 | Architecture PR #3 (revision 5) has Codex conditional design acceptance; not unconditional | Codex review 21 Oct | Pairing security gate open |
