// The weekly review (wireframe W13, redesigned as a visual review): a hero band with the
// week and its coverage, three rings (one segment per workday), a day strip, up to three
// highlights, the phone self-reports with editable labels, and "Show as table" for a
// plain view of every figure. Missing days are drawn hatched with a dash, a measured zero
// as an outline with "0"; neither is ever counted as the other.
import type { ComponentChildren } from 'preact';
import type { DayMark, MetricRow, PhoneEntry, PhoneLabel, Ratio, WeekReview } from '../shared/types';
import {
  clock,
  dayMonth,
  dayName,
  dayShort,
  duration,
  fraction,
  info,
  range,
  symbol,
  words,
} from './marks';

function point(cx: number, cy: number, r: number, deg: number): string {
  const a = (deg * Math.PI) / 180;
  return `${(cx + r * Math.cos(a)).toFixed(2)} ${(cy + r * Math.sin(a)).toFixed(2)}`;
}

function arc(r0: number, r1: number, a0: number, a1: number): string {
  const c = 66;
  return `M${point(c, c, r1, a0)} A${r1} ${r1} 0 0 1 ${point(c, c, r1, a1)} L${point(c, c, r0, a1)} A${r0} ${r0} 0 0 0 ${point(c, c, r0, a0)} Z`;
}

/** One segment per workday (and per other day with something done). */
function ringDays(w: WeekReview, row: MetricRow): { date: string; mark: DayMark }[] {
  return w.days
    .map((d, i) => ({ date: d.date, workday: d.workday, mark: row.marks[i] ?? 'no_data' }))
    .filter((d) => d.workday || d.mark === 'met' || d.mark === 'partly');
}

function sub(row: MetricRow): string {
  const bits: string[] = [];
  if (row.partly) bits.push(`${row.partly} partly`);
  if (row.zero) bits.push(`${row.zero} at 0`);
  if (row.no_data) bits.push(`${row.no_data} no data`);
  if (row.metric === 'breaks' && row.nothing_due) bits.push(`${row.nothing_due} with none offered`);
  return bits.join(' · ') || (row.value.den > 0 ? 'every recorded day' : '');
}

function centre(row: MetricRow): { num: string; unit: string } {
  const i = info(row.metric);
  if (row.value.den > 0) return { num: fraction(row.value), unit: i.unit };
  if (row.metric === 'water' && row.zero + row.partly > 0) return { num: '–', unit: 'no goal set' };
  return { num: '–', unit: 'nothing yet' };
}

function Ring({ w, row, last }: { w: WeekReview; row: MetricRow; last: Ratio | null }) {
  const i = info(row.metric);
  const days = ringDays(w, row);
  const span = days.length > 0 ? 360 / days.length : 360;
  const c = centre(row);
  const label = `${i.label}: ${c.num} ${c.unit}. ${sub(row) || 'No days yet'}.`;
  return (
    <div class={`ring-card ${row.metric}`}>
      <svg class="ring" viewBox="0 0 132 132" role="img" aria-label={label}>
        <defs>
          <pattern
            id={`partly-${row.metric}`}
            width="7"
            height="7"
            patternUnits="userSpaceOnUse"
            patternTransform="rotate(-45)"
          >
            <rect class="paper" width="7" height="7" />
            <rect class="stripe" width="4" height="7" />
          </pattern>
          <pattern
            id={`hatch-${row.metric}`}
            width="6"
            height="6"
            patternUnits="userSpaceOnUse"
            patternTransform="rotate(45)"
          >
            <rect class="paper" width="6" height="6" />
            <rect class="hatch" width="2.6" height="6" />
          </pattern>
        </defs>
        {days.map((d, n) => {
          const a0 = -90 + n * span + 2.5;
          const a1 = a0 + span - 5;
          return (
            <path class={`seg ${d.mark}`} d={arc(45, 61, a0, a1)} key={d.date}>
              <title>{`${dayName(d.date)}: ${words(d.mark, row.metric)}`}</title>
            </path>
          );
        })}
        <text class="ring-num" x="66" y="70" text-anchor="middle">
          {c.num}
        </text>
        <text class="ring-unit" x="66" y="86" text-anchor="middle">
          {c.unit}
        </text>
      </svg>
      <div class="ring-text">
        <div class="ring-name">{i.label}</div>
        <div class="ring-sub">{sub(row)}</div>
        {last ? <div class="ring-last">Last week: {fraction(last)}</div> : null}
      </div>
    </div>
  );
}

