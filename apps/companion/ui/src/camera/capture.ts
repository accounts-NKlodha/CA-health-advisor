// The camera page's capture (architecture 4.5): the camera at 640 x 480, the pose worker,
// and the stop that lets go of the camera. Frames never leave this page and are never
// stored: the worker gets the track (or one image at a time) and sends back only each
// frame's derived result, which is passed to the shell.
//
// The shell decides; this module carries out its commands: start, stop, restart the
// worker, frame rate, and the self-test's freeze and network probe.
import { call, report } from '../shared/ipc';
import type { CapturePath, PageCommand } from '../shared/types';
import PoseWorker from './pose.worker?worker&inline';
import type { FromWorker, ToWorker } from './pose.worker';

/** How long the worker gets to close itself before it is ended. */
const WORKER_CLOSE_MS = 600;

interface TrackProcessor {
  readable: ReadableStream<VideoFrame>;
}
type TrackProcessorClass = new (init: { track: MediaStreamTrack }) => TrackProcessor;

function processorClass(): TrackProcessorClass | null {
  const c = (globalThis as { MediaStreamTrackProcessor?: TrackProcessorClass }).MediaStreamTrackProcessor;
  return typeof c === 'function' ? c : null;
}

/** The bundled model and runtime, as absolute addresses (the worker runs from a blob). */
function assets(): { base: string; model: string } {
  return {
    base: new URL('./mediapipe/wasm', location.href).href,
    model: new URL('./mediapipe/pose_landmarker_lite.task', location.href).href,
  };
}

function errorName(e: unknown): string {
  if (e instanceof Error && e.name) return e.name.slice(0, 64);
  if (typeof e === 'object' && e !== null && 'name' in e) return String((e as { name: unknown }).name).slice(0, 64);
  return 'Error';
}

class Capture {
  private stream: MediaStream | null = null;
  /** The track clone the current worker reads (processor path). */
  private clone: MediaStreamTrack | null = null;
  private video: HTMLVideoElement | null = null;
  private worker: Worker | null = null;
  private fps = 3;
  private starting = false;
  private reported = false;
  private path: CapturePath = processorClass() ? 'processor' : 'video';
  /** Bumped by every stop, so a start overtaken by a stop lets go of what it got. */
  private generation = 0;

  get running(): boolean {
    return this.stream !== null || this.starting;
  }

  command(c: PageCommand): void {
    switch (c.kind) {
      case 'start':
        void this.start();
        break;
      case 'stop':
        void this.stop();
        break;
      case 'restart_worker':
        this.restartWorker();
        break;
      case 'fps':
        this.fps = c.fps;
        this.worker?.postMessage({ type: 'fps', fps: c.fps } satisfies ToWorker);
        break;
      case 'freeze':
        this.worker?.postMessage({ type: 'freeze' } satisfies ToWorker);
        break;
      case 'probe':
        this.worker?.postMessage({ type: 'probe' } satisfies ToWorker);
        break;
    }
  }

  /** Start the camera and the worker (a no-op while running). */
  async start(): Promise<void> {
    if (this.running) return;
    this.starting = true;
    this.reported = false;
    const gen = this.generation;
    let stream: MediaStream;
    try {
      stream = await navigator.mediaDevices.getUserMedia({
        audio: false,
        video: { width: { ideal: 640 }, height: { ideal: 480 }, frameRate: { ideal: 15, max: 30 } },
      });
    } catch (e) {
      this.starting = false;
      if (gen === this.generation) {
        call('camera_report', { report: { kind: 'error', name: errorName(e), model: false } }).catch(report);
      }
      return;
    }
    if (gen !== this.generation) {
      // Stopped while waiting for the camera.
      for (const t of stream.getTracks()) t.stop();
      this.starting = false;
      return;
    }
    this.stream = stream;
    this.starting = false;
    const track = stream.getVideoTracks()[0];
    // Another app taking the camera, or the device going away, ends the track.
    track?.addEventListener('ended', () => {
      if (this.stream === stream) {
        void this.stop(false);
        call('camera_report', { report: { kind: 'error', name: 'NotReadableError', model: false } }).catch(report);
      }
    });
    if (this.path === 'video') {
      const v = document.createElement('video');
      v.muted = true;
      v.playsInline = true;
      v.srcObject = stream;
      this.video = v;
      await v.play().catch(() => undefined);
    }
    this.startWorker();
  }

