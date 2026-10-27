// node --test: the pet menu and keyboard panel group the engine's entries into short
// sections (owner review: "very cluttery, make some sections"), keep every entry and its
// full label as its accessible name, and the size control reaches 50% to 150%.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { groupActions, placeOf, sizeCommitter, snapSize, wheelSize } from '../src/shared/menuGroups.ts';

const a = (id, label, continues = false) => ({ id, label, in_tray: false, continues });

// The entries an idle office day shows (crates/hy-pet link/present.rs menu_actions).
const IDLE = [
  a('water_200', '+200 ml water', true),
  a('water_panel', 'Water... (250 / 2000 ml)', true),
  a('movement_start', "Today's movement"),
  a('break', 'Mark a break'),
  a('deadline_day', 'Deadline day'),
  a('focus_30', 'Focus 30 min'),
  a('focus_60', 'Focus 60 min'),
  a('focus_90', 'Focus 90 min'),
  a('pomodoro_25', 'Pomodoro 25/5'),
  a('pomodoro_50', 'Pomodoro 50/10'),
  a('phone_note', 'Phone interruption', true),
  a('meeting_on', "I'm in a meeting"),
  a('pause_30', 'Pause 30 min'),
  a('pause_60', 'Pause 1 hour'),
  a('pause_today', 'Pause for today'),
  a('pause_on', 'Pause until I resume'),
];

function flatten(sections) {
  const out = [];
  for (const s of sections) {
    for (const item of s.items) {
      if (item.kind === 'button') out.push({ section: s.id, id: item.action.id, text: item.action.label });
      else for (const g of item.segments) out.push({ section: s.id, id: g.action.id, text: g.short, row: item.label });
    }
  }
  return out;
}

test('the idle menu shows Water, Movement & breaks, Focus and Pause in order', () => {
  const s = groupActions(IDLE);
  assert.deepEqual(
    s.map((x) => x.title),
    ['Water', 'Movement & breaks', 'Focus', 'Pause'],
  );
});

test('every entry appears exactly once', () => {
  const ids = flatten(groupActions(IDLE)).map((x) => x.id);
  assert.deepEqual([...ids].sort(), IDLE.map((x) => x.id).sort());
});

test('sections hold the right entries', () => {
  const by = {};
  for (const x of flatten(groupActions(IDLE))) (by[x.section] ??= []).push(x.id);
  assert.deepEqual(by.water, ['water_200', 'water_panel']);
  assert.deepEqual(by.movement, ['movement_start', 'break', 'phone_note']);
  assert.deepEqual(by.focus, ['focus_30', 'focus_60', 'focus_90', 'pomodoro_25', 'pomodoro_50', 'deadline_day']);
  assert.deepEqual(by.pause, ['meeting_on', 'pause_30', 'pause_60', 'pause_today', 'pause_on']);
});

test('focus, Pomodoro and pause durations become compact segmented rows', () => {
  const focus = groupActions(IDLE).find((x) => x.id === 'focus');
  const rows = focus.items.filter((i) => i.kind === 'segments');
  assert.deepEqual(rows.map((r) => r.label), ['Focus', 'Pomodoro']);
  assert.deepEqual(rows[0].segments.map((g) => g.short), ['30 min', '60 min', '90 min']);
  assert.deepEqual(rows[1].segments.map((g) => g.short), ['25/5', '50/10']);
  const pause = groupActions(IDLE).find((x) => x.id === 'pause');
  const row = pause.items.find((i) => i.kind === 'segments');
  assert.equal(row.label, 'Pause');
  assert.deepEqual(row.segments.map((g) => g.short), ['30 min', '1 hour', 'For today', 'Until I resume']);
  // The full label stays with each segment for its accessible name.
  assert.equal(row.segments[3].action.label, 'Pause until I resume');
});

