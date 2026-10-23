// The setup wizard (wireframe W1), in Preact: eight steps, each possible without a
// server. Back keeps earlier choices; Next saves the step's choices through the same
// commands as the settings tabs, so the pet and coaching follow at once. It opens by
// itself on the first start and can be run again from Settings.
import { render } from 'preact';
import { useEffect, useState } from 'preact/hooks';
import { applyA11y, followA11y } from '../shared/a11y';
import { call, report } from '../shared/ipc';
import type { CoachConfig, Intensity, SettingsPage, WeekdayName } from '../shared/types';
import { reminderChanges, servingEdit, workweekChange } from '../screens/changes';
import { PetSettings } from '../screens/PetSettings';

const STEPS = ['Welcome', 'Connect', 'Pet', 'Workweek', 'Water', 'Reminders', 'Sharing', 'Try a reminder'];

const DAYS: [WeekdayName, string][] = [
  ['mon', 'Monday'],
  ['tue', 'Tuesday'],
  ['wed', 'Wednesday'],
  ['thu', 'Thursday'],
  ['fri', 'Friday'],
  ['sat', 'Saturday'],
  ['sun', 'Sunday'],
];

const INTENSITIES: [Intensity, string][] = [
  ['light', 'Light'],
  ['balanced', 'Balanced'],
  ['more_often', 'More often'],
];

/** The choices of steps 4 to 6, kept while moving back and forth. */
interface Draft {
  days: WeekdayName[];
  start: string;
  end: string;
  zone: string;
  reviewDay: WeekdayName;
  reviewAt: string;
  goal: string;
  servings: string;
  coach: CoachConfig;
}

function draftOf(p: SettingsPage): Draft {
  return {
    days: p.schedule.workdays,
    start: p.schedule.start,
    end: p.schedule.end,
    zone: p.zone,
    reviewDay: p.coach.review.day,
    reviewAt: p.coach.review.at,
    goal: p.water_goal_ml === null ? '' : String(p.water_goal_ml),
    servings: p.servings.join(', '),
    coach: p.coach,
  };
}

function parseServings(text: string): number[] | null {
  const parts = text
    .split(',')
    .map((t) => t.trim())
    .filter((t) => t !== '');
  const list = parts.map(Number);
  return list.length > 0 && list.every((n) => Number.isInteger(n)) ? list : null;
}