function Strip({ w }: { w: WeekReview }) {
  return (
    <table class="strip">
      <caption class="sr">The week day by day</caption>
      <thead>
        <tr>
          <td />
          {w.days.map((d) => (
            <th scope="col" key={d.date} class={d.workday ? '' : 'non-work'}>
              {dayShort(d.date)}
              <span class="sr"> {dayMonth(d.date)}</span>
            </th>
          ))}
        </tr>
      </thead>
      <tbody>
        {w.rows.map((r) => (
          <tr class={r.metric} key={r.metric}>
            <th scope="row">{info(r.metric).short}</th>
            {r.marks.map((m, n) => {
              const date = w.days[n]?.date ?? '';
              return (
                <td class={`mk ${m}`} key={date} title={`${dayName(date)}: ${words(m, r.metric)}`}>
                  <span aria-hidden="true">{symbol(m)}</span>
                  <span class="sr">{words(m, r.metric)}</span>
                </td>
              );
            })}
          </tr>
        ))}
      </tbody>
    </table>
  );
}

export function Legend() {
  const items: [DayMark, string][] = [
    ['met', 'done or goal met'],
    ['partly', 'partly'],
    ['zero', '0: none recorded (app running)'],
    ['no_data', 'no data: app not running'],
    ['nothing_due', 'no break offered'],
    ['off', 'day off'],
    ['today', 'not started, today or later'],
  ];
  return (
    <ul class="legend">
      {items.map(([m, text]) => (
        <li key={m}>
          <span class={`mk ${m}`} aria-hidden="true">
            {symbol(m)}
          </span>
          {text}
        </li>
      ))}
    </ul>
  );
}