  private startWorker(): void {
    const stream = this.stream;
    const track = stream?.getVideoTracks()[0];
    if (!stream || !track) return;
    const w = new PoseWorker();
    this.worker = w;
    let readable: ReadableStream<VideoFrame> | null = null;
    const Proc = processorClass();
    if (this.path === 'processor' && Proc) {
      this.clone = track.clone();
      readable = new Proc({ track: this.clone }).readable;
    }
    w.onmessage = (e: MessageEvent<FromWorker>) => this.fromWorker(w, e.data);
    w.onerror = () => {
      if (this.worker === w) {
        call('camera_report', { report: { kind: 'error', name: 'WorkerError', model: true } }).catch(report);
      }
    };
    const init: ToWorker = { type: 'init', ...assets(), fps: this.fps, readable };
    w.postMessage(init, readable ? [readable as unknown as Transferable] : []);
  }

  private fromWorker(w: Worker, m: FromWorker): void {
    if (this.worker !== w) return;
    switch (m.type) {
      case 'ready': {
        if (this.reported) return; // a restarted worker: the shell already knows
        this.reported = true;
        const s = this.stream?.getVideoTracks()[0]?.getSettings();
        call('camera_report', {
          report: {
            kind: 'started',
            width: s?.width ?? 0,
            height: s?.height ?? 0,
            cores: navigator.hardwareConcurrency || 1,
            model_ms: m.model_ms,
            path: this.path,
          },
        }).catch(report);
        break;
      }
      case 'result':
        call('camera_frame', { report: m.report }).catch(report);
        break;
      case 'need':
        void this.sendImage(w);
        break;
      case 'error':
        call('camera_report', { report: { kind: 'error', name: m.name, model: m.model } }).catch(report);
        break;
      case 'probe':
        call('camera_report', { report: { kind: 'probe', blocked: m.blocked, detail: m.detail } }).catch(report);
        break;
      case 'closed':
        break;
    }
  }

  private async sendImage(w: Worker): Promise<void> {
    const v = this.video;
    if (!v || v.readyState < 2) return;
    try {
      const bitmap = await createImageBitmap(v);
      if (this.worker === w) {
        w.postMessage({ type: 'frame', bitmap } satisfies ToWorker, [bitmap]);
      } else {
        bitmap.close();
      }
    } catch {
      // The next request tries again.
    }
  }

  /** End the worker: ask it to close, then end it regardless. */
  private endWorker(): Promise<void> {
    const w = this.worker;
    this.worker = null;
    this.clone?.stop();
    this.clone = null;
    if (!w) return Promise.resolve();
    return new Promise((resolve) => {
      const done = () => {
        w.terminate();
        resolve();
      };
      const timer = setTimeout(done, WORKER_CLOSE_MS);
      w.onmessage = (e: MessageEvent<FromWorker>) => {
        if (e.data.type === 'closed') {
          clearTimeout(timer);
          done();
        }
      };
      w.postMessage({ type: 'close' } satisfies ToWorker);
    });
  }

  private restartWorker(): void {
    if (!this.stream) return;
    void this.endWorker().then(() => {
      if (this.stream) this.startWorker();
    });
  }

  /** Let go of the camera first, then end the worker, then tell the shell (if asked). */
  async stop(tell = true): Promise<void> {
    this.generation += 1;
    const stream = this.stream;
    this.stream = null;
    if (stream) for (const t of stream.getTracks()) t.stop();
    if (this.video) {
      this.video.srcObject = null;
      this.video = null;
    }
    await this.endWorker();
    if (tell) call('camera_report', { report: { kind: 'stopped' } }).catch(report);
  }
}

export const capture = new Capture();
