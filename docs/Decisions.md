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
| DEC13 | Water pace follows the time at the PC, not the clock: the amount expected by now is the goal times the time the user was at the PC (app running, session unlocked, PC awake) inside today's work window, divided by the window's length, rounded to 10 ml and capped at the goal. Locked, asleep, PC off and app not running add nothing, so after an unlock or a start the pace goes on from where it stopped. The 100 ml behind rule and the goal-reached pause are unchanged. Screen time (the same presence at any hour) is shown on its own: today's in the pet menu and keyboard panel, each day's in the weekly review and monthly progress | Owner request 28 October | After a long lock the app asked for a 600 ml backlog two minutes after the unlock, because the old pace spread the goal over the window by clock time. The time at the PC is kept with the day's tallies in the saved engine state, so it survives a restart the same day; each day's screen time is stored in `activity_day.screen_s` (migration 0007, additive, nullable: older rows are no data) because the reviews read up to a month of days |

PRD.md is the consolidated proposal. Research notes may discuss alternatives; they do not override the approved requirement baseline.