function Plain({ w }: { w: WeekReview }) {
  return (
    <div class="plain">
      <table class="t">
        <caption class="sr">The week as a table</caption>
        <thead>
          <tr>
            <th scope="col">Activity</th>
            {w.days.map((d) => (
              <th scope="col" key={d.date}>
                {dayShort(d.date)} {dayMonth(d.date)}
              </th>
            ))}
            <th scope="col">Summary</th>
          </tr>
        </thead>
        <tbody>
          {w.rows.map((r) => {
            const i = info(r.metric);
            const c = centre(r);
            return (
              <tr key={r.metric}>
                <th scope="row">{i.label}</th>
                {r.marks.map((m, n) => (
                  <td key={w.days[n]?.date ?? n}>{words(m, r.metric)}</td>
                ))}
                <td>
                  {c.num} {c.unit}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
      <p class="note">
        "0" means the app was running and nothing was logged or done. "No data" means the app was
        not running; those days are not counted as misses.
      </p>
    </div>
  );
}

const LABELS: [PhoneLabel, string][] = [
  ['work', 'Work'],
  ['personal', 'Personal'],
  ['unsure', 'Unsure'],
];

function Phone({
  w,
  onLabel,
}: {
  w: WeekReview;
  onLabel: (e: PhoneEntry, label: PhoneLabel) => void;
}) {
  const c = w.phone.counts;
  const total = c.work + c.personal + c.unsure + c.unlabelled;
  const parts = [
    c.work ? `${c.work} work` : '',
    c.personal ? `${c.personal} personal` : '',
    c.unsure ? `${c.unsure} unsure` : '',
    c.unlabelled ? `${c.unlabelled} without a label` : '',
  ].filter(Boolean);
  return (
    <section aria-labelledby="phone-h">
      <h3 id="phone-h">
        Phone interruptions <span class="muted">(self-reported, not detected)</span>
      </h3>
      {total === 0 ? (
        <p class="note">None noted this week.</p>
      ) : (
        <>
          <p>
            {total} noted: {parts.join(', ')}.
          </p>
          <table class="t phone">
            <caption class="sr">Phone interruptions this week, with their labels</caption>
            <tbody>
              {w.phone.entries.map((e) => (
                <tr key={e.id}>
                  <th scope="row">
                    {dayShort(e.date)} {dayMonth(e.date)}, {clock(e.time)}
                  </th>
                  <td>
                    <div class="seg" role="group" aria-label="Label">
                      {LABELS.map(([l, text]) => (
                        <button
                          type="button"
                          class="seg-btn"
                          key={l}
                          aria-pressed={e.label === l}
                          onClick={() => {
                            if (e.label !== l) onLabel(e, l);
                          }}
                        >
                          {text}
                        </button>
                      ))}
                    </div>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          <p class="note">Change a label at any time; the note keeps its day and time.</p>
        </>
      )}
    </section>
  );
}

const TITLES: Record<WeekReview['status'], string> = {
  normal: 'Your week',
  first_week: 'Your first week',
  not_enough_data: 'Not enough data yet',
  before_start: 'No data for this week',
};

function coverageLine(w: WeekReview): string {
  const c = w.coverage;
  if (w.status === 'first_week') {
    const first = w.days.find((_d, n) => w.rows[0]?.marks[n] !== 'not_started');
    const since = first ? ` (started ${dayName(first.date)})` : '';
    return `${c.recorded} ${c.recorded === 1 ? 'day' : 'days'} recorded so far${since}`;
  }
  const missing = c.workdays - c.recorded;
  const tail = missing > 0 ? ` (app not running on ${missing})` : '';
  return `Recorded on ${c.recorded} of ${c.workdays} workdays${tail}`;
}

export function WeekView({
  w,
  plain,
  onPlain,
  onWeek,
  onLabel,
}: {
  w: WeekReview;
  plain: boolean;
  onPlain: () => void;
  onWeek: (day: string | null) => void;
  onLabel: (e: PhoneEntry, label: PhoneLabel) => void;
}) {
  const cmp = w.comparison;
  const last = (r: MetricRow): Ratio | null => (cmp ? cmp[r.metric] : null);
  const thisWeek = w.next_week === null;
  let note: ComponentChildren = null;
  if (w.status === 'not_enough_data') {
    note = 'Not enough data for conclusions this week. Missing days are not counted as misses.';
  } else if (w.status === 'first_week') {
    note = 'Comparisons start after two full weeks.';
  } else if (w.status === 'before_start') {
    note = 'You had not started using HealthierYou in this week.';
  }
  return (
    <div class="week">
      <div class={`hero ${w.status}`}>
        <div>
          <div class="kicker">Week of {range(w.start, w.end)}</div>
          <h2 class="hero-title">{TITLES[w.status]}</h2>
          <p class="hero-cov">
            {coverageLine(w)}. Figures come only from what you logged or the PC recorded.
          </p>
        </div>
        <div class="hero-tools">
          <button type="button" class="btn" aria-pressed={plain} onClick={onPlain}>
            {plain ? 'Show as graphics' : 'Show as table'}
          </button>
        </div>
      </div>

      <nav class="pager" aria-label="Weeks">
        <button
          type="button"
          class="btn"
          disabled={w.previous_week === null}
          onClick={() => onWeek(w.previous_week)}
        >
          Previous week
        </button>
        <button type="button" class="btn" disabled={thisWeek} onClick={() => onWeek(w.next_week)}>
          Next week
        </button>
        {thisWeek ? null : (
          <button type="button" class="btn" onClick={() => onWeek(null)}>
            This week
          </button>
        )}
      </nav>

      {note ? <p class="callout">{note}</p> : null}

      {plain ? (
        <Plain w={w} />
      ) : (
        <>
          <div class="rings">
            {w.rows.map((r) => (
              <Ring w={w} row={r} last={last(r)} key={r.metric} />
            ))}
          </div>
          <Strip w={w} />
          <Legend />
        </>
      )}

      {w.highlights.length > 0 ? (
        <section aria-labelledby="hi-h">
          <h3 id="hi-h">Highlights</h3>
          <ul class="highlights">
            {w.highlights.map((h) => (
              <li key={h}>{h}</li>
            ))}
          </ul>
          {cmp?.longest_block_s ? (
            <p class="note">Last week's longest work block: {duration(cmp.longest_block_s)} (estimate).</p>
          ) : null}
        </section>
      ) : null}

      {w.phone.show ? <Phone w={w} onLabel={onLabel} /> : null}

      <details class="how">
        <summary>How these are worked out</summary>
        <ul>
          <li>
            Movement routine: days a routine was marked complete, out of the days this PC recorded.
            Partly done routines are shown but not counted as complete.
          </li>
          <li>
            Breaks taken: movement and eye-rest reminders you marked done or started, out of those
            you answered or left unanswered. A snooze is not counted, because the reminder comes
            back.
          </li>
          <li>
            Water goal: days your own goal was reached, out of recorded days with a goal. Each day
            is judged against the goal you had then.
          </li>
          <li>
            Today counts once it is over, or as soon as it is done. Work blocks and focus counts
            are estimates from this PC's keyboard and mouse activity, never from what you type.
          </li>
          <li>Your review stays on this PC.</li>
        </ul>
      </details>
    </div>
  );
}
