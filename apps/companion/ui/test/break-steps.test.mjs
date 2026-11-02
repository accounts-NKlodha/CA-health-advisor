// node --test: the combined 5-minute break (DEC15). Its step cards carry a countdown the
// page runs by itself from when the card is drawn (the shell does not redraw the card every
// second), and Settings > Reminders words the reminder style and its cadence.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { BREAK_MINUTES, cadenceNote, countdownText, secondsLeft } from '../src/shared/breaks.ts';

test('a step counts down in whole seconds from when its card was drawn', () => {
  const drawn = 1_000_000;
  // The water step: 10 seconds.
  assert.equal(secondsLeft(10, drawn, drawn), 10);
  assert.equal(secondsLeft(10, drawn, drawn + 999), 10);
  assert.equal(secondsLeft(10, drawn, drawn + 1000), 9);
  assert.equal(secondsLeft(10, drawn, drawn + 9500), 1);
  assert.equal(secondsLeft(10, drawn, drawn + 10_000), 0);
  // Never below zero, and a clock that steps back shows the full time, not more.
  assert.equal(secondsLeft(10, drawn, drawn + 60_000), 0);
  assert.equal(secondsLeft(10, drawn, drawn - 5000), 10);
});

test('the countdown in words, and what happens when it runs out', () => {
  // The eye step: 15 seconds; the breathing step: 60 to 90 seconds.
  assert.equal(countdownText(15), '15 s left');
  assert.equal(countdownText(1), '1 s left');
  assert.equal(countdownText(60), '1 min left');
  assert.equal(countdownText(90), '1 min 30 s left');
  // At zero the engine moves on to the next step by itself.
  assert.equal(countdownText(0), 'Time is up. Moving on…');
});

test('the break interval choices', () => {
  assert.deepEqual([...BREAK_MINUTES], [20, 30, 45, 60]);
});

test('the reminder style note says what the pet offers, and how often', () => {
  const combined = cadenceNote({ reminder_style: 'combined', break_minutes: 30 });
  assert.match(combined, /^One break every 30 min of time at the PC/);
  for (const step of ['water', 'eye exercise', 'movement routine', 'slow breathing with your eyes closed']) {
    assert.ok(combined.includes(step), step);
  }
  assert.match(combined, /nothing is replayed afterwards/);
  assert.match(cadenceNote({ reminder_style: 'combined', break_minutes: 45 }), /every 45 min/);
  // Separate reminders keep the words they had before DEC15.
  const separate = cadenceNote({ reminder_style: 'separate', break_minutes: 30 });
  assert.match(separate, /^Balanced: movement after 45 min of active work, eye rest every 20 min/);
  assert.match(separate, /5-minute calm gap/);
});
