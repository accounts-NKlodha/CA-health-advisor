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
| DEC12 | Pet size range 50% to 150% in steps of 5 (was 75% to 150%); slider and mouse wheel in the keyboard panel, S/M/L/XL kept as quick picks | Owner request 27 October, after an office-PC review | The smallest size was still too big on screen. The `pet_profile` column keeps its frozen 75 floor; sizes below 75 are stored in the `pet_scale_small` setting |
| DEC14 | The memory budgets (V1 companion at most 150 MB; V2 companion and inference at most 350 MB during check-ins) count only the app's own memory: the private working set summed over the process tree, at the median and at the highest sample. The whole-tree working set is still recorded and reported, for information only. The limits themselves are unchanged | Owner decision 28 October 2026 | The 8-hour companion soak's whole-tree working set (median 218 MB, peak 335 MB, falling to about 97 MB late in the run) moved with pages shared with other programs, which Windows trims and re-adds at will; the app's own memory was steady (private working set median about 67 MB, private bytes flat). Under this rule that run passes the 150 MB budget. `Test-Soak.ps1` and `Test-CameraSoak.ps1` apply it |

PRD.md is the consolidated proposal. Research notes may discuss alternatives; they do not override the approved requirement baseline.
