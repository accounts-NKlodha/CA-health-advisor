// node --test: the text of Settings > App & updates (slice V1-6).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { autostartLine, checkSummary, markNote, outcomeLine, rollbackLine } from '../src/screens/app.ts';

const check = (over) => ({
  current: '0.1.0',
  version: '0.2.0',
  plan: 'upgrade',
  size_mb: 172.4,
  marked_from_internet: false,
  ...over,
});

test('the checked file says what installing it will do', () => {
  assert.equal(
    checkSummary(check()),
    'Update from version 0.1.0 to 0.2.0 (172.4 MB). The signature checked out.',
  );
  assert.match(checkSummary(check({ plan: 'rollback', version: '0.0.9' })), /^Go back from version 0\.1\.0 to 0\.0\.9, the version this PC was updated from/);
});

test("a downloaded file warns about Windows' own check, which stays on", () => {
  assert.equal(markNote(check()), null);
  const note = markNote(check({ marked_from_internet: true }));
  assert.match(note, /Windows protected your PC/);
  assert.match(note, /does not switch that check off/);
  assert.match(note, /nothing is changed/);
});

test('each outcome reads as one line', () => {
  const o = (kind, detail = '') => ({ kind, from: '0.1.0', to: '0.2.0', at: '2026-11-10 18:05', detail });
  assert.equal(outcomeLine(o('updated')), '2026-11-10 18:05: updated from 0.1.0 to 0.2.0.');
  assert.match(outcomeLine(o('rolled_back', 'Version 0.1.0 was put back.')), /was undone\. Version 0\.1\.0 was put back\.$/);
  assert.match(outcomeLine(o('blocked', 'x')), /did not run\. x$/);
  assert.match(outcomeLine(o('failed', 'y')), /failed\. y$/);
});

test('start at sign-in respects a choice made in Windows', () => {
  assert.match(autostartLine({ enabled: true, switched_off_in_windows: false }), /starts when you sign in/);
  assert.match(autostartLine({ enabled: false, switched_off_in_windows: false }), /does not start by itself/);
  assert.match(autostartLine({ enabled: true, switched_off_in_windows: true }), /Switched off in Windows/);
});

test('going back names the only version it can be', () => {
  const page = { previous_version: null };
  assert.match(rollbackLine(page), /there is none yet/);
  assert.match(rollbackLine({ previous_version: '0.1.0' }), /install version 0\.1\.0/);
});
