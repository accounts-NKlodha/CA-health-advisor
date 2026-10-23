// Words, symbols and dates shared by the weekly review and monthly progress (wireframes
// W13 and W14). Every mark has a symbol and words, so no status depends on colour alone;
// "0" (recorded, none logged) and "no data" (app not running) are never drawn alike.
import type { DayMark, Metric, Ratio } from '../shared/types';

const DAYS = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];
const DAY_NAMES = ['Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday', 'Sunday'];
const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
const MONTH_NAMES = [
  'January',
  'February',
  'March',
  'April',
  'May',
  'June',
  'July',
  'August',
  'September',
  'October',
  'November',
  'December',
];

/** A "YYYY-MM-DD" date as numbers, without any time zone conversion. */
export function parts(date: string): { y: number; m: number; d: number } {
  const [y = 0, m = 1, d = 1] = date.split('-').map(Number);
  return { y, m, d };
}

/** 0 for Monday to 6 for Sunday (the date is a calendar date, not an instant). */
export function weekdayIndex(date: string): number {
  const { y, m, d } = parts(date);
  const js = new Date(Date.UTC(y, m - 1, d)).getUTCDay();
  return (js + 6) % 7;
}

export function dayShort(date: string): string {
  return DAYS[weekdayIndex(date)] ?? '';
}

export function dayName(date: string): string {
  return DAY_NAMES[weekdayIndex(date)] ?? '';
}

/** "19 Oct". */
export function dayMonth(date: string): string {
  const { m, d } = parts(date);
  return `${d} ${MONTHS[m - 1] ?? ''}`;
}

/** "19 to 25 Oct" or "28 Sep to 4 Oct". */
export function range(start: string, end: string): string {
  const a = parts(start);
  const b = parts(end);
  return a.m === b.m ? `${a.d} to ${dayMonth(end)}` : `${dayMonth(start)} to ${dayMonth(end)}`;
}

/** "October 2026" from "2026-10". */
export function monthName(month: string): string {
  const [y = '', m = '1'] = month.split('-');
  return `${MONTH_NAMES[Number(m) - 1] ?? ''} ${y}`;
}

/** "Oct" from "2026-10". */
export function monthShort(month: string): string {
  const [, m = '1'] = month.split('-');
  return MONTHS[Number(m) - 1] ?? '';
}

/** "11:52" from "11:52:00". */
export function clock(time: string): string {
  return time.slice(0, 5);
}

/** "45 min" or "1 h 55 min". */
export function duration(secs: number): string {
  const mins = Math.floor(secs / 60);
  const h = Math.floor(mins / 60);
  const m = mins % 60;
  if (h === 0) return `${m} min`;
  return m === 0 ? `${h} h` : `${h} h ${m} min`;
}

export interface MetricInfo {
  metric: Metric;
  /** Ring and row label. */
  label: string;
  /** Short row heading. */
  short: string;
  /** What the ring's number counts. */
  unit: string;
  /** Monthly chip. */
  monthly: string;
  /** Monthly unit line. */
  monthlyUnit: string;
}

export const METRICS: MetricInfo[] = [
  {
    metric: 'movement',
    label: 'Movement routine',
    short: 'Movement',
    unit: 'days done',
    monthly: 'Routines completed',
    monthlyUnit: '% of recorded days',
  },
  {
    metric: 'breaks',
    label: 'Breaks taken',
    short: 'Breaks',
    unit: 'breaks taken',
    monthly: 'Break follow-through',
    monthlyUnit: '% of offered breaks',
  },
  {
    metric: 'water',
    label: 'Water goal',
    short: 'Water',
    unit: 'goal days',
    monthly: 'Water goal days',
    monthlyUnit: '% of recorded days with a goal',
  },
];

export function info(metric: Metric): MetricInfo {
  return METRICS.find((m) => m.metric === metric) ?? (METRICS[0] as MetricInfo);
}

/** The symbol drawn in a day cell. */
export function symbol(mark: DayMark): string {
  switch (mark) {
    case 'met':
      return '✓';
    case 'partly':
      return '◐';
    case 'zero':
      return '0';
    case 'nothing_due':
      return '○';
    case 'no_data':
      return '–';
    case 'off':
      return 'off';
    case 'not_started':
    case 'today':
    case 'future':
      return '·';
  }
}

/** The words for a mark, for tooltips, screen readers and the table view. */
export function words(mark: DayMark, metric: Metric): string {
  switch (mark) {
    case 'met':
      return metric === 'water' ? 'goal met' : metric === 'breaks' ? 'every break taken' : 'done';
    case 'partly':
      return metric === 'water' ? 'some logged' : metric === 'breaks' ? 'some taken' : 'partly done';
    case 'zero':
      return '0, none recorded';
    case 'nothing_due':
      return 'no break offered';
    case 'no_data':
      return 'no data (app not running)';
    case 'off':
      return 'day off';
    case 'not_started':
      return 'before you started';
    case 'today':
      return 'today, in progress';
    case 'future':
      return 'later this week';
  }
}

/** "4/6", or a dash when there is nothing to count. */
export function fraction(r: Ratio): string {
  return r.den === 0 ? '–' : `${r.num}/${r.den}`;
}

/** "67%" or "no data". */
export function percent(r: Ratio): string {
  return r.pct === null ? 'nothing to count' : `${r.pct}%`;
}

/** Whether the page may animate: the in-app switch and Windows' reduced-motion setting. */
export function mayAnimate(motion: boolean): boolean {
  return motion && !window.matchMedia('(prefers-reduced-motion: reduce)').matches;
}
