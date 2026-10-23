// The recovery-day plan window (wireframe W15), in Preact. It exists only while open.
// The form edits a draft: the saved plan if one is pending, otherwise a starting plan for
// the next workday. "Save plan" replaces the pending plan (only one is kept), "Discard
// changes" goes back to what is saved, and "Cancel plan" removes the pending plan. The
// engine checks every plan; the recovery morning lists only the items that were saved.
import { render } from 'preact';
import { useEffect, useState } from 'preact/hooks';
import { applyA11y, followA11y } from '../shared/a11y';
import { call, report } from '../shared/ipc';
import type { PlanPage, RecoveryPlan } from '../shared/types';

const WEEKDAYS = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];

/** "2026-10-22" as "Thu 22 Oct". */
function dayName(iso: string): string {
  const [y, m, d] = iso.split('-').map(Number) as [number, number, number];
  const date = new Date(Date.UTC(y, m - 1, d));
  return `${WEEKDAYS[date.getUTCDay()]} ${d} ${MONTHS[m - 1]}`;
}

/** "16:00:00" as "16:00" for a time field. */
const hhmm = (t: string | null) => (t ? t.slice(0, 5) : '');

function same(a: RecoveryPlan, b: RecoveryPlan): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

function App() {
  const [page, setPage] = useState<PlanPage | null>(null);
  const [draft, setDraft] = useState<RecoveryPlan | null>(null);
  // The walk and finish times are kept while their switch is off, to come back on.
  const [walk, setWalk] = useState('');
  const [finish, setFinish] = useState('');
  const [status, setStatus] = useState('');
  const [error, setError] = useState('');

  const adopt = (p: PlanPage) => {
    setPage(p);
    applyA11y(p.a11y);
    const base = p.plan ?? p.draft;
    setDraft(base);
    setWalk(hhmm(base.walk_at ?? p.draft.walk_at) || '16:00');
    setFinish(hhmm(base.finish_at ?? p.draft.finish_at) || '18:30');
  };

  useEffect(() => {
    call('plan_page')
      .then(adopt)
      .catch((e: unknown) => setError(String(e)));
    followA11y();
  }, []);

  if (error) {
    return (
      <div class="shell">
        <p class="warn" role="alert">
          {error} Close the window and open it again to retry.
        </p>
      </div>
    );
  }
  if (!page || !draft) return <div class="shell" aria-busy="true" />;

  const saved = page.plan;
  const base = saved ?? page.draft;
  const changed = !same(draft, base);
  const set = (change: Partial<RecoveryPlan>) => setDraft({ ...draft, ...change });
  const goal = page.water_goal_ml === null ? 'your own goal' : `your own goal (${page.water_goal_ml} ml)`;

  const run = (p: Promise<PlanPage>, done: string) => {
    p.then((v) => {
      adopt(v);
      setStatus(done);
    }).catch((e: unknown) => setStatus(String(e)));
  };

  return (
    <div class="shell plan">
      <header class="top">
        <h1>Recovery day: {dayName(draft.date)}</h1>
      </header>
      <main>
        <p class="note">
          A gentler routine you control. Not a leave request, and not a reason to work longer
          today.
        </p>
        {saved ? (
          <p class="saved">Saved plan for {dayName(saved.date)}. Only one plan is kept; saving again replaces it.</p>
        ) : null}
        <form
          onSubmit={(e) => {
            e.preventDefault();
            run(call('plan_save', { plan: draft }), `Plan saved for ${dayName(draft.date)}.`);
          }}
        >
          <section aria-labelledby="date-h">
            <h2 id="date-h">Date</h2>
            <label class="field">
              <span>Change date</span>
              <select value={draft.date} onChange={(e) => set({ date: e.currentTarget.value })}>
                {page.dates.map((d) => (
                  <option value={d.date} key={d.date}>
                    {dayName(d.date)}
                    {d.workday ? '' : ' (day off)'}
                  </option>
                ))}
              </select>
            </label>
          </section>
          <section aria-labelledby="items-h">
            <h2 id="items-h">Plan items</h2>
            <label class="check block">
              <input
                type="checkbox"
                role="switch"
                checked={draft.more_breaks}
                onChange={(e) => set({ more_breaks: e.currentTarget.checked })}
              />
              <span>Short movement and screen-free breaks offered a little more often</span>
            </label>
            <div class="plan-item">
              <label class="check">
                <input
                  type="checkbox"
                  role="switch"
                  checked={draft.walk_at !== null}
                  onChange={(e) =>
                    e.currentTarget.checked
                      ? set({ walk_at: `${walk}:00` })
                      : set({ walk_at: null, walk_repeat: false })
                  }
                />
                Optional comfortable 10 to 15 minute walk at
              </label>
              <input
                type="time"
                aria-label="Walk time"
                value={walk}
                disabled={draft.walk_at === null}
                onInput={(e) => {
                  const v = e.currentTarget.value;
                  setWalk(v);
                  if (v) set({ walk_at: `${v}:00` });
                }}
              />
              <label class="check">
                <input
                  type="checkbox"
                  checked={draft.walk_repeat}
                  disabled={draft.walk_at === null}
                  onChange={(e) => set({ walk_repeat: e.currentTarget.checked })}
                />
                repeat every 90 min
              </label>
            </div>
            <label class="check block">
              <input
                type="checkbox"
                role="switch"
                checked={draft.lunch_away}
                onChange={(e) => set({ lunch_away: e.currentTarget.checked })}
              />
              <span>Lunch away from the screen, if practical</span>
            </label>
            <label class="check block">
              <input
                type="checkbox"
                role="switch"
                checked={draft.calm_pause}
                onChange={(e) => set({ calm_pause: e.currentTarget.checked })}
              />
              <span>One brief calming pause</span>
            </label>
            <div class="plan-item">
              <label class="check">
                <input
                  type="checkbox"
                  role="switch"
                  checked={draft.finish_at !== null}
                  onChange={(e) => set({ finish_at: e.currentTarget.checked ? `${finish}:00` : null })}
                />
                Finish work around
              </label>
              <input
                type="time"
                aria-label="Finish time"
                value={finish}
                disabled={draft.finish_at === null}
                onInput={(e) => {
                  const v = e.currentTarget.value;
                  setFinish(v);
                  if (v) set({ finish_at: `${v}:00` });
                }}
              />
            </div>
            <label class="check block">
              <input
                type="checkbox"
                role="switch"
                checked={draft.reflection}
                onChange={(e) => set({ reflection: e.currentTarget.checked })}
              />
              <span>One end-of-day question: "Did this routine feel helpful?"</span>
            </label>
            <p class="note">
              Water stays at {goal}, with no catch-up. Today's normal movement routine is reused;
              nothing extra is owed.
            </p>
          </section>
          <div class="actions">
            <button type="submit" class="btn primary" disabled={saved !== null && !changed}>
              Save plan
            </button>
            <button
              type="button"
              class="btn"
              disabled={!changed}
              onClick={() => {
                adopt(page);
                setStatus(saved ? 'Back to the saved plan.' : 'Back to the starting plan.');
              }}
            >
              Discard changes
            </button>
            {saved ? (
              <button
                type="button"
                class="btn link"
                onClick={() => run(call('plan_cancel'), 'Plan cancelled. Nothing changes on that day.')}
              >
                Cancel plan
              </button>
            ) : null}
          </div>
        </form>
        <p class="status" role="status">
          {status}
        </p>
      </main>
    </div>
  );
}

const root = document.getElementById('app');
if (root) {
  render(<App />, root);
}

window.addEventListener('unhandledrejection', (e) => report(e.reason));
