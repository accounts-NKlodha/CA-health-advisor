// The combined 5-minute break (DEC15): the countdown its step cards run on the page, and
// the words Settings > Reminders uses for the reminder style. Pure functions (no imports
// at run time), tested with node:test in ui/test.
import type { CoachConfig } from './types';

/** The break interval choices, in minutes of time at the PC. */
export const BREAK_MINUTES = [20, 30, 45, 60] as const;

/** Whole seconds left of a countdown of `seconds` drawn at `drawnMs`, at `nowMs`. */
export function secondsLeft(seconds: number, drawnMs: number, nowMs: number): number {
  const gone = Math.floor(Math.max(0, nowMs - drawnMs) / 1000);
  return Math.max(0, seconds - gone);
}

/** The countdown in words. At zero the engine moves on to the next step by itself. */
export function countdownText(left: number): string {
  if (left <= 0) return 'Time is up. Moving on…';
  const m = Math.floor(left / 60);
  const s = left % 60;
  if (m === 0) return `${s} s left`;
  return s === 0 ? `${m} min left` : `${m} min ${s} s left`;
}

/** The note under the reminder style: what the pet offers, and how often. */
export function cadenceNote(c: CoachConfig): string {
  if (c.reminder_style === 'combined') {
    return (
      `One break every ${c.break_minutes} min of time at the PC: a few sips of water, an eye ` +
      'exercise, the movement routine and slow breathing with your eyes closed, about 5 ' +
      'minutes in all. Every step can be skipped. Water pace is still shown, without its own ' +
      'reminder. Nothing is offered during focus, meetings, presentations, a pause or a ' +
      'locked PC, and nothing is replayed afterwards.'
    );
  }
  return (
    'Balanced: movement after 45 min of active work, eye rest every 20 min, water only when ' +
    'you are behind your own pace. One reminder at a time, with at least a 5-minute calm gap ' +
    'before the next one; snooze 10 min.'
  );
}
