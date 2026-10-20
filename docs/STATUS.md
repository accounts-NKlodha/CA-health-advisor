# Project status

## Update 20 October 2026

- Direction confirmed by owner (Rohit, 20 Oct, via Codex review on PR #1): complete V1 and V2 first, then wearable feasibility. V3 (hosted, wearables, rankings, groups) is deferred, not removed. Wearable feasibility no longer blocks V1/V2 architecture; no wearable research is being done now.
- Schedule: the original target of V1+V2+V3 by 5 Nov 2026 is no longer committed under this sequence. V1/V2 dates will be estimated in the architecture proposal. 5 Nov is not claimed to be achievable.
- Batch 1 (desktop pet / resource spike): provisionally accepted by Codex on the owner-approved 8 GB Windows 11 laptop. Not independently verified (Codex read only the summary and ledger); not validated on 4 GB. Minimum PC target is now 8 GB (was 4 GB). Windows 10 camera operation remains unproven (camera failed on the i7 Windows 10 desktop, whose USB webcam Windows did not detect). Default Media Foundation capture is retained; forced DirectShow is rejected. Sanitized evidence: docs/evidence/batch1/.
- Open items (tracked in docs/Blockers.md): uninstall leaves a registry entry; display scaling beyond 100%/125% and downloaded-installer SmartScreen/Mark-of-the-Web untested; multi-hour camera soak test and second working webcam needed before UAT/pilot.
- Next gates: V1/V2 architecture proposal (unverified assumptions flagged) -> Codex architecture review -> wireframes/clickable prototype -> Rohit's explicit prototype approval -> product development in slices. Architecture proposal is authorised; the prototype and product development are not approved. Separate purchases need owner approval. Apple enrolment, batch 2 and hosting setup are not authorised.
- Spend (estimates from session meters, not reconciled to balance): batch 1 about $21.2 against a $20 cap (stop at $18), about $1.2 over; coordination test under $1. See docs/Budget.md.
- Coordination test HY-COORD-TEST-1: Codex observed Claude's reply and commit; the follow-up was owner-triggered, not an automatic Codex wakeup. Automatic Codex wakeup remains unconfigured.

## Earlier status (18 October)

Updated 18 October 2026 (revision 2 of the delivery plan).

- Baseline: PRD 0.3 package approved by the owner on 18 October; the repository PRD.md (0.1) is superseded until refreshed.
- Phase: feasibility. Batch 1 (desktop pet on a 4 GB PC, offline install, webcam resources) approved and being built on branch spike/batch1. Batch 2 (wearables, meeting detection) not yet requested.
- Gates in order: feasibility review (22 Oct) -> architecture -> wireframes and clickable prototype -> owner's prototype approval -> product development. Architecture, wireframes and product code are blocked.
- Delivery plan and feasibility packet: docs/Feasibility_Packet.md. Pilot readiness for 1 November is not yet supported by evidence; blockers and options are in section 2.
- Coordination: this repository is the shared record; automatic agent wake-ups are unproven until HY-COORD-TEST-1 passes (docs/Coordination_Test.md).
- No product code, installer for staff, hosted deployment or prototype approval exists.

## Earlier status (17 October)


- Phase: Discovery & PRD draft.
- User PRD approval: pending review of the written artifact.
- Prototype: not yet produced; Claude owns it.
- Product development: blocked by explicit user prototype gate.
- Remote repository: https://github.com/accounts-NKlodha/CA-health-advisor (public). User explicitly authorised uploading this planning package after the public-disclosure question. This document revision is the authorised publication package; see repository history for the publication commit. Local CLI authentication is not required for the connector route.
- Claude handoff: prepared locally; not delivered to an external Claude session. Claude.ai project versus cloud coding access must be verified.
- Target deadline: 5 November 2026, all three versions; feasibility unresolved.
- Wearable model confirmation: Apple Watch Series 8/iPhone 13 confirmed by user; Fossil Gen 5 inferred from 'watch 5'; Galaxy Fit3/4 uncertain; Samsung phones S24 FE and A25 respectively.
- Deployment inputs: three pilot participants confirmed by user; server specification, total rollout count and tester schedule remain pending.
- No code, installer, hosted deployment, prototype approval or completed test run is claimed.
