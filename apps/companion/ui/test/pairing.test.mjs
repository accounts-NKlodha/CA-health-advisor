// node --test (Node strips the TypeScript types): the pairing panel's text and checks.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { checkCodeText, newPcs, slipInput, slipLooksComplete, statusLine } from '../src/screens/pairing.ts';

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
