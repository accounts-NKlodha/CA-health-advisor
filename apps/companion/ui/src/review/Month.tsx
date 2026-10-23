// Monthly progress (wireframe W14): a rate per eligible day for one metric at a time,
// one bar per month with its count and coverage, the user's first four weeks as a dashed
// baseline, and the selected month day by day. A measured zero is an outline at 0%, a
// month with no data a hatched full-height bar marked "no data", and the month in
// progress a dashed outline. "Show as table" gives every figure as text.
import type { Metric, MonthBar, MonthReview, Ratio } from '../shared/types';
import { Legend } from './Week';
import { METRICS, dayMonth, info, monthName, monthShort, percent, symbol, weekdayIndex, words } from './marks';

const TOP = 26;
const H = 160;
const STEP = 92;
const BAR = 52;

function detail(metric: Metric, r: Ratio): string {
  if (metric === 'breaks') return `${r.num} of ${r.den} ${r.den === 1 ? 'break' : 'breaks'}`;
  return `${r.num} of ${r.den} ${r.den === 1 ? 'day' : 'days'}`;
}

function barKind(b: MonthBar, r: Ratio): 'none' | 'empty' | 'zero' | 'measured' {
  if (b.coverage.recorded === 0) return 'none';
  if (r.den === 0) return 'empty';
  return r.num === 0 ? 'zero' : 'measured';
}

function barValue(b: MonthBar, r: Ratio): string {
  switch (barKind(b, r)) {
    case 'none':
      return '–';
    case 'empty':
      return '–';
    default:
      return `${r.pct ?? 0}%`;
  }
}

function barCaption(metric: Metric, b: MonthBar, r: Ratio): string {
  switch (barKind(b, r)) {
    case 'none':
      return 'no data';
    case 'empty':
      return metric === 'water' ? 'no goal set' : 'nothing to count';
    default:
      return detail(metric, r);
  }
}

function Chart({ m, metric, showBaseline }: { m: MonthReview; metric: Metric; showBaseline: boolean }) {
  const width = 24 + m.bars.length * STEP;
  const base = showBaseline ? m.baseline?.[metric] ?? null : null;
  const baseY = base?.pct != null ? TOP + H - (base.pct * H) / 100 : null;
  const i = info(metric);
  return (
    <svg
      class="chart"
      viewBox={`0 0 ${width} ${TOP + H + 48}`}
      role="img"
      aria-label={`${i.monthly} by month. Use "Show as table" for the values.`}
    >
      <defs>
        <pattern id="bar-hatch" width="8" height="8" patternUnits="userSpaceOnUse" patternTransform="rotate(45)">
          <rect class="paper" width="8" height="8" />
          <rect class="hatch" width="4" height="8" />
        </pattern>
      </defs>
      <line class="axis" x1="12" y1={TOP + H} x2={width - 4} y2={TOP + H} />
      {m.bars.map((b, n) => {
        const r = b[metric];
        const kind = barKind(b, r);
        const x = 20 + n * STEP;
        const h = kind === 'measured' ? Math.max(2, ((r.pct ?? 0) * H) / 100) : kind === 'zero' ? 2 : H;
        const cls = `bar ${kind}${b.in_progress ? ' part' : ''}`;
        return (
          <g key={b.month}>
            <rect class={cls} x={x} y={TOP + H - h} width={BAR} height={h}>
              <title>{`${monthName(b.month)}: ${barValue(b, r)}, ${barCaption(metric, b, r)}${b.in_progress ? ' (month in progress)' : ''}`}</title>
            </rect>
            <text class="bar-num" x={x + BAR / 2} y={TOP + H - h - 6} text-anchor="middle">
              {barValue(b, r)}
            </text>
            <text class="bar-lab" x={x + BAR / 2} y={TOP + H + 18} text-anchor="middle">
              {monthShort(b.month)}
            </text>
            <text class="bar-sub" x={x + BAR / 2} y={TOP + H + 34} text-anchor="middle">
              {barCaption(metric, b, r)}
            </text>
          </g>
        );
      })}
      {baseY !== null && base ? (
        <g class="baseline">
          <line x1="12" y1={baseY} x2={width - 4} y2={baseY} />
          <text x={width - 6} y={baseY - 5} text-anchor="end">
            your first 4 weeks: {base.pct}%
          </text>
        </g>
      ) : null}
    </svg>
  );
}

