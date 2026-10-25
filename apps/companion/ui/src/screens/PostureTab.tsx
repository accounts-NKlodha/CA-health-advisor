// Settings > Posture check (V2-1; wireframe W10): the status, Set up (the consent and
// camera page), Turn camera on or off, Try again, Withdraw consent, and the privacy note.
// Slice V2-2 adds the calibration's state; Calibrate is on the camera page, where the
// user can see their progress.
import { useEffect, useState } from 'preact/hooks';
import { call, on, report } from '../shared/ipc';
import type { CameraPage } from '../shared/types';
import { coverageText, offersRetry, postureDetail, statusDetail } from '../camera/text';

export function PostureTab() {
  const [page, setPage] = useState<CameraPage | null>(null);
  const [status, setStatus] = useState('');

  useEffect(() => {
    call('camera_page').then(setPage).catch(report);
    const off = on('hy://camera', setPage);
    return () => {
      off.then((f) => f()).catch(report);
    };
  }, []);

  if (!page) return <main class="pane" aria-busy="true" />;

  const act = (p: Promise<CameraPage>, done: string) =>
    p.then((v) => {
      setPage(v);
      setStatus(done);
    }).catch((e: unknown) => setStatus(String(e)));

  return (
    <main class="pane">
      <h2>Posture check</h2>
      <section aria-labelledby="cam-status-h">
        <h3 id="cam-status-h">Status</h3>
        <p role="status">
          <strong>{page.words}</strong>
        </p>
        <p>{statusDetail(page.status)}</p>
        <div class="actions">
          {!page.consented ? (
            <button type="button" class="btn primary" onClick={() => void call('camera_open').catch(report)}>
              Set up
            </button>
          ) : page.on ? (
            <button type="button" class="btn primary" onClick={() => act(call('camera_off'), 'Camera off. Capture stopped.')}>
              Turn camera off
            </button>
          ) : (
            <button type="button" class="btn primary" onClick={() => act(call('camera_on'), 'Camera turned on.')}>
              Turn camera on
            </button>
          )}
          {offersRetry(page.status) ? (
            <button type="button" class="btn" onClick={() => act(call('camera_try_again'), 'Trying again.')}>
              Try again
            </button>
          ) : null}
          {page.consented ? (
            <button type="button" class="btn" onClick={() => void call('camera_open').catch(report)}>
              Open camera page
            </button>
          ) : null}
        </div>
      </section>
      {page.consented ? (
        <section aria-labelledby="cam-cal-h">
          <h3 id="cam-cal-h">Calibration</h3>
          <p>
            <strong>{page.posture.words}</strong>
          </p>
          <p>{postureDetail(page.posture)}</p>
          {page.posture.calibrated_at ? (
            <p class="note">Calibrated {new Date(page.posture.calibrated_at).toLocaleString()}.</p>
          ) : null}
          <p class="note">{coverageText(page.posture.today)}</p>
          <button type="button" class="btn" onClick={() => void call('camera_open').catch(report)}>
            Calibrate on the camera page
          </button>
        </section>
      ) : null}
      <section aria-labelledby="cam-privacy-h">
        <h3 id="cam-privacy-h">Privacy</h3>
        <p class="note">
          Off unless you turn it on. Camera frames are processed on this PC in memory only:
          never saved, never sent to the office server or the internet. Posture results stay
          on this PC and are not shared. The pet shows a "CAM" badge whenever the camera may
          be in use.
        </p>
        {page.consented ? (
          <button
            type="button"
            class="btn link"
            onClick={() => act(call('camera_withdraw'), 'Consent withdrawn. The camera is off.')}
          >
            Withdraw consent
          </button>
        ) : null}
      </section>
      <p class="status" role="status">
        {status}
      </p>
    </main>
  );
}
