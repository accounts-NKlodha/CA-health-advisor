// node --test: screen time (DEC13) is shown apart from the goals: today's figure in the pet
// menu and keyboard panel, each day's in the weekly review and monthly progress. A day
// without a figure is no data, never zero.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  screenDay,
  screenShort,
  screenToday,
  screenTotal,
  screenWords,
} from '../src/shared/screenTime.ts';

test('the pet menu line shows today in hours and minutes', () => {
  assert.equal(screenToday(200), 'Screen time today: 3 h 20 min');
  assert.equal(screenToday(45), 'Screen time today: 45 min');
  assert.equal(screenToday(120), 'Screen time today: 2 h');
  assert.equal(screenToday(0), 'Screen time today: 0 min');
  // Nothing until the engine reports today's figure.
  assert.equal(screenToday(null), null);
});

test('a day cell is short and no data is a dash, not zero', () => {
  assert.equal(screenShort(3 * 3600 + 5 * 60 + 59), '3h 05m');
  assert.equal(screenShort(10 * 3600 + 20 * 60), '10h 20m');
  assert.equal(screenShort(45 * 60), '45m');
  assert.equal(screenShort(0), '0m');
  assert.equal(screenShort(null), '–');
  assert.notEqual(screenShort(null), screenShort(0));
});

test('words for titles and screen readers', () => {
  assert.equal(screenWords(3 * 3600 + 20 * 60), '3 h 20 min');
  assert.equal(screenDay(5400), 'screen time 1 h 30 min');
  assert.equal(screenDay(null), 'screen time: no data');
});

test('the week total counts only days with a figure', () => {
  const days = [3600, null, 5400, 0, null, null, null].map((screen_s) => ({ screen_s }));
  assert.equal(screenTotal(days), '2 h 30 min over 3 days');
  assert.equal(screenTotal([{ screen_s: 600 }]), '10 min over 1 day');
  assert.equal(screenTotal([{ screen_s: null }]), 'No data');
});
