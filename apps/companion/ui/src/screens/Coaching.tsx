// Settings > Reminders (W4), Movement programme (W6) and Focus and Pomodoro (W7). Each
// change is saved at once as a whole coaching configuration; the engine follows on its
// next step.
import { call } from '../shared/ipc';
import type { CoachConfig, Intensity, MovementVersion, SettingsPage } from '../shared/types';
import type { Save } from './main';

interface TabProps {
  page: SettingsPage;
  save: Save;
}

function useCoach({ page, save }: TabProps) {
  return (change: Partial<CoachConfig>, done: string) =>
    save(call('save_coach_settings', { config: { ...page.coach, ...change } }), done);
}

function Switch({
  checked,
  onChange,
  children,
}: {
  checked: boolean;
  onChange: (on: boolean) => void;
  children: preact.ComponentChildren;
}) {
  return (
    <label class="check block">
      <input
        type="checkbox"
        role="switch"
        checked={checked}
        onChange={(e) => onChange(e.currentTarget.checked)}
      />
      <span>{children}</span>
    </label>
  );
}

const INTENSITIES: [Intensity, string][] = [
  ['light', 'Light'],
  ['balanced', 'Balanced'],
  ['more_often', 'More often'],
];

export function RemindersTab(props: TabProps) {
  const c = props.page.coach;
  const coach = useCoach(props);
  const [start, end] = [props.page.schedule.start, props.page.schedule.end];
  return (
    <main class="pane">
      <h2>Reminders</h2>
      <section aria-labelledby="int-h">
        <h3 id="int-h">Intensity</h3>
        <div class="seg" role="group" aria-labelledby="int-h">
          {INTENSITIES.map(([value, label]) => (
            <button
              type="button"
              class="seg-btn"
              key={value}
              aria-pressed={c.intensity === value}
              onClick={() => coach({ intensity: value }, `Intensity: ${label}.`)}
            >
              {label}
            </button>
          ))}
        </div>
        <p class="note">
          Balanced: movement after 45 min of active work, eye rest every 20 min, water only when
          you are behind your own pace. One reminder at a time, with at least a 5-minute calm gap
          before the next one; snooze 10 min. These are starting values to test, not medical
          advice.
        </p>
      </section>

      <section aria-labelledby="cat-h">
        <h3 id="cat-h">What I remind you about</h3>
        <Switch
          checked={c.categories.movement}
          onChange={(on) =>
            coach({ categories: { ...c.categories, movement: on } }, on ? 'Movement breaks on.' : 'Movement breaks off.')
          }
        >
          Movement breaks
        </Switch>
        <Switch
          checked={c.categories.eye_rest}
          onChange={(on) =>
            coach({ categories: { ...c.categories, eye_rest: on } }, on ? 'Eye rest on.' : 'Eye rest off.')
          }
        >
          Eye rest (look away)
        </Switch>
        <Switch
          checked={c.categories.water}
          onChange={(on) =>
            coach({ categories: { ...c.categories, water: on } }, on ? 'Water pacing on.' : 'Water pacing off.')
          }
        >
          Water pacing
        </Switch>
      </section>

      <section aria-labelledby="quiet-h">
        <h3 id="quiet-h">When I stay quiet</h3>
        <label class="check block">
          <input type="checkbox" role="switch" checked disabled />
          <span>
            Outside work hours ({start} to {end}) and on days off <span class="note">(always on)</span>
          </span>
        </label>
        <Switch
          checked={c.hide_during_presentation}
          onChange={(on) =>
            coach(
              { hide_during_presentation: on },
              on ? 'Hidden during presentations.' : 'Shown during presentations.',
            )
          }
        >
          During presentations and full-screen apps: the pet and every bubble are hidden, the
          tray icon stays, and nothing is replayed afterwards.{' '}
          <span class="note">(automatic; may not detect every meeting)</span>
        </Switch>
        <Switch
          checked={c.media_in_use_hint}
          onChange={(on) =>
            coach({ media_in_use_hint: on }, on ? 'Camera or microphone hint on.' : 'Camera or microphone hint off.')
          }
        >
          When the camera or microphone is in use <span class="note">(experimental, off by default)</span>
        </Switch>
        <p class="note">
          You can always use "I'm in a meeting" on the pet. Reminders missed while quiet are not
          replayed.
        </p>
      </section>

      <section aria-labelledby="long-h">
        <h3 id="long-h">Long-day check-in</h3>
        <Switch
          checked={c.long_day_check_in.enabled}
          onChange={(on) =>
            coach(
              { long_day_check_in: { ...c.long_day_check_in, enabled: on } },
              on ? 'Long-day check-in on.' : 'Long-day check-in off.',
            )
          }
        >
          Offer a deadline-day check-in after a long day of PC activity
        </Switch>
        <label class="inline">
          <span>After about</span>
          <select
            value={String(c.long_day_check_in.hours)}
            disabled={!c.long_day_check_in.enabled}
            onChange={(e) => {
              const hours = Number(e.currentTarget.value);
              coach({ long_day_check_in: { ...c.long_day_check_in, hours } }, `Check-in after about ${hours} hours.`);
            }}
          >
            {[8, 9, 10].map((h) => (
              <option value={String(h)} key={h}>
                {h}
              </option>
            ))}
          </select>
          <span>hours of PC activity today</span>
        </label>
        <p class="note">
          Hours of activity are an estimate from this PC. They are not evidence of stress or
          tiredness. "Deadline day" on the pet menu works either way.
        </p>
      </section>
    </main>
  );
}

