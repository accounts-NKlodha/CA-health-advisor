// node --test (Node strips the TypeScript types): settings saves send only changed fields
// and a page keeps the newest answer (Codex review of #13, stale full-config writes).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { newer, reminderChanges, servingEdit, workweekChange } from '../src/screens/changes.ts';

const saved = { workdays: ['mon', 'tue', 'wed', 'thu', 'fri', 'sat'], start: '09:30', end: '18:30', zone: 'Asia/Kolkata' };

test('an unchanged workweek form sends nothing', () => {
  assert.equal(workweekChange(saved, { ...saved, workdays: [...saved.workdays] }), null);
});

test('a workweek save sends only the fields the form changed', () => {
  assert.deepEqual(workweekChange(saved, { ...saved, start: '08:00' }), { hours: { start: '08:00', end: '18:30' } });
  assert.deepEqual(workweekChange(saved, { ...saved, zone: 'Europe/London' }), { zone: 'Europe/London' });
  assert.deepEqual(workweekChange(saved, { ...saved, workdays: ['mon', 'tue'] }), { workdays: ['mon', 'tue'] });
});

const page = (revision, eyeRest) => ({ revision, coach: { categories: { eye_rest: eyeRest } } });

test('answers to rapid successive saves: the newest revision stays shown', () => {
  // Two toggles sent at once; the second answer (revision 2) arrives first.
  let shown = page(0, true);
  shown = newer(shown, page(2, false));
  shown = newer(shown, page(1, true));
  assert.equal(shown.revision, 2);
  // A refresh after a change made in another window (the review's Apply) replaces it.
  shown = newer(shown, page(3, false));
  assert.equal(shown.revision, 3);
  // An equal revision is a fresh read and wins; nothing shown yet takes any answer.
  assert.equal(newer(page(3, true), page(3, false)).coach.categories.eye_rest, false);
  assert.equal(newer(null, page(0, true)).revision, 0);
});

test('the setup wizard sends its serving list as an edit of what it showed', () => {
  assert.equal(servingEdit([150, 250, 500], [150, 250, 500]), null);
  assert.deepEqual(servingEdit([150, 250, 500], [200, 250, 500]), { edit_servings: { add: [200], remove: [150] } });
});

test('the setup wizard sends one change per reminder field it changed', () => {
  const shown = { intensity: 'balanced', categories: { movement: true, eye_rest: true, water: true } };
  assert.deepEqual(reminderChanges(shown, shown), []);
  assert.deepEqual(
    reminderChanges(shown, { intensity: 'light', categories: { movement: true, eye_rest: false, water: true } }),
    [{ intensity: 'light' }, { eye_rest: false }],
  );
});