function App() {
  const [step, setStep] = useState(0);
  const [page, setPage] = useState<SettingsPage | null>(null);
  const [draft, setDraft] = useState<Draft | null>(null);
  const [status, setStatus] = useState('');
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    call('setup_page')
      .then((p) => {
        setPage(p);
        setDraft(draftOf(p));
        applyA11y(p.a11y);
      })
      .catch((e: unknown) => setStatus(String(e)));
    followA11y();
  }, []);

  if (!page || !draft) {
    return (
      <div class="shell setup" aria-busy={status === ''}>
        <p class="status" role="status">
          {status}
        </p>
      </div>
    );
  }

  const set = (change: Partial<Draft>) => setDraft({ ...draft, ...change });

  /** Save the current step's choices; resolves when it is safe to move on. */
  const saveStep = (): Promise<void> => {
    switch (step) {
      // Each step sends only what it changed against what it showed, never a whole copy:
      // Settings or the weekly review may have changed other fields meanwhile.
      case 3: {
        const ordered = DAYS.map(([d]) => d).filter((d) => draft.days.includes(d));
        const change = workweekChange(
          { ...page.schedule, zone: page.zone },
          { workdays: ordered, start: draft.start, end: draft.end, zone: draft.zone },
        );
        const review = page.coach.review;
        const reviewChanged = draft.reviewDay !== review.day || draft.reviewAt !== review.at;
        return (change ? call('save_workweek', { change }) : Promise.resolve(page))
          .then((p) =>
            reviewChanged
              ? call('save_coach_settings', { change: { review: { day: draft.reviewDay, at: draft.reviewAt } } })
              : p,
          )
          .then(setPage);
      }
      case 4: {
        const servings = parseServings(draft.servings);
        if (!servings) return Promise.reject(new Error('Write the serving buttons as numbers, such as 150, 250, 500.'));
        const goal = draft.goal.trim() === '' ? null : Number(draft.goal);
        if (goal !== null && !Number.isInteger(goal)) {
          return Promise.reject(new Error('Write the goal as a whole number of ml, or leave it empty.'));
        }
        const edit = servingEdit(page.servings, servings);
        const goalSave =
          goal !== null && goal !== page.water_goal_ml
            ? call('save_water_settings', { change: { goal } })
            : Promise.resolve(page);
        return goalSave.then((p) => (edit ? call('save_water_settings', { change: edit }) : p)).then(setPage);
      }
      case 5:
        return reminderChanges(page.coach, draft.coach)
          .reduce<Promise<SettingsPage>>(
            (done, change) => done.then(() => call('save_coach_settings', { change })),
            Promise.resolve(page),
          )
          .then(setPage);
      default:
        return Promise.resolve();
    }
  };

  const next = () => {
    setBusy(true);
    saveStep()
      .then(() => {
        setStatus('');
        setStep(step + 1);
      })
      .catch((e: unknown) => setStatus(e instanceof Error ? e.message : String(e)))
      .finally(() => setBusy(false));
  };

  const body = () => {
    switch (step) {
      case 0:
        return (
          <section>
            <h2>Welcome to HealthierYou</h2>
            <p>
              A small pet on your screen that suggests short movement breaks, eye rest and water
              during your work hours, and a weekly review of how it went.
            </p>
            <ul>
              <li>It works on this PC without any server.</li>
              <li>It has no sound and never blocks your keyboard or mouse.</li>
              <li>One reminder at a time; you can snooze, skip or pause at any moment.</li>
              <li>Everything you log stays on this PC unless you later choose to share it.</li>
            </ul>
            <p class="note">These are habits to try, not medical advice.</p>
          </section>
        );
      case 1:
        return (
          <section>
            <h2>Connect (optional)</h2>
            <label class="option">
              <input type="radio" name="connect" checked readOnly />
              <span>
                <strong>Use on this PC only</strong>
                <br />
                Everything works here; nothing is sent anywhere.
              </span>
            </label>
            <label class="option unavailable">
              <input type="radio" name="connect" disabled />
              <span>
                <strong>Pair with the office server</strong>
                <br />
                Comes with the office server in a later update. You can pair then from Settings.
              </span>
            </label>
            <label class="option unavailable">
              <input type="radio" name="connect" disabled />
              <span>
                <strong>Hosted account</strong>
                <br />
                Coming later.
              </span>
            </label>
          </section>
        );
      case 2:
        return <PetSettings />;
      case 3:
        return (
          <section>
            <h2>Workweek</h2>
            <fieldset class="field">
              <legend>Workdays</legend>
              <div class="actions">
                {DAYS.map(([d, label]) => (
                  <label class="check" key={d}>
                    <input
                      type="checkbox"
                      checked={draft.days.includes(d)}
                      onChange={(e) =>
                        set({
                          days: e.currentTarget.checked ? [...draft.days, d] : draft.days.filter((x) => x !== d),
                        })
                      }
                    />
                    {label}
                    {d === 'sun' ? ' (optional)' : ''}
                  </label>
                ))}
              </div>
            </fieldset>
            <div class="inline">
              <label for="s-start">Work hours</label>
              <input id="s-start" type="time" value={draft.start} onInput={(e) => set({ start: e.currentTarget.value })} />
              <label for="s-end">to</label>
              <input id="s-end" type="time" value={draft.end} onInput={(e) => set({ end: e.currentTarget.value })} />
            </div>
            <div class="inline">
              <label for="s-review">Weekly review</label>
              <select
                id="s-review"
                value={draft.reviewDay}
                onChange={(e) => set({ reviewDay: e.currentTarget.value as WeekdayName })}
              >
                {DAYS.map(([d, label]) => (
                  <option value={d} key={d}>
                    {label}
                  </option>
                ))}
              </select>
              <label for="s-review-at">at</label>
              <input
                id="s-review-at"
                type="time"
                value={draft.reviewAt}
                onInput={(e) => set({ reviewAt: e.currentTarget.value })}
              />
            </div>
            <label class="field">
              <span>Time zone</span>
              <select value={draft.zone} onChange={(e) => set({ zone: e.currentTarget.value })}>
                {page.zones.map((z) => (
                  <option value={z} key={z}>
                    {z}
                  </option>
                ))}
              </select>
            </label>
            <p class="note">Reminders are offered only inside these hours on workdays.</p>
          </section>
        );
      case 4:
        return (
          <section>
            <h2>Water</h2>
            <label class="field">
              <span>Daily water goal (ml), optional</span>
              <input
                type="number"
                min={page.goal_min_ml}
                max={page.goal_max_ml}
                value={draft.goal}
                onInput={(e) => set({ goal: e.currentTarget.value })}
              />
            </label>
            <p class="note">
              Your own choice, {page.goal_min_ml} to {page.goal_max_ml} ml. HealthierYou never
              suggests or raises it. You can leave it empty and decide later.
            </p>
            <label class="field">
              <span>Serving buttons (ml)</span>
              <input
                type="text"
                value={draft.servings}
                onInput={(e) => set({ servings: e.currentTarget.value })}
              />
            </label>
            <p class="note">
              Up to {page.max_servings}, separated by commas, each {page.serving_min_ml} to{' '}
              {page.serving_max_ml} ml in steps of 10.
            </p>
          </section>
        );
      case 5:
        return (
          <section>
            <h2>Reminders</h2>
            <div class="seg" role="group" aria-label="Intensity">
              {INTENSITIES.map(([value, label]) => (
                <button
                  type="button"
                  class="seg-btn"
                  key={value}
                  aria-pressed={draft.coach.intensity === value}
                  onClick={() => set({ coach: { ...draft.coach, intensity: value } })}
                >
                  {label}
                </button>
              ))}
            </div>
            <p class="note">
              Balanced: movement after 45 min of active work, eye rest every 20 min, water only
              when you are behind your own pace. Starting values to test, not medical advice.
            </p>
            {(
              [
                ['movement', 'Movement breaks'],
                ['eye_rest', 'Eye rest (look away)'],
                ['water', 'Water pacing'],
              ] as const
            ).map(([key, label]) => (
              <label class="check block" key={key}>
                <input
                  type="checkbox"
                  role="switch"
                  checked={draft.coach.categories[key]}
                  onChange={(e) =>
                    set({
                      coach: {
                        ...draft.coach,
                        categories: { ...draft.coach.categories, [key]: e.currentTarget.checked },
                      },
                    })
                  }
                />
                <span>{label}</span>
              </label>
            ))}
            <p class="note">The pet stays quiet outside work hours and during presentations.</p>
          </section>
        );
      case 6:
        return (
          <section>
            <h2>Sharing</h2>
            <p class="saved-note">Not paired: nothing is shared.</p>
            <p>
              Optional sharing with the office server comes later. Every category starts off, and
              you choose each one yourself after pairing.
            </p>
          </section>
        );
      default:
        return (
          <section>
            <h2>Try a reminder</h2>
            <p>See how a reminder looks beside your pet. It closes by itself.</p>
            <button
              type="button"
              class="btn"
              onClick={() => {
                call('setup_sample').catch(report);
                setStatus('A sample reminder is showing beside your pet.');
              }}
            >
              Show a sample reminder
            </button>
            <p class="note">You can change any of these choices later in Settings.</p>
          </section>
        );
    }
  };

  const last = step === STEPS.length - 1;
  return (
    <div class="shell setup">
      <header class="top">
        <h1>
          Setup: step {step + 1} of {STEPS.length}
        </h1>
        <ol class="steps-list">
          {STEPS.map((s, i) => (
            <li key={s} aria-current={i === step ? 'step' : undefined}>
              {i + 1}. {s}
            </li>
          ))}
        </ol>
      </header>
      <main>{body()}</main>
      <p class="status" role="status">
        {status}
      </p>
      <nav class="wizard-nav" aria-label="Setup steps">
        <button
          type="button"
          class="btn"
          disabled={step === 0 || busy}
          onClick={() => {
            setStatus('');
            setStep(step - 1);
          }}
        >
          Back
        </button>
        {last ? (
          <button
            type="button"
            class="btn primary"
            disabled={busy}
            onClick={() => {
              setBusy(true);
              call('setup_finish')
                .catch((e: unknown) => setStatus(String(e)))
                .finally(() => setBusy(false));
            }}
          >
            Finish
          </button>
        ) : (
          <button type="button" class="btn primary" disabled={busy} onClick={next}>
            Next
          </button>
        )}
      </nav>
    </div>
  );
}

const root = document.getElementById('app');
if (root) {
  render(<App />, root);
}

window.addEventListener('unhandledrejection', (e) => report(e.reason));
