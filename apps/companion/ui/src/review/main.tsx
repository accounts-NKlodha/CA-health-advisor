// The review window (wireframes W13 and W14), in Preact. It exists only while open: the
// window is created on demand and destroyed on close (A7). It reads the weekly review and
// monthly progress from the shell, which builds them from the store's per-day tallies and
// events, and changes nothing except the label of a past phone report and the settings a
// suggestion applies (or undoes).
import { render } from 'preact';
import { useCallback, useEffect, useState } from 'preact/hooks';
import { applyA11y, followA11y } from '../shared/a11y';
import { call, report } from '../shared/ipc';
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
import { WeekView } from './Week';

type Tab = 'week' | 'month';

function App() {
  const [tab, setTab] = useState<Tab>('week');
  const [day, setDay] = useState<string | null>(null);
  const [month, setMonth] = useState<string | null>(null);
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

  useEffect(() => {
    if (tab === 'week') loadWeek(day);
    else loadMonth(month);
  }, [tab, day, month, loadWeek, loadMonth]);

  useEffect(() => followA11y(), []);

  // Coming back to the window shows anything logged meanwhile.
  useEffect(() => {
    const again = () => {
      if (tab === 'week') loadWeek(day);
      else loadMonth(month);
    };
    window.addEventListener('focus', again);
    return () => window.removeEventListener('focus', again);
  }, [tab, day, month, loadWeek, loadMonth]);

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
              setTab('week');
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
              setTab('month');
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
              setDay(d);
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
            onMonth={setMonth}
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
