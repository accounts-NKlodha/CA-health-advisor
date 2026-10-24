# V2 meditation pause (slice V2-4)

Status: built, off by default. The content is **provisional, pending the planned content
review** (like the movement routines), and every meditation screen says so.

## What the user sees

- **Settings > Focus & Pomodoro > Meditation pause (W11):** "Invite me once a day" is off
  until switched on; the time (default 15:00) can then be changed. Nothing else to set.
- **Invitation (B12):** at that time on a workday, inside work hours, the pet offers
  "A short pause?" with **2**, **5** (offered first) or **10 minutes**, and **Not today**.
  It is shown at most once a day, only when no card would be held back (not during a
  meeting, presentation, focus, Pomodoro, exercise, pause, lock or away), and after the
  reminders' calm gap. If the time passes during a hold, it comes when the hold ends, the
  same day. Unanswered, it closes quietly after 10 minutes. It is never replayed and
  never carried to the next day.
- **The pause:** five short text parts of equal length (the chosen length divided by
  five), one at a time, with a part counter, **Next** (**Finish** on the last part),
  **Pause** or **Resume**, **Stop**, and a link to the other version: breath-focused, or
  eyes-open with no mention of the breath. No sound, no breathing animation, no health
  claims. While it runs it is the only bubble: reminders and cards wait (an exercise).
- **End:** "Done. Back to work whenever you are ready." closes after a minute or on OK.
  Stop, the bubble's close button, any hold that starts (lock, sleep, coaching paused,
  focus, a presentation, a meeting, the end of work hours) and the app closing end it as
  stopped.

## What is recorded

One `meditation.session` event per started pause: `session_id`, `minutes` (2, 5 or 10),
`version` (`breath` or `eyes_open`), `outcome` (`done` or `stopped`) and `parts_reached`
(1 to 5; 5 when done). It is written only by the user's own action (manual), validated
like every other event, and stays on this PC: it is in no shared summary. An invitation
that is skipped or closes unanswered records nothing. The event type is new text in the
existing `events` table, so no migration is needed.

## Where it lives

| Part | Where |
| --- | --- |
| Content | `crates/hy-core/src/coach/meditation.rs` |
| Rules | `crates/hy-core/src/coach/engine/meditation.rs` (invitation, timers, holds, the event) |
| Setting | `CoachConfig.meditation` (`{"at": "HH:MM" or null}`, defaults to off for saved settings without it), `CoachChange::Meditation` |
| Event | `EventPayload::MeditationSession` in `crates/hy-core/src/schema.rs` |
| Cards | `crates/hy-pet/src/link/present.rs` and `route.rs` |
| Settings page | `apps/companion/ui/src/screens/Coaching.tsx` |

## Tests

- `hy-core` `coach_engine` (10 meditation traces): off until set; once a day with Not
  today and the next day; never starts by itself and closes quietly; meetings, focus and
  a time after work hours; the five parts time out and Done is recorded; Next, Pause,
  Resume, Switch and Stop; Finish and OK; stopped by a lock, a meeting, the close button
  and a restart; a pause running at the end of work hours stops then; the
  state saves and loads (and loads without the field).
- `hy-core` unit tests: the content (five short parts, no breath in the eyes-open
  version), the setting (off by default, whole minutes), the event's rules, and the
  settings change shape.
- `hy-pet` link: every card is valid and every action routes; the invitation's choices
  (5 minutes first, junk lengths refused), the part counter, Finish, Resume, the version
  switch and the last screen.
- Companion: the settings shapes the page sends are accepted.

## Needs physical testing

None beyond the general V1 checks: the pause is text on the pet's bubble and is covered
by the tests above. The content review is a release-checklist item.

## Decisions to confirm

1. **Invitation only.** The pause starts from the daily invitation. There is no pet-menu
   entry to start one at any other time in V2-4.
2. **Default time.** The time field starts at 15:00 when the invitation is switched on.
3. **The end of work hours stops a running pause**, like the other holds (the parts
   reached are recorded).
