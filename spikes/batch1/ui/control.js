// Camera resource test (C1 to C4): MediaPipe pose lite at 3 frames a second.
// Resource evidence only; it says nothing about posture accuracy.
import { FilesetResolver, PoseLandmarker } from './vendor/vision_bundle.mjs';

const invoke = (kind, data) => window.__TAURI__.core.invoke('report', { kind, data }).catch(() => {});
const $ = (id) => document.getElementById(id);
const video = $('video');
let stream = null, timer = null, landmarker = null;
let n = 0, detected = 0, inferSum = 0, inferMax = 0;

async function getLandmarker() {
  if (landmarker) return landmarker;
  const t0 = performance.now();
  const fileset = await FilesetResolver.forVisionTasks('./vendor/wasm');
  landmarker = await PoseLandmarker.createFromOptions(fileset, {
    baseOptions: { modelAssetPath: './vendor/pose_landmarker_lite.task', delegate: 'CPU' },
    runningMode: 'VIDEO',
    numPoses: 1,
  });
  invoke('model_loaded', { load_ms: Math.round(performance.now() - t0), delegate: 'CPU' });
  return landmarker;
}

function step() {
  if (!stream) return;
  if (video.readyState >= 2) {
    const t0 = performance.now();
    const r = landmarker.detectForVideo(video, t0);
    const ms = performance.now() - t0;
    n++; inferSum += ms; inferMax = Math.max(inferMax, ms);
    if (r.landmarks && r.landmarks.length) detected++;
  }
  timer = setTimeout(step, 333);
}

setInterval(() => {
  if (!stream || !n) return;
  const data = { frames: n, person_detected: detected, mean_infer_ms: +(inferSum / n).toFixed(1), max_infer_ms: Math.round(inferMax) };
  invoke('camera_frames', data);
  $('stats').textContent = `Last 5 s: ${n} frames, person seen in ${detected}, inference ${data.mean_infer_ms} ms average`;
  n = 0; detected = 0; inferSum = 0; inferMax = 0;
}, 5000);

async function startCamera() {
  $('start').disabled = true;
  $('status').textContent = 'Starting camera...';
  invoke('camera_start_pressed', {});
  try {
    stream = await navigator.mediaDevices.getUserMedia({ video: { width: 640, height: 480 }, audio: false });
  } catch (e) {
    stream = null;
    const busy = e.name === 'NotReadableError' || e.name === 'AbortError';
    $('status').textContent = busy
      ? `Camera unavailable (${e.name}): another app may be using it. The pet keeps working.`
      : `Camera unavailable (${e.name}). The pet keeps working.`;
    invoke('camera_error', { name: e.name, message: String(e.message || '') });
    $('start').disabled = false;
    return false;
  }
  video.srcObject = stream;
  await video.play().catch(() => {});
  invoke('camera_started', {});
  try {
    await getLandmarker();
  } catch (e) {
    invoke('model_error', { message: String(e && e.message || e) });
    $('status').textContent = `Pose model failed to load: ${e && e.message || e}`;
  }
  $('status').textContent = 'Camera test running. Sit normally; press Stop when done.';
  $('stop').disabled = false;
  if (landmarker) step();
  return true;
}
$('start').onclick = startCamera;

function stopCamera() {
  clearTimeout(timer);
  if (stream) stream.getTracks().forEach((t) => t.stop());
  stream = null;
  video.srcObject = null;
  invoke('camera_stopped', {});
  $('status').textContent = 'Camera stopped. Count the seconds until the camera light goes off.';
  $('stop').disabled = true;
  $('start').disabled = false;
}
$('stop').onclick = stopCamera;

invoke('control_ready', { userAgent: navigator.userAgent });

// Scripted run: camera for N minutes, then 5 short start/stop cycles.
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
(async () => {
  const cfg = await window.__TAURI__.core.invoke('autotest_mode').catch(() => ({ mode: '' }));
  if (!cfg.mode) return;
  invoke('autotest_begin', cfg);
  await sleep(3000);
  if (cfg.mode === 'camera_denied') {
    await startCamera();
    if (stream) stopCamera();
    await sleep(6000);
  } else if (cfg.mode === 'camera') {
    if (await startCamera()) {
      await sleep(cfg.camera_minutes * 60000);
      stopCamera();
      for (let i = 0; i < 5; i++) {
        await sleep(10000);
        if (!(await startCamera())) break;
        await sleep(20000);
        stopCamera();
      }
    }
  }
  invoke('autotest_done', { mode: cfg.mode });
})();
