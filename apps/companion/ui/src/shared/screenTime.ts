// Screen time (DEC13): the time the user was at this PC (the app running, the session
// unlocked, the PC awake), shown apart from everything else. Today's figure is in the pet
// menu and the keyboard panel, each day's in the weekly review and monthly progress.
// Pure, so `node --test` can check it. A day without a figure is no data, never zero.

/** "3 h 20 min", "45 min" or "0 min". */
export function screenWords(secs: number): string {
  const mins = Math.floor(secs / 60);
  const h = Math.floor(mins / 60);
  const m = mins % 60;
  if (h === 0) return `${m} min`;
  return m === 0 ? `${h} h` : `${h} h ${m} min`;
}

/** A day cell: "3h 20m", "45m", or "–" with no data. */
export function screenShort(secs: number | null): string {
  if (secs === null) return '–';
  const mins = Math.floor(secs / 60);
  const h = Math.floor(mins / 60);
  const m = mins % 60;
  return h === 0 ? `${m}m` : `${h}h ${String(m).padStart(2, '0')}m`;
}

/** A day in words for a title or a screen reader: "screen time 3 h 20 min" or "screen time: no data". */
export function screenDay(secs: number | null): string {
  return secs === null ? 'screen time: no data' : `screen time ${screenWords(secs)}`;
}

/** Total screen time over the days that have a figure: "21 h 5 min over 5 days", or "No data". */
export function screenTotal(days: { screen_s: number | null }[]): string {
  const known = days.flatMap((d) => (d.screen_s === null ? [] : [d.screen_s]));
  if (known.length === 0) return 'No data';
  const total = known.reduce((a, b) => a + b, 0);
  return `${screenWords(total)} over ${known.length} ${known.length === 1 ? 'day' : 'days'}`;
}

/** The pet menu's line, or `null` before the engine reports today's figure. */
export function screenToday(minutes: number | null): string | null {
  return minutes === null ? null : `Screen time today: ${screenWords(minutes * 60)}`;
}