function Table({ m, metric }: { m: MonthReview; metric: Metric }) {
  const i = info(metric);
  return (
    <table class="t">
      <caption class="sr">{i.monthly} by month</caption>
      <thead>
        <tr>
          <th scope="col">Month</th>
          <th scope="col">{i.monthlyUnit}</th>
          <th scope="col">Detail</th>
          <th scope="col">Workdays recorded</th>
        </tr>
      </thead>
      <tbody>
        {m.bars.map((b) => {
          const r = b[metric];
          const kind = barKind(b, r);
          return (
            <tr key={b.month}>
              <th scope="row">{monthName(b.month)}</th>
              <td>{kind === 'none' ? 'No data' : percent(r)}</td>
              <td>
                {barCaption(metric, b, r)}
                {b.in_progress ? ' (month in progress)' : ''}
              </td>
              <td>
                {b.coverage.recorded} of {b.coverage.workdays}
              </td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}

function Days({ m, metric }: { m: MonthReview; metric: Metric }) {
  const first = m.days[0]?.date;
  const offset = first ? weekdayIndex(first) : 0;
  return (
    <section aria-labelledby="days-h">
      <h3 id="days-h">
        {monthName(m.month)}, day by day ({info(metric).monthly.toLowerCase()})
      </h3>
      <div class="days-head" aria-hidden="true">
        {['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'].map((d) => (
          <span key={d}>{d}</span>
        ))}
      </div>
      <ol class="days">
        {m.days.map((d, n) => {
          const mark = d[metric];
          const start = n === 0 ? ` start-${offset}` : '';
          return (
            <li class={`day mk ${mark}${start}`} key={d.date} title={`${dayMonth(d.date)}: ${words(mark, metric)}`}>
              <span class="dn" aria-hidden="true">
                {Number(d.date.slice(8))}
              </span>
              <span class="sym" aria-hidden="true">
                {symbol(mark)}
              </span>
              <span class="sr">
                {dayMonth(d.date)}: {words(mark, metric)}
              </span>
            </li>
          );
        })}
      </ol>
      <Legend />
    </section>
  );
}

export function MonthView({
  m,
  metric,
  plain,
  showBaseline,
  onMetric,
  onPlain,
  onBaseline,
  onMonth,
}: {
  m: MonthReview;
  metric: Metric;
  plain: boolean;
  showBaseline: boolean;
  onMetric: (metric: Metric) => void;
  onPlain: () => void;
  onBaseline: () => void;
  onMonth: (month: string | null) => void;
}) {
  const i = info(metric);
  const early = m.status === 'too_early';
  return (
    <div class="month">
      <div class="month-head">
        <h2>Monthly progress</h2>
        <nav class="pager" aria-label="Months">
          <button
            type="button"
            class="btn"
            disabled={m.previous_month === null}
            onClick={() => onMonth(m.previous_month)}
          >
            Previous month
          </button>
          <span class="pager-label">{monthName(m.month)}</span>
          <button
            type="button"
            class="btn"
            disabled={m.next_month === null}
            onClick={() => onMonth(m.next_month)}
          >
            Next month
          </button>
        </nav>
      </div>
      <div class="seg chips" role="group" aria-label="Measure">
        {METRICS.map((x) => (
          <button
            type="button"
            class="seg-btn"
            key={x.metric}
            aria-pressed={x.metric === metric}
            onClick={() => onMetric(x.metric)}
          >
            {x.monthly}
          </button>
        ))}
      </div>
      <p class="note">
        {i.monthlyUnit}. Rates count only days the PC recorded, so longer months don't look better
        and missing days are not misses. Each month is judged against the goal you had then.
      </p>
      {early ? (
        <p class="callout">
          Not enough data yet. Your first monthly chart appears after two weeks of use. Days you used
          the app: {m.days_used}.
        </p>
      ) : (
        <>
          {plain ? <Table m={m} metric={metric} /> : <Chart m={m} metric={metric} showBaseline={showBaseline} />}
          {plain ? null : (
            <ul class="legend">
              <li>
                <span class="key measured" aria-hidden="true" />
                measured
              </li>
              <li>
                <span class="key zero" aria-hidden="true" />
                measured zero
              </li>
              <li>
                <span class="key none" aria-hidden="true" />
                no data
              </li>
              <li>
                <span class="key part" aria-hidden="true" />
                month in progress
              </li>
              {m.baseline ? (
                <li>
                  <span class="key base" aria-hidden="true" />
                  baseline (your first 4 weeks)
                </li>
              ) : null}
            </ul>
          )}
          <div class="row">
            <button type="button" class="btn" aria-pressed={plain} onClick={onPlain}>
              {plain ? 'Show as chart' : 'Show as table'}
            </button>
            {m.baseline && !plain ? (
              <button type="button" class="btn" aria-pressed={showBaseline} onClick={onBaseline}>
                {showBaseline ? 'Hide baseline' : 'Show baseline'}
              </button>
            ) : null}
          </div>
        </>
      )}
      <Days m={m} metric={metric} />
    </div>
  );
}
