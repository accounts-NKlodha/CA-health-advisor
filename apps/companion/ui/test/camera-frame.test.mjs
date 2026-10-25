// node --test: the camera page's valid-frame rule and words (slice V2-1). Landmarks here
// are synthetic numbers, never taken from an image of anyone.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { MIN_VISIBILITY, frameDue, judgeFrame, nextTimestamp, postureFeatures } from '../src/camera/frame.ts';
import { CALIBRATION_HELP, CONSENT_LINES, coverageText, offersRetry, postureDetail, statsLine, statusDetail } from '../src/camera/text.ts';

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

// ------------------------------------------------------------------ posture features (V2-2)

test('a valid frame carries its posture numbers; an invalid one carries none', () => {
  const r = judgeFrame([figure()], 30, 4 / 3);
  assert.ok(r.features);
  const f = r.features;
  assert.equal(f.head, 'ears');
  // Shoulders 0.30 of the frame width apart, level, centred at (0.5, 0.6).
  assert.equal(f.width, 0.3);
  assert.equal(f.tilt, 0);
  assert.equal(f.mid_x, 0.5);
  assert.equal(f.mid_y, 0.6);
  // Ears 0.12 apart: 0.4 of the shoulder width; level, so no roll; the nose centred.
  assert.equal(f.span, 0.4);
  assert.equal(f.roll, 0);
  assert.equal(f.turn, 0);
  // The gap is measured in frame-width units: (0.6 - 0.29) * 0.75 / 0.3.
  assert.ok(Math.abs(f.gap - 0.775) < 1e-3, String(f.gap));
  assert.equal(judgeFrame([figure({ 0: { visibility: 0.1 } })], 30).features, undefined);
  assert.equal(judgeFrame([], 30).features, undefined);
  // Only numbers: nothing like a landmark list rides along.
  assert.deepEqual(Object.keys(f).sort(), ['gap', 'head', 'mid_x', 'mid_y', 'roll', 'span', 'tilt', 'turn', 'width']);
});

test('slumping lowers the gap, leaning in widens the shoulders, tilting shows', () => {
  const base = judgeFrame([figure()], 30).features;
  const slumped = judgeFrame([figure({ 0: { y: 0.4 }, 2: { y: 0.37 }, 5: { y: 0.37 }, 7: { y: 0.39 }, 8: { y: 0.39 } })], 30).features;
  assert.ok(slumped.gap < base.gap - 0.2);
  const lean = judgeFrame([figure({ 11: { x: 0.3 }, 12: { x: 0.7 } })], 30).features;
  assert.ok(lean.width > base.width);
  const tilted = judgeFrame([figure({ 11: { y: 0.64 } })], 30).features;
  assert.ok(Math.abs(tilted.tilt) > 0.05);
  const rolled = judgeFrame([figure({ 7: { y: 0.33 } })], 30).features;
  assert.ok(Math.abs(rolled.roll) > 10);
  const turned = judgeFrame([figure({ 0: { x: 0.54 } })], 30).features;
  assert.ok(Math.abs(turned.turn) > 0.3);
  // A mirrored image gives the same numbers (apart from the sign of the turn).
  const mirror = figure().map((p) => ({ ...p, x: 1 - p.x }));
  const m = judgeFrame([mirror], 30).features;
  assert.equal(m.roll, base.roll);
  assert.equal(m.gap, base.gap);
});

test('the eyes stand in for unseen ears', () => {
  const r = judgeFrame([figure({ 7: { visibility: 0.1 }, 8: { visibility: 0.1 } })], 30);
  assert.equal(r.valid, true);
  assert.equal(r.features.head, 'eyes');
  assert.equal(r.features.span, 0.2);
  assert.equal(postureFeatures(figure({ 11: { x: 0.5 }, 12: { x: 0.5 } }), true, 4 / 3), null);
});

test('the calibration words name the reason and never judge posture', () => {
  const view = (state, issue = null) => ({ state, words: '', issue, calibrated_at: null, can_calibrate: true, stats: {}, today: null });
  assert.match(postureDetail(view({ kind: 'not_calibrated' })), /no posture cues/i);
  assert.match(postureDetail(view({ kind: 'calibrating', detail: { valid: 12, needed: 40 } })), /12 of 40/);
  assert.match(postureDetail(view({ kind: 'not_usable', detail: 'too_far' }, 'you are too far')), /not usable: you are too far/);
  assert.match(postureDetail(view({ kind: 'suspect', detail: 'camera_moved' })), /paused/);
  assert.match(postureDetail(view({ kind: 'recalibrate' })), /Calibrate again/);
  const all = [CALIBRATION_HELP, postureDetail(view({ kind: 'ready' }))].join(' ').toLowerCase();
  for (const w of ['bad', 'poor posture', 'wrong', 'slouch', 'health']) assert.ok(!all.includes(w), w);
});

test("today's coverage is counts in words, never a score, and says nothing without data", () => {
  assert.match(coverageText(null), /not been on long enough/);
  const none = { camera_on_s: 30, no_reading_s: 0, unscored_s: 0, usable_s: 30, usable_share: 1, episodes: 0, cues_shown: 0 };
  assert.match(coverageText(none), /not been on long enough/);
  const day = { camera_on_s: 3600, no_reading_s: 300, unscored_s: 120, usable_s: 3180, usable_share: 3180 / 3600, episodes: 2, cues_shown: 1 };
  const t = coverageText(day);
  assert.match(t, /camera on 60 minutes/);
  assert.match(t, /88% of it/);
  assert.match(t, /no reading 5 minutes/);
  assert.match(t, /not calibrated 2 minutes/);
  assert.match(t, /1 cue shown/);
  for (const w of ['bad', 'poor posture', 'score', 'health']) assert.ok(!t.toLowerCase().includes(w), w);
});