const VERSIONS: [MovementVersion, string][] = [
  ['ask', 'Ask each time'],
  ['seated', 'Seated'],
  ['standing', 'Standing'],
];

export function MovementTab(props: TabProps) {
  const { page } = props;
  const coach = useCoach(props);
  return (
    <main class="pane">
      <h2>Movement programme</h2>
      {page.provisional ? <p class="warn">{page.provisional}</p> : null}
      <section aria-labelledby="prog-h">
        <h3 id="prog-h">Monday to Saturday</h3>
        <table class="plain">
          <thead>
            <tr>
              <th scope="col">Day</th>
              <th scope="col">Three movements (about 2 min)</th>
            </tr>
          </thead>
          <tbody>
            {page.programme.map((d) => (
              <tr key={d.day}>
                <th scope="row">{d.day}</th>
                <td>{d.movements.join(' · ')}</td>
              </tr>
            ))}
            <tr>
              <th scope="row">Sun</th>
              <td class="note">Optional. No catch-up for missed days.</td>
            </tr>
          </tbody>
        </table>
      </section>
      <section aria-labelledby="lib-h">
        <h3 id="lib-h">Movement library ({page.library.length} desk-safe movements)</h3>
        <ul class="lib">
          {page.library.map((m) => (
            <li key={m.name}>
              <strong>{m.name}</strong>
              <br />
              <span class="note">
                {m.place} · {m.dose}
                {m.seated ? ` · seated alternative: ${m.seated.toLowerCase()}` : ''}
              </span>
            </li>
          ))}
        </ul>
      </section>
      <section aria-labelledby="ver-h">
        <h3 id="ver-h">Default version</h3>
        <div class="seg" role="group" aria-labelledby="ver-h">
          {VERSIONS.map(([value, label]) => (
            <button
              type="button"
              class="seg-btn"
              key={value}
              aria-pressed={page.coach.movement_version === value}
              onClick={() => coach({ movement_version: value }, `Default version: ${label.toLowerCase()}.`)}
            >
              {label}
            </button>
          ))}
        </div>
        <p class="note">
          With a default, the routine starts that version and offers the other one instead. Every
          movement has Skip and Stop. Completion is self-reported.
        </p>
      </section>
    </main>
  );
}

export function FocusTab(props: TabProps) {
  const c = props.page.coach;
  const coach = useCoach(props);
  return (
    <main class="pane">
      <h2>Focus & Pomodoro</h2>
      <section aria-labelledby="focus-h">
        <h3 id="focus-h">Quiet focus</h3>
        <p>
          Presets 30, 60 or 90 minutes, or your own length, from the pet menu. Reminders pause
          while it runs; nothing is replayed afterwards.
        </p>
      </section>
      <section aria-labelledby="pomo-h">
        <h3 id="pomo-h">Pomodoro</h3>
        <p>Off until you press Start on the pet. Presets 25/5, 50/10 or your own lengths.</p>
        <Switch
          checked={c.pomodoro_auto_start}
          onChange={(on) =>
            coach({ pomodoro_auto_start: on }, on ? 'The next round starts by itself.' : 'The next round waits for you.')
          }
        >
          Start the next round automatically <span class="note">(off by default)</span>
        </Switch>
        <p class="note">
          Locking the PC pauses the timer until you resume. Meetings and presentations pause it
          too.
        </p>
      </section>
    </main>
  );
}
