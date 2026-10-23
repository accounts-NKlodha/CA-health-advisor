// node --test: the review window keeps the week the review offer's "Open" named (Codex
// review of #13): in a new window, in one already open on another week or on Monthly
// progress, across focus refreshes and tab changes.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { START, navigate, request } from '../src/review/nav.ts';

const offer = { type: 'target', week: '2026-10-19' };

test('a new window opened by the offer loads the offer week', () => {
  const nav = navigate(START, offer);
  assert.deepEqual(request(nav), { command: 'review_week', day: '2026-10-19' });
});

test('a focus refresh keeps the offer week', () => {
  const nav = navigate(START, offer);
  // A focus refresh asks again for exactly what is shown.
  assert.deepEqual(request(nav), request(navigate(START, offer)));
  assert.equal(request(nav).day, '2026-10-19');
});

test('Month and back to Week keeps the offer week', () => {
  let nav = navigate(START, offer);
  nav = navigate(nav, { type: 'tab', tab: 'month' });
  assert.equal(request(nav).command, 'review_month');
  nav = navigate(nav, { type: 'tab', tab: 'week' });
  assert.deepEqual(request(nav), { command: 'review_week', day: '2026-10-19' });
});

test('an open window on another week or on Monthly progress goes to the offer week', () => {
  let nav = navigate(START, { type: 'week', day: '2026-10-05' });
  nav = navigate(nav, { type: 'tab', tab: 'month' });
  nav = navigate(nav, { type: 'month', month: '2026-09' });
  nav = navigate(nav, offer);
  assert.deepEqual(request(nav), { command: 'review_week', day: '2026-10-19' });
  // The month chosen earlier stays for when the user switches back.
  assert.equal(navigate(nav, { type: 'tab', tab: 'month' }).month, '2026-09');
});

test('paging weeks after the offer works as before', () => {
  const nav = navigate(navigate(START, offer), { type: 'week', day: '2026-10-12' });
  assert.equal(request(nav).day, '2026-10-12');
});
