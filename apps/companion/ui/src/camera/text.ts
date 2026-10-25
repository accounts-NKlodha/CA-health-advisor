// The camera page's words (wireframes W10 and W16), kept apart from the component so they
// can be tested with node --test.
import type { CameraPage, CameraStatus, PostureView } from '../shared/types';

/** The consent text (W16): what the posture check does and does not do. */
export const CONSENT_LINES: readonly string[] = [
  'The posture check uses your webcam to notice when your sitting position changes a lot from how you sat when you set it up. It is not a medical device and gives no health score.',
  'Camera frames are processed on this PC, in memory only. They are never saved, never sent anywhere (not to the office server, not to the internet) and never shown to anyone.',
  'While the camera is on, the pet shows a "CAM" badge. You can turn it off at any time from the pet menu, the tray or Settings, and capture stops within two seconds.',
  'It pauses by itself when you are away or the PC is locked, outside your work hours, and during presentations and meetings.',
  'Everything else in HealthierYou works the same without it.',
];

/** The longer explanation under the status line. */
export function statusDetail(s: CameraStatus): string {
  switch (s.kind) {
    case 'off':
      return 'Camera off. Capture stopped.';
    case 'starting':
      return 'Starting the camera and the posture model on this PC.';
    case 'reading':
      return 'The camera is on and reading your position. Nothing is saved or sent.';
    case 'no_reading':
      if (s.detail === 'starting') return 'The camera is on; waiting for the first reading.';
      if (s.detail === 'stalled') return 'The posture model stopped responding; it is being restarted.';
      switch (s.detail.frame) {
        case 'absent':
          return 'No one in view, so there is no reading. Nothing is measured until you are back.';
        case 'low_confidence':
          return 'Your face and both shoulders need to be clearly visible for a reading. Check the light and that nothing covers the camera.';
        case 'out_of_frame':
          return 'Both shoulders need to be inside the camera view for a reading.';
      }
      return 'No reading.';
    case 'held':
      switch (s.detail) {
        case 'away':
          return 'Paused while you are away or the PC is locked. The camera is not in use.';
        case 'paused':
          return 'Paused while coaching is paused. The camera is not in use.';
        case 'off_hours':
          return 'Paused outside your work hours. The camera is not in use.';
        case 'presentation':
          return 'Paused during a presentation. The camera is not in use.';
        case 'meeting':
          return 'Paused during a meeting. The camera is not in use.';
      }
      return 'Paused. The camera is not in use.';
    case 'unavailable':
      switch (s.detail) {
        case 'denied':
          return 'Windows blocked the camera for this app. To allow it, open Windows Settings > Privacy & security > Camera, turn on camera access for desktop apps, then choose Try again. Everything else keeps working.';
        case 'busy':
          return 'Another app is using the camera. Nothing is measured; the posture check tries again every minute. Everything else keeps working.';
        case 'no_camera':
          return 'No camera was found. The posture check tries again every minute. Everything else keeps working.';
        case 'model_failed':
          return 'The posture model could not load on this PC. Everything else keeps working.';
        case 'stalled':
          return 'The camera page stopped responding twice, so the camera was turned off. You can turn it on again.';
        case 'failed':
          return 'The camera could not start. Everything else keeps working.';
      }
      return 'The camera cannot be used now.';
  }
}

/** Whether the page offers "Try again". */
export function offersRetry(s: CameraStatus): boolean {
  return s.kind === 'unavailable';
}

/** A short line of measurements, for the curious (and the self-test's screenshots). */
export function statsLine(p: CameraPage): string {
  const s = p.stats;
  const parts = [`${s.frames} frames read`, `${s.valid_frames} usable`];
  if (s.inference_ms !== null) parts.push(`${Math.round(s.inference_ms)} ms a frame`);
  parts.push(`${p.fps} a second`);
  return parts.join(', ');
}

/** What calibration is (slice V2-2), shown before the first one. */
export const CALIBRATION_HELP =
  'Calibrate once: sit as you normally do, facing the screen, and keep still for about 15 seconds. Posture cues compare later readings with this, never with an ideal posture.';

/** The longer explanation of the calibration and baseline state (slice V2-2). */
export function postureDetail(p: PostureView): string {
  const s = p.state;
  switch (s.kind) {
    case 'not_calibrated':
      return 'No calibration yet, so no posture cues are given.';
    case 'calibrating':
      return `Keep still, sitting as you normally do: ${s.detail.valid} of ${s.detail.needed} clear frames.`;
    case 'not_usable':
      return `Calibration not usable: ${p.issue ?? 'try again'}. No posture cues are given until a calibration succeeds.`;
    case 'ready':
      return 'Calibrated. After about a minute of sitting clearly differently from this, the pet may give one short cue (at most every 30 minutes, never during focus).';
    case 'suspect':
      if (s.detail === 'app_restart' || s.detail === 'camera_restart') {
        return 'Checking that the camera still sees you as at calibration. Cues are paused meanwhile.';
      }
      return 'The camera view seems to have changed (moved or tilted). Cues are paused until it matches the calibration again, or you recalibrate.';
    case 'recalibrate':
      return 'The camera view no longer matches the calibration, so cues are paused. Calibrate again to resume them.';
  }
  return '';
}
