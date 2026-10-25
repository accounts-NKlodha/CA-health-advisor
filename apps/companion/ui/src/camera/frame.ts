// The valid-frame rule (architecture 4.5), applied in the pose worker to each frame's
// landmarks, which never leave the worker. Only the derived result is reported: valid or
// not, why not, and the mean visibility of the landmarks the rule needs.
//
// MediaPipe Pose landmark indices: 0 nose, 2 and 5 the eyes, 7 and 8 the ears, 11 and 12
// the shoulders. Coordinates are normalised to the frame (0 to 1).
import type { FrameIssue, FrameReport } from '../shared/types';

/** A landmark as the pose model gives it (the fields the rule reads). */
export interface Point {
  x: number;
  y: number;
  visibility?: number;
}

export const NOSE = 0;
export const LEFT_EYE = 2;
export const RIGHT_EYE = 5;
export const LEFT_EAR = 7;
export const RIGHT_EAR = 8;
export const LEFT_SHOULDER = 11;
export const RIGHT_SHOULDER = 12;

/** The least visibility a required landmark needs. */
export const MIN_VISIBILITY = 0.5;

const vis = (p: Point | undefined): number => {
  const v = p?.visibility;
  return typeof v === 'number' && Number.isFinite(v) ? Math.min(1, Math.max(0, v)) : 0;
};

const seen = (p: Point | undefined): boolean => vis(p) >= MIN_VISIBILITY;

const inside = (p: Point | undefined): boolean =>
  p !== undefined &&
  Number.isFinite(p.x) &&
  Number.isFinite(p.y) &&
  p.x >= 0 &&
  p.x <= 1 &&
  p.y >= 0 &&
  p.y <= 1;

/**
 * Judge one frame: `people` is the model's landmark list per person found (the first is
 * used; the worker asks for one). `inferenceMs` is the model's time for the frame.
 */
export function judgeFrame(people: readonly (readonly Point[])[], inferenceMs: number): FrameReport {
  const ms = Number.isFinite(inferenceMs) ? Math.min(60_000, Math.max(0, inferenceMs)) : 0;
  const lm = people[0];
  if (!lm || lm.length <= RIGHT_SHOULDER) {
    return { valid: false, issue: 'absent', inference_ms: ms, visibility: 0 };
  }
  const nose = lm[NOSE];
  const eyes = seen(lm[LEFT_EYE]) && seen(lm[RIGHT_EYE]);
  const ears = seen(lm[LEFT_EAR]) && seen(lm[RIGHT_EAR]);
  const ls = lm[LEFT_SHOULDER];
  const rs = lm[RIGHT_SHOULDER];
  // The pair used for the head is the better seen of eyes and ears.
  const eyePair = (vis(lm[LEFT_EYE]) + vis(lm[RIGHT_EYE])) / 2;
  const earPair = (vis(lm[LEFT_EAR]) + vis(lm[RIGHT_EAR])) / 2;
  let head = Math.max(eyePair, earPair);
  if (eyes !== ears) head = eyes ? eyePair : earPair;
  const visibility = (vis(nose) + head + vis(ls) + vis(rs)) / 4;
  let issue: FrameIssue | null = null;
  if (!seen(nose) || !(eyes || ears) || !seen(ls) || !seen(rs)) {
    issue = 'low_confidence';
  } else if (!inside(ls) || !inside(rs)) {
    issue = 'out_of_frame';
  }
  return {
    valid: issue === null,
    issue,
    inference_ms: ms,
    visibility: Math.round(visibility * 1000) / 1000,
  };
}

/** Whether `next` is due at `fps`, given the last processed frame's time (ms). */
export function frameDue(lastMs: number | null, nowMs: number, fps: number): boolean {
  if (lastMs === null) return true;
  const gap = 1000 / Math.max(1, fps);
  // A little slack so a 30 fps camera gives an even 3 fps, not 2.7.
  return nowMs - lastMs >= gap - 12;
}

/** A strictly increasing timestamp for the video-mode model (it refuses repeats). */
export function nextTimestamp(lastTs: number, nowMs: number): number {
  const t = Math.floor(nowMs);
  return t > lastTs ? t : lastTs + 1;
}
