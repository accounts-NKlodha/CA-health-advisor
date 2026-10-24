// The camera page (architecture 4.5; wireframe W16), in Preact. It is its own window and
// exists only while the posture check needs it: the consent text the first time, then
// the status while the camera is on (the window may be hidden; capture goes on), and the
// reason when the camera cannot be used. Only this window may use the camera, and only
// while the shell says so.
import { render } from 'preact';
import { useEffect, useState } from 'preact/hooks';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { applyA11y, followA11y } from '../shared/a11y';
import { call, on, report } from '../shared/ipc';
import type { CameraPage } from '../shared/types';
import { capture } from './capture';
import { CONSENT_LINES, offersRetry, statsLine, statusDetail } from './text';

function closeWindow(): void {
  getCurrentWindow().close().catch(report);
}

function App() {
  const [page, setPage] = useState<CameraPage | null>(null);
  const [error, setError] = useState('');

  const adopt = (p: CameraPage) => {
    setPage(p);
    applyA11y(p.a11y);
    // The shell's word decides; a page asked to capture starts (a no-op if running).
    if (p.capture) void capture.start();
  };

  useEffect(() => {
    // Listen first, then ask: a start sent while the page loads is not lost.
    Promise.all([
      on('camera://view', adopt),
      on('camera://command', (c) => capture.command(c)),
    ])
      .then(() => call('camera_page'))
      .then(adopt)
      .catch((e: unknown) => setError(String(e)));
    followA11y();
  }, []);

  if (error) {
    return (
      <div class="shell">
        <p class="warn" role="alert">
          {error} Close the window and open it again to retry.
        </p>
      </div>
    );
  }
  if (!page) return <div class="shell" aria-busy="true" />;

  const act = (p: Promise<CameraPage>) => p.then(adopt).catch((e: unknown) => setError(String(e)));

  if (!page.consented) {
    return (
      <div class="shell camera">
        <header class="top">
          <h1>Posture check</h1>
        </header>
        <main>
          <section aria-labelledby="consent-h">
            <h2 id="consent-h">Before you turn on the camera</h2>
            {CONSENT_LINES.map((l) => (
              <p key={l}>{l}</p>
            ))}
          </section>
          <div class="actions">
            <button type="button" class="btn primary" onClick={() => act(call('camera_consent', { accept: true }))}>
              Turn on camera
            </button>
            <button type="button" class="btn" onClick={() => act(call('camera_consent', { accept: false }))}>
              Not now
            </button>
          </div>
        </main>
      </div>
    );
  }

  const s = page.status;
  const capturing = s.kind === 'starting' || s.kind === 'reading' || s.kind === 'no_reading';
  return (
    <div class="shell camera">
      <header class="top">
        <h1>Posture check</h1>
      </header>
      <main>
        <p class="status-line" role="status">
          <span class={capturing ? 'cam-chip on' : 'cam-chip'} aria-hidden="true">
            CAM
          </span>{' '}
          <strong>{page.words}</strong>
        </p>
        <p role={s.kind === 'unavailable' ? 'alert' : undefined}>{statusDetail(s)}</p>
        {capturing ? <p class="note">{statsLine(page)}</p> : null}
        <p class="note">
          Frames stay in memory on this PC and are never saved or sent. Closing this window
          keeps the posture check running; turn it off here, from the pet menu or the tray.
        </p>
        <div class="actions">
          {page.on ? (
            <button type="button" class="btn primary" onClick={() => act(call('camera_off'))}>
              Turn camera off
            </button>
          ) : (
            <button type="button" class="btn primary" onClick={() => act(call('camera_on'))}>
              Turn camera on
            </button>
          )}
          {offersRetry(s) ? (
            <button type="button" class="btn" onClick={() => act(call('camera_try_again'))}>
              Try again
            </button>
          ) : null}
          <button type="button" class="btn" onClick={closeWindow}>
            Close
          </button>
        </div>
      </main>
    </div>
  );
}

const app = document.getElementById('app');
if (app) render(<App />, app);