test('a shared warning moves to the row label and is never dropped', () => {
  const s = groupActions([
    a('focus_30_rp', 'Focus 30 min (ends Pomodoro)'),
    a('focus_60_rp', 'Focus 60 min (ends Pomodoro)'),
    a('pomodoro_25_rf_rc', 'Pomodoro 25/5 (ends focus, resumes coaching)'),
  ]);
  const rows = s[0].items;
  assert.equal(rows[0].label, 'Focus (ends Pomodoro)');
  assert.deepEqual(rows[0].segments.map((g) => g.short), ['30 min', '60 min']);
  assert.equal(rows[1].label, 'Pomodoro (ends focus, resumes coaching)');
});

test('running states keep their buttons in their section', () => {
  const s = groupActions([
    a('open_pending', 'Show waiting reminder'),
    a('movement_done', 'Movement done'),
    a('focus_end', 'End focus'),
    a('pomodoro_pause', 'Pause Pomodoro'),
    a('pomodoro_end', 'End Pomodoro'),
    a('recovery_show', 'Recovery day: Fri 30 Oct'),
    a('meeting_off', 'Meeting over'),
    a('resume', 'Resume coaching'),
    a('something_new', 'A new entry'),
  ]);
  assert.deepEqual(s.map((x) => x.id), ['top', 'movement', 'focus', 'pause', 'more']);
  assert.equal(s[0].title, '');
  assert.deepEqual(
    flatten(s).filter((x) => x.section === 'focus').map((x) => x.id),
    ['focus_end', 'pomodoro_pause', 'pomodoro_end', 'recovery_show'],
  );
  assert.equal(placeOf('pomodoro_25_rc').row, 'pomodoro');
  assert.equal(placeOf('pomodoro_resume').row, undefined);
});

test('the size snaps to 5% steps from 50% to 150%', () => {
  assert.equal(snapSize(47, 50, 150, 5), 50);
  assert.equal(snapSize(73, 50, 150, 5), 75);
  assert.equal(snapSize(160, 50, 150, 5), 150);
});

test('wheel down makes the pet smaller, one step per notch, within the range', () => {
  assert.equal(wheelSize(75, 100, 50, 150, 5), 70);
  assert.equal(wheelSize(75, -100, 50, 150, 5), 80);
  assert.equal(wheelSize(50, 100, 50, 150, 5), 50);
  assert.equal(wheelSize(150, -3, 50, 150, 5), 150);
  assert.equal(wheelSize(100, 0, 50, 150, 5), 100);
});

// A fake clock: timers run only when the test advances time.
function fakeTimers() {
  let now = 0;
  let next = 1;
  const queue = new Map();
  return {
    set(fn, ms) {
      const id = next++;
      queue.set(id, { at: now + ms, fn });
      return id;
    },
    clear(id) {
      queue.delete(id);
    },
    advance(ms) {
      now += ms;
      for (const [id, t] of [...queue].sort((x, y) => x[1].at - y[1].at)) {
        if (t.at <= now) {
          queue.delete(id);
          t.fn();
        }
      }
    },
  };
}

test('a preset chosen before a pending wheel save settles wins', () => {
  // The menu's wiring: a wheel notch saves later, a preset click saves now.
  const timers = fakeTimers();
  const sizes = sizeCommitter(timers, 250);
  const saved = [];
  const save = (pct) => saved.push(pct);
  // Wheel down once from 75: 70 is pending.
  sizes.later(wheelSize(75, 100, 50, 150, 5), save);
  timers.advance(100);
  // XL clicked before 250 ms.
  sizes.now(150, save);
  timers.advance(1000);
  assert.deepEqual(saved, [150], 'the pending wheel save must not restore 70');
});

test('wheel notches save once, after the wheel settles, with the last size', () => {
  const timers = fakeTimers();
  const sizes = sizeCommitter(timers, 250);
  const saved = [];
  const save = (pct) => saved.push(pct);
  let pct = 100;
  for (let i = 0; i < 4; i += 1) {
    pct = wheelSize(pct, 100, 50, 150, 5);
    sizes.later(pct, save);
    timers.advance(100);
  }
  assert.deepEqual(saved, []);
  timers.advance(250);
  assert.deepEqual(saved, [80]);
  // A slider change after a wheel turn also replaces the pending save.
  sizes.later(75, save);
  sizes.now(120, save);
  timers.advance(1000);
  assert.deepEqual(saved, [80, 120]);
});
