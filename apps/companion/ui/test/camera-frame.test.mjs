// node --test: the camera page's valid-frame rule and words (slice V2-1). Landmarks here
// are synthetic numbers, never taken from an image of anyone.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { MIN_VISIBILITY, frameDue, judgeFrame, nextTimestamp } from '../src/camera/frame.ts';
import { CONSENT_LINES, offersRetry, statsLine, statusDetail } from '../src/camera/text.ts';

/** 33 synthetic landmarks: a centred, upright figure, every point seen. */
function figure(over = {}) {
  const lm = Array.from({ length: 33 }, () => ({ x: 0.5, y: 0.5, visibility: 0.99 }));
  lm[0] = { x: 0.5, y: 0.3, visibility: 0.99 };
  lm[2] = { x: 0.47, y: 0.27, visibility: 0.99 };
  lm[5] = { x: 0.53, y: 0.27, visibility: 0.99 };
  lm[7] = { x: 0.44, y: 0.29, visibility: 0.9 };
  lm[8] = { x: 0.56, y: 0.29, visibility: 0.9 };
  lm[11] = { x: 0.35, y: 0.6, visibility: 0.95 };
  lm[12] = { x: 0.65, y: 0.6, visibility: 0.95 };
  for (const [i, p] of Object.entries(over)) lm[Number(i)] = { ...lm[Number(i)], ...p };
  return lm;
}

test('a frame with the nose, both eyes and both shoulders seen and inside is valid', () => {
  const r = judgeFrame([figure()], 37.4);
  assert.equal(r.valid, true);
  assert.equal(r.issue, null);
  assert.equal(r.inference_ms, 37.4);
  assert.ok(r.visibility > 0.9 && r.visibility <= 1);
});

test('no person is absent, with no visibility', () => {
  assert.deepEqual(judgeFrame([], 20), { valid: false, issue: 'absent', inference_ms: 20, visibility: 0 });
  assert.equal(judgeFrame([figure().slice(0, 12)], 20).issue, 'absent');
});

test('both ears may stand in for both eyes, but one of each is not enough', () => {
  const lowEyes = { 2: { visibility: 0.2 }, 5: { visibility: 0.2 } };
  assert.equal(judgeFrame([figure(lowEyes)], 30).valid, true);
  const oneEach = { 2: { visibility: 0.2 }, 8: { visibility: 0.2 } };
  assert.equal(judgeFrame([figure(oneEach)], 30).issue, 'low_confidence');
});

test('a landmark just under the visibility floor gives low confidence', () => {
  const under = MIN_VISIBILITY - 0.01;
  assert.equal(judgeFrame([figure({ 0: { visibility: under } })], 30).issue, 'low_confidence');
  assert.equal(judgeFrame([figure({ 12: { visibility: under } })], 30).issue, 'low_confidence');
  assert.equal(judgeFrame([figure({ 11: { visibility: MIN_VISIBILITY } })], 30).valid, true);
  // A missing visibility counts as not seen.
  assert.equal(judgeFrame([figure({ 0: { visibility: undefined } })], 30).issue, 'low_confidence');
});

test('a shoulder outside the frame is out of frame', () => {
  assert.equal(judgeFrame([figure({ 11: { x: -0.02 } })], 30).issue, 'out_of_frame');
  assert.equal(judgeFrame([figure({ 12: { y: 1.05 } })], 30).issue, 'out_of_frame');
});

test('reports always pass the shell checks: finite, in range, valid xor issue', () => {
  for (const ms of [Number.NaN, -5, 1e9]) {
    const r = judgeFrame([figure()], ms);
    assert.ok(r.inference_ms >= 0 && r.inference_ms <= 60000);
  }
  const r = judgeFrame([figure({ 0: { visibility: 7 } })], 30);
  assert.ok(r.visibility <= 1);
  assert.equal(r.valid, r.issue === null);
});

test('frames are taken at the set rate and timestamps always increase', () => {
  assert.equal(frameDue(null, 0, 3), true);
  assert.equal(frameDue(1000, 1300, 3), false);
  assert.equal(frameDue(1000, 1322, 3), true); // a 30 fps camera gives an even 3 a second
  assert.equal(frameDue(1000, 1900, 1), false);
  assert.equal(frameDue(1000, 2000, 1), true);
  assert.equal(nextTimestamp(100, 100.4), 101);
  assert.equal(nextTimestamp(100, 250.9), 250);
});

test('the consent text says what the camera does and does not do', () => {
  const all = CONSENT_LINES.join(' ');
  assert.match(all, /not a medical device/);
  assert.match(all, /never saved, never sent/);
  assert.match(all, /"CAM" badge/);
  assert.match(all, /within two seconds/);
  assert.match(all, /works the same without it/);
});

test('each status explains itself, and only an unavailable camera offers Try again', () => {
  assert.equal(statusDetail({ kind: 'off' }), 'Camera off. Capture stopped.');
  assert.match(statusDetail({ kind: 'unavailable', detail: 'denied' }), /Privacy & security > Camera/);
  assert.match(statusDetail({ kind: 'unavailable', detail: 'busy' }), /Another app is using the camera/);
  assert.match(statusDetail({ kind: 'no_reading', detail: { frame: 'out_of_frame' } }), /shoulders/);
  assert.match(statusDetail({ kind: 'held', detail: 'meeting' }), /not in use/);
  assert.equal(offersRetry({ kind: 'unavailable', detail: 'no_camera' }), true);
  assert.equal(offersRetry({ kind: 'reading' }), false);
  const stats = { frames: 30, valid_frames: 0, stalls: 0, worker_restarts: 0, last_stop_ms: null, last_stop_forced: false, model_ms: 900, inference_ms: 41.6, fps: 3 };
  assert.equal(statsLine({ stats, fps: 3 }), '30 frames read, 0 usable, 42 ms a frame, 3 a second');
});
