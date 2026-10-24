// Settings > App & updates (slice V1-6): the version, start at sign-in, installing a
// signed update file (with the previous version kept and put back if the new one does
// not start), and where the data lives.
import { useEffect, useState } from 'preact/hooks';
import { call, report } from '../shared/ipc';
import type { AppPage, UpdateCheck } from '../shared/types';
import { autostartLine, checkSummary, markNote, outcomeLine, rollbackLine } from './app';

export function AppTab() {
  const [page, setPage] = useState<AppPage | null>(null);
  const [check, setCheck] = useState<UpdateCheck | null>(null);
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState('');

  useEffect(() => {
    call('app_page').then(setPage).catch(report);
  }, []);

  if (!page) return <main class="pane" aria-busy="true" />;

  const choose = () => {
    setBusy(true);
    setCheck(null);
    setStatus('Checking the file...');
    call('update_choose')
      .then((c) => {
        setCheck(c);
        setStatus('');
      })
      .catch((e: unknown) => setStatus(String(e)))
      .finally(() => setBusy(false));
  };

  const install = () => {
    setBusy(true);
    setStatus('Installing. HealthierYou closes now and starts again when the update is done...');
    call('update_install').catch((e: unknown) => {
      setStatus(String(e));
      setBusy(false);
    });
  };

  const note = check ? markNote(check) : null;
  return (
    <main class="pane">
      <h2>App &amp; updates</h2>
      <p>
        HealthierYou version {page.version}.
        {page.install_dir ? ` Installed for you in ${page.install_dir}.` : ''}
      </p>

      {page.autostart ? (
        <section aria-labelledby="autostart-h">
          <h3 id="autostart-h">Start at sign-in</h3>
          <label class="check block">
            <input
              type="checkbox"
              checked={page.autostart.enabled}
              onChange={(e) => {
                call('set_autostart', { enabled: e.currentTarget.checked })
                  .then(setPage)
                  .catch((err: unknown) => setStatus(String(err)));
              }}
            />
            <span>Start HealthierYou when I sign in to Windows</span>
          </label>
          <p class="note">{autostartLine(page.autostart)}</p>
        </section>
      ) : null}

      <section aria-labelledby="update-h">
        <h3 id="update-h">Install update from file</h3>
        <p>
          Choose the installer from the shared folder or USB stick. Its signature file (the same
          name ending in .sig) must be next to it. Nothing runs until the signature checks out.
          Your data stays where it is. If the new version does not start, the version you have now
          is put back.
        </p>
        {page.update_blocked ? (
          <p class="note" role="note">
            {page.update_blocked}
          </p>
        ) : (
          <button type="button" class="btn" disabled={busy || page.in_progress} onClick={choose}>
            Choose update file...
          </button>
        )}
        {check ? (
          <div class="notice" role="group" aria-label="Checked update">
            <p>{checkSummary(check)}</p>
            {note ? <p class="warn">{note}</p> : null}
            <div class="actions">
              <button type="button" class="btn primary" disabled={busy} onClick={install}>
                Install and restart
              </button>
                <button
                type="button"
                class="btn"
                disabled={busy}
                onClick={() => {
                  setCheck(null);
                  setStatus('');
                }}
              >
                Cancel
              </button>
            </div>
          </div>
        ) : null}
        <p class="note">{rollbackLine(page)}</p>
        {page.in_progress ? <p class="note">An update is in progress.</p> : null}
        {page.last ? <p class="note">Last update: {outcomeLine(page.last)}</p> : null}
        <p class="status" role="status">
          {status}
        </p>
      </section>

      <section aria-labelledby="uninstall-h">
        <h3 id="uninstall-h">Your data and uninstalling</h3>
        <p>
          Your data is in {page.data_dir}. To remove HealthierYou, use Windows Settings &gt; Apps.
          The data is kept unless you tick "Also delete my data" there, so a reinstall picks up
          where you left off. Export a backup first if you want a copy elsewhere.
        </p>
      </section>
    </main>
  );
}
