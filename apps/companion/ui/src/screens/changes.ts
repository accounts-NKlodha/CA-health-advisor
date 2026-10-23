// What a settings save sends, and which answer a page keeps. Pure functions (no imports
// at run time), tested with node:test in ui/test.
//
// A save sends only the fields the user changed, never the page's cached copy of the
// settings: another window (the weekly review's Apply or Undo) may have changed other
// fields since the page loaded, and a whole copy would silently undo them. The shell
// applies each change to what is saved now, one at a time on the engine's thread.
import type { CoachChange, CoachConfig, SettingsPage, WaterChange, WeekdayName, WorkweekChange } from '../shared/types';

/** The saved workweek, and what the form shows now. */
export interface WorkweekForm {
  workdays: WeekdayName[];
  start: string;
  end: string;
  zone: string;
}

/** Only the workweek fields the form changed, or null when nothing changed. */
export function workweekChange(saved: WorkweekForm, form: WorkweekForm): WorkweekChange | null {
  const change: WorkweekChange = {};
  if (form.workdays.join() !== saved.workdays.join()) change.workdays = form.workdays;
  if (form.start !== saved.start || form.end !== saved.end) change.hours = { start: form.start, end: form.end };
  if (form.zone !== saved.zone) change.zone = form.zone;
  return Object.keys(change).length > 0 ? change : null;
}

/** The page to show: answers to quick successive saves may arrive out of order, so the
 *  page with the newest revision wins (an equal revision is a fresh read: it wins). */
export function newer(shown: SettingsPage | null, next: SettingsPage): SettingsPage {
  return shown !== null && shown.revision > next.revision ? shown : next;
}

/** The setup wizard's serving list as one edit against the list it showed: what to add
 *  and what to remove, so buttons another window changed meanwhile are kept. */
export function servingEdit(shown: number[], wanted: number[]): WaterChange | null {
  const add = wanted.filter((ml) => !shown.includes(ml));
  const remove = shown.filter((ml) => !wanted.includes(ml));
  return add.length + remove.length > 0 ? { edit_servings: { add, remove } } : null;
}

/** The wizard's reminder choices as one change per field that differs from what it showed. */
export function reminderChanges(shown: CoachConfig, wanted: CoachConfig): CoachChange[] {
  const out: CoachChange[] = [];
  if (wanted.intensity !== shown.intensity) out.push({ intensity: wanted.intensity });
  if (wanted.categories.movement !== shown.categories.movement) out.push({ movement: wanted.categories.movement });
  if (wanted.categories.eye_rest !== shown.categories.eye_rest) out.push({ eye_rest: wanted.categories.eye_rest });
  if (wanted.categories.water !== shown.categories.water) out.push({ water: wanted.categories.water });
  return out;
}
