// node --test (Node strips the TypeScript types): the pairing panel's text and checks.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  checkCodeText,
  enableNotice,
  newPcs,
  shareLine,
  slipInput,
  slipLooksComplete,
  statusLine,
} from '../src/screens/pairing.ts';

const SLIP = 'hy1-office-pc-8443-' + 'a'.repeat(52) + '-' + 'b'.repeat(16) + '-cccc';

const view = (over) => ({
  state: 'none',
  alias: null,
  host: null,
  words: [],
  check_code: null,
  confirmed_on: null,
  manual_address: null,
  devices: [],
  sharing: [],
  notices: [],
  hold: false,
  mismatch: false,
  last_sync: null,
  message: null,
  ...over,
});

test('a pasted slip keeps its characters; only spacing is tidied', () => {
  assert.equal(slipInput(`  ${SLIP}\n`), SLIP);
  assert.equal(slipInput('hy1-a  b\n c'), 'hy1-a b c');
});

test('Pair is enabled only for text shaped like a slip', () => {
  assert.equal(slipLooksComplete(SLIP), true);
  assert.equal(slipLooksComplete(SLIP.toUpperCase()), true);
  assert.equal(slipLooksComplete('hy1-office-pc-8443-abc'), false);
  assert.equal(slipLooksComplete('hello ' + 'a'.repeat(90)), false);
});

test('the check code is read out in two pairs', () => {
  assert.equal(checkCodeText('7KQ2'), '7K Q2');
  assert.equal(checkCodeText(null), '');
});

test('the status line says what is shared in each state', () => {
  assert.match(statusLine(view({})), /nothing is shared or sent/);
  assert.match(statusLine(view({ state: 'pending', alias: 'u1', host: 'office-pc' })), /Waiting for your administrator.*Nothing is shared yet/);
  assert.match(statusLine(view({ state: 'confirmed', alias: 'u1', host: 'office-pc', confirmed_on: '2026-10-23' })), /since 2026-10-23/);
});

test('only other PCs that are new are announced', () => {
  const devices = [
    { device_ref: 'a', label: 'PC-A', confirmed: true, this_pc: true, new: true },
    { device_ref: 'b', label: 'PC-B', confirmed: true, this_pc: false, new: true },
    { device_ref: 'c', label: 'PC-C', confirmed: true, this_pc: false, new: false },
  ];
  assert.deepEqual(newPcs(view({ state: 'confirmed', devices })).map((d) => d.label), ['PC-B']);
});

test('the status line says when sharing is paused or how many categories are shared', () => {
  const base = { state: 'confirmed', alias: 'u1', host: 'office-pc', confirmed_on: '2026-10-23' };
  assert.match(statusLine(view({ ...base, hold: true })), /Server recovering: sharing paused/);
  assert.match(statusLine(view({ ...base, mismatch: true })), /history mismatch/);
  const one = [{ category: 'hydration', label: 'Hydration summary', enabled: true, share_from: '2026-10-23', waiting: false, available: true }];
  assert.match(statusLine(view({ ...base, sharing: one, last_sync: '2026-10-23 12:00' })), /Sharing 1 category; last synced 2026-10-23 12:00/);
});

test('each switch says on from when, waiting, or off; the notice says what is sent', () => {
  assert.equal(shareLine({ label: 'Hydration summary', enabled: true, share_from: '2026-10-23', waiting: false }), 'Hydration summary: on (from 2026-10-23)');
  assert.equal(shareLine({ label: 'Hydration summary', enabled: false, share_from: null, waiting: true }), 'Hydration summary: waiting for the office server');
  assert.equal(shareLine({ label: 'Hydration summary', enabled: false, share_from: null, waiting: false }), 'Hydration summary: off');
  // A category with no record producer yet never looks implemented, even if switched on
  // from another PC.
  for (const enabled of [false, true]) {
    assert.equal(
      shareLine({ label: 'Posture check summary (V2)', enabled, share_from: null, waiting: false, available: false }),
      'Posture check summary (V2): not yet sharing (coming in a later update)',
    );
  }
  // Outcomes and the work session are produced in V1 and read like any other category.
  for (const label of ['Reminder and exercise outcomes', 'Work-session summary']) {
    assert.equal(
      shareLine({ label, enabled: true, share_from: '2026-10-23', waiting: false, available: true }),
      `${label}: on (from 2026-10-23)`,
    );
  }
  assert.match(enableNotice('Hydration summary'), /daily totals.*from today on/);
});
