// The pose worker (architecture 4.5): MediaPipe Pose Landmarker lite on the CPU, in video
// mode, at the frame rate the shell sets (3 a second, 1 under load). The landmarks stay in
// this worker and are dropped after each frame; only the derived result (valid or not,
// why not, inference time, mean visibility) goes to the page. The model and the WebAssembly
// runtime are bundled with the app and loaded from it; nothing is fetched from the network,
// and the page's content policy (which this inline worker inherits) blocks any attempt.
//
// Frames come either from a camera track stream handed over by the page (the worker owns
// the timing, so a hidden window keeps its rate) or, where the browser has no track
// processor, as single images the worker asks the page for.
import { FilesetResolver, PoseLandmarker } from '@mediapipe/tasks-vision';
import { frameDue, judgeFrame, nextTimestamp } from './frame';

/** What the page sends. */
export type ToWorker =
  | { type: 'init'; base: string; model: string; fps: number; readable: ReadableStream<VideoFrame> | null }
  | { type: 'fps'; fps: number }
  | { type: 'frame'; bitmap: ImageBitmap }
  | { type: 'freeze' }
  | { type: 'probe' }
  | { type: 'close' };

/** What the worker sends. */
export type FromWorker =
  | { type: 'ready'; model_ms: number }
  | { type: 'error'; name: string; model: boolean }
  | { type: 'result'; report: ReturnType<typeof judgeFrame> }
  | { type: 'need' }
  | { type: 'probe'; blocked: boolean; detail: string }
  | { type: 'closed' };

interface Scope {
  postMessage(m: FromWorker): void;
  addEventListener(type: 'message', fn: (e: MessageEvent<ToWorker>) => void): void;
  addEventListener(type: 'securitypolicyviolation', fn: (e: SecurityPolicyViolationEvent) => void): void;
  close(): void;
}
const scope = self as unknown as Scope;

let landmarker: PoseLandmarker | null = null;
let fps = 3;
let lastFrameMs: number | null = null;
let lastTs = 0;
let frozen = false;
let closing = false;
let reader: ReadableStreamDefaultReader<VideoFrame> | null = null;
let askTimer: ReturnType<typeof setTimeout> | null = null;
/** Content-policy refusals seen in this worker (the library's usage metrics included). */
let blockedRequests = 0;

scope.addEventListener('securitypolicyviolation', () => {
  blockedRequests += 1;
});

function post(m: FromWorker): void {
  scope.postMessage(m);
}

function detect(source: VideoFrame | ImageBitmap): void {
  if (!landmarker || frozen || closing) return;
  const now = performance.now();
  lastTs = nextTimestamp(lastTs, now);
  const t0 = performance.now();
  const result = landmarker.detectForVideo(source, lastTs);
  const ms = performance.now() - t0;
  const report = judgeFrame(result.landmarks, ms);
  // The landmarks go no further than this function.
  post({ type: 'result', report });
}

/** Read the track stream: every frame is closed at once unless it is due for the model. */
async function pump(stream: ReadableStream<VideoFrame>): Promise<void> {
  reader = stream.getReader();
  for (;;) {
    let next: ReadableStreamReadResult<VideoFrame>;
    try {
      next = await reader.read();
    } catch {
      return;
    }
    if (next.done || closing) {
      next.value?.close();
      return;
    }
    const frame = next.value;
    try {
      const now = performance.now();
      if (frameDue(lastFrameMs, now, fps)) {
        lastFrameMs = now;
        detect(frame);
      }
    } catch (e) {
      post({ type: 'error', name: errorName(e), model: true });
    } finally {
      frame.close();
    }
  }
}

/** Without a track stream: ask the page for one image at the current rate. */
function ask(): void {
  if (closing) return;
  askTimer = setTimeout(() => {
    if (!frozen) post({ type: 'need' });
    ask();
  }, 1000 / Math.max(1, fps));
}

function errorName(e: unknown): string {
  if (e instanceof Error && e.name) return e.name.slice(0, 64);
  return 'Error';
}

async function init(m: Extract<ToWorker, { type: 'init' }>): Promise<void> {
  fps = m.fps;
  const t0 = performance.now();
  try {
    const files = await FilesetResolver.forVisionTasks(m.base);
    landmarker = await PoseLandmarker.createFromOptions(files, {
      baseOptions: { modelAssetPath: m.model, delegate: 'CPU' },
      runningMode: 'VIDEO',
      numPoses: 1,
      outputSegmentationMasks: false,
    });
  } catch (e) {
    post({ type: 'error', name: errorName(e), model: true });
    return;
  }
  post({ type: 'ready', model_ms: Math.round(performance.now() - t0) });
  if (m.readable) {
    void pump(m.readable);
  } else {
    ask();
  }
}

/** Try to reach the network (self-test only): the content policy must refuse it. */
async function probe(): Promise<void> {
  const before = blockedRequests;
  let detail: string;
  try {
    await fetch('https://example.com/', { mode: 'no-cors', cache: 'no-store' });
    detail = 'the request went out';
  } catch (e) {
    detail = `refused (${errorName(e)})`;
  }
  // The violation event is queued; give it a moment.
  await new Promise((r) => setTimeout(r, 200));
  const blocked = blockedRequests > before;
  post({
    type: 'probe',
    blocked,
    detail: `${detail}; content-policy refusals in this worker so far: ${blockedRequests}`,
  });
}

function close(): void {
  closing = true;
  if (askTimer !== null) clearTimeout(askTimer);
  void reader?.cancel().catch(() => undefined);
  try {
    landmarker?.close();
  } catch {
    // Closing anyway.
  }
  landmarker = null;
  post({ type: 'closed' });
  scope.close();
}

scope.addEventListener('message', (e) => {
  const m = e.data;
  switch (m.type) {
    case 'init':
      void init(m);
      break;
    case 'fps':
      fps = m.fps;
      break;
    case 'frame':
      try {
        detect(m.bitmap);
      } catch (err) {
        post({ type: 'error', name: errorName(err), model: true });
      } finally {
        m.bitmap.close();
      }
      break;
    case 'freeze':
      // Self-test only: stop answering, as a hung model would.
      frozen = true;
      break;
    case 'probe':
      void probe();
      break;
    case 'close':
      close();
      break;
  }
});
