// The review window (wireframes W13 and W14), in Preact. It exists only while open: the
// window is created on demand and destroyed on close (A7). It reads the weekly review and
// monthly progress from the shell, which builds them from the store's per-day tallies and
// events, and changes nothing except the label of a past phone report and the settings a
// suggestion applies (or undoes).
import { render } from 'preact';
import { useCallback, useEffect, useReducer, useState } from 'preact/hooks';
import { applyA11y, followA11y } from '../shared/a11y';
import { call, on, report } from '../shared/ipc';
import type {
  Metric,
  MonthReview,
  PhoneEntry,
  PhoneLabel,
  ReviewPage,
  Suggestion,
  SuggestionAnswer,
  WeekReview,
} from '../shared/types';
import { mayAnimate } from './marks';
import { MonthView } from './Month';
import { START, navigate, request } from './nav';
import { WeekView } from './Week';

function App() {
  // The tab, week and month shown, kept here: every load asks for exactly this.
  const [nav, go] = useReducer(navigate, START);
  const { tab, day } = nav;
  // The first load waits until the shell said whether the review offer named a week.
  const [ready, setReady] = useState(false);
  const [week, setWeek] = useState<ReviewPage<WeekReview> | null>(null);
  const [monthly, setMonthly] = useState<ReviewPage<MonthReview> | null>(null);
  const [metric, setMetric] = useState<Metric>('water');
  const [plain, setPlain] = useState(false);
  const [baseline, setBaseline] = useState(true);
  const [status, setStatus] = useState('');
  const [error, setError] = useState('');

  const loadWeek = useCallback((d: string | null) => {
    call('review_week', { day: d })
      .then((p) => {
        setWeek(p);
        applyA11y(p.a11y);
        setError('');
      })
      .catch((e: unknown) => setError(String(e)));
  }, []);

  const loadMonth = useCallback((m: string | null) => {
    call('review_month', { month: m })
      .then((p) => {
        setMonthly(p);
        applyA11y(p.a11y);
        setError('');
      })
      .catch((e: unknown) => setError(String(e)));
  }, []);

  const load = useCallback(() => {
    const r = request(nav);
    if (r.command === 'review_week') loadWeek(r.day);
    else loadMonth(r.month);
  }, [nav, loadWeek, loadMonth]);

  useEffect(() => {
    if (ready) load();
  }, [ready, load]);

  // "Open" on the weekly review offer names a week. The shell keeps it until this page
  // takes it: on load (a new window), and when told (a window already open, on any week
  // or on Monthly progress).
  useEffect(() => {
    const target = () =>
      call('review_target')
        .then((week) => {
          if (week) {
            go({ type: 'target', week });
            setStatus('');
          }
        })
        .catch(report);
    target().finally(() => setReady(true));
    on('hy://review-target', () => {
      void target();
    }).catch(report);
  }, []);

  useEffect(() => followA11y(), []);

  // Coming back to the window shows anything logged meanwhile, on the same week or month.
  useEffect(() => {
    if (!ready) return undefined;
    window.addEventListener('focus', load);
    return () => window.removeEventListener('focus', load);
  }, [ready, load]);

  const relabel = (e: PhoneEntry, label: PhoneLabel) => {
    call('review_phone_label', { week: week?.review.start ?? day, entry: e.id, label })
      .then((p) => {
        setWeek(p);
        setStatus(`Saved as ${label}.`);
      })
      .catch((err: unknown) => {
        setStatus(String(err));
        loadWeek(day);
      });
  };

  const suggest = (s: Suggestion, answer: SuggestionAnswer) => {
    const start = week?.review.start;
    if (!start) return;
    call('review_suggestion', { week: start, kind: s.kind, answer })
      .then((p) => {
        setWeek(p);
        setStatus(
          answer === 'apply'
            ? 'Applied. Undo puts the old setting back.'
            : answer === 'undo'
              ? 'Undone. The old setting is back.'
              : 'Hidden for this week.',
        );
      })
      .catch((err: unknown) => {
        setStatus(String(err));
        loadWeek(day);
      });
  };

  const page = tab === 'week' ? week : monthly;
  const animate = page ? mayAnimate(page.motion) : false;

  return (
    <div class={`shell review${animate ? ' anim' : ''}`}>
      <header class="top">
        <h1>{tab === 'week' ? 'Weekly review' : 'Monthly progress'}</h1>
        <nav class="seg tabs" aria-label="Review">
          <button
            type="button"
            class="seg-btn"
            aria-pressed={tab === 'week'}
            onClick={() => {
              go({ type: 'tab', tab: 'week' });
              setStatus('');
            }}
          >
            Weekly review
          </button>
          <button
            type="button"
            class="seg-btn"
            aria-pressed={tab === 'month'}
            onClick={() => {
              go({ type: 'tab', tab: 'month' });
              setStatus('');
            }}
          >
            Monthly progress
          </button>
        </nav>
      </header>
      {error ? (
        <p class="callout" role="alert">
          {error} Nothing was changed. Close the window and open it again to retry.
        </p>
      ) : null}
      <main aria-busy={page === null}>
        {tab === 'week' && week ? (
          <WeekView
            w={week.review}
            plain={plain}
            onPlain={() => setPlain(!plain)}
            onWeek={(d) => {
              go({ type: 'week', day: d });
              setStatus('');
            }}
            onLabel={relabel}
            onSuggestion={suggest}
          />
        ) : null}
        {tab === 'month' && monthly ? (
          <MonthView
            m={monthly.review}
            metric={metric}
            plain={plain}
            showBaseline={baseline}
            onMetric={setMetric}
            onPlain={() => setPlain(!plain)}
            onBaseline={() => setBaseline(!baseline)}
            onMonth={(m) => go({ type: 'month', month: m })}
          />
        ) : null}
      </main>
      <p class="status" role="status">
        {status}
      </p>
    </div>
  );
}

const root = document.getElementById('app');
if (root) {
  render(<App />, root);
}

// Errors from the shell are shown on the page; anything else is logged.
window.addEventListener('unhandledrejection', (e) => report(e.reason));
