# Discovery and design backlog

Draft 18 October 2026. This is a gate/work register, not an authorised coding plan.

| ID | Work | Owner | State / dependency | Required evidence |
|---|---|---|---|---|
| D01 | Consolidate scope and PRD | Codex | Draft ready | PRD with stable IDs and acceptance criteria |
| D02 | Research three device routes | Codex discovery agent | Desk research complete; hardware checks pending | Wearable_Feasibility.md; later physical model/record proof |
| D03 | Confirm execution/credit environment | User + Claude | Pending | Claude code execution/write smoke check, eligible credit balance |
| D04 | Publish planning to user-supplied repository | Codex | Explicit public-disclosure approval received; this revision contains the publication package | Repository commit and verified document tree |
| D05 | Review and approve PRD | User | Pending | Approval recorded in Approvals.md |
| D06 | Architecture/feasibility proposal | Claude | Brief ready; no executable spike authorised | Evidence and explicit unknowns; resource and device plan |
| D07 | Wireframes and simulated clickable prototype | Claude | After written scope review | PRD section 10 flows, ten pets, requirement mapping |
| D08 | Independent design review | Codex agents | Requires prototype | One consolidated review and rubric result |
| D09 | Prototype approval | User | Requires resolved D08 | Explicit approval of named artifact revision |
| D10 | Detailed implementation plan/slices | Claude with Codex acceptance planning | After PRD; implementation after D09 | Dependencies, owners, test and budget allowances |
| D11 | Local deployment/test inventory | User | Three pilot people confirmed; other inputs pending | Server specification, rollout count, tester schedule, device OS/model |

No build task is active. V1, V2 and V3 remain required. Failed feasibility produces an explicit decision, not silent removal from scope.


## Update 20 October 2026

Sequence is V1 and V2 first, then wearable feasibility. V3 is deferred, not removed.

| ID | Work | State |
|---|---|---|
| D06 | V1/V2 architecture proposal (flag unverified assumptions; estimate V1/V2 dates) | Authorised 20 Oct; next |
| D12 | Codex architecture review | After D06 |
| D07, D08, D09 | Wireframes/clickable prototype, design review, Rohit's explicit prototype approval | Not approved; after D12 |
| D10 | Product development in slices | Not approved; after D09 |
| D13 | Batch 1 open items (Blockers.md B06 to B10) | Tracked; soak test and second webcam before UAT/pilot |
| D14 | Wearable feasibility (Apple, Fossil, Samsung; formerly batch 2 and Wearable_Feasibility.md follow-ups) | Deferred until after V1/V2; no research now; batch 2 not authorised |
| D15 | V3: hosted deployment, wearables, rankings, groups | Deferred, not removed |

## Update 21 October 2026

| ID | Work | State |
|---|---|---|
| D06, D12 | V1/V2 architecture proposal and Codex review | Revision 5 (PR #3); conditional design acceptance; pairing security gate open |
| D07, D08, D09 | Prototype, design review, owner approval | Done 21 Oct: PR #4 approved with three defaults; Codex closed the verification gate |
| D10 | Product development in slices | Authorised 21 Oct; V1-0 in progress |
| D16 | Pairing/security sign-off | Required before V1-5 (B11) |
| D17 | Exercise-content review | Pending (B12) |
| D18 | Art review of remaining six pet designs | Pending (B13) |
| D19 | Independent testing, UAT, three-user pilot | Pending (B14); forecast V1 pilot about 19 Nov, V2 pilot about 27 Nov (not a deadline) |
| D14, D15 | Wearable feasibility; V3 | Deferred; V4 also out of scope now |
