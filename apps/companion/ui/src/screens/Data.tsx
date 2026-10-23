// Settings > Sharing & server (wireframe W8) and Backup (W9).
// Sharing: pairing with the office server (slice V1-5a). The user pastes the slip from
// the administrator; this PC then shows the server's six words and its own check code to
// read out, and stays "waiting for confirmation" until the administrator confirms it.
// Every optional category stays off; switching them on arrives with sync (V1-5b).
// Backup: "Export my data" writes one encrypted file to the Documents folder; "Import a
// backup" replaces all HealthierYou data on this PC with a file's (never merges), keeps
// every deletion from both sides, and restarts the app.
import { useEffect, useState } from 'preact/hooks';
import { call, on, report, stageBackup } from '../shared/ipc';
import type { BackupSummary, ImportFinished, LinkView, SettingsPage } from '../shared/types';
import { checkCodeText, newPcs, slipInput, slipLooksComplete, statusLine } from './pairing';

const MIN_PASSWORD = 8;

export function SharingTab({ page }: { page: SettingsPage }) {
  const [link, setLink] = useState<LinkView | null>(null);
  const [error, setError] = useState('');
  useEffect(() => {
    call('server_status').then(setLink).catch((err: unknown) => setError(String(err)));
  }, []);
  const paired = link !== null && link.state !== 'none';
  return (
    <main class="pane">
      <h2>Sharing & server</h2>
      <p class="saved-note" role="status">
        {link ? statusLine(link) : 'Reading...'}
      </p>
      {error ? (
        <p class="note" role="alert">
          {error}
        </p>
      ) : null}
      {link ? <ServerLink link={link} onChange={setLink} onError={setError} /> : null}
      <section aria-labelledby="share-h">
        <h3 id="share-h">Optional sharing</h3>
        <p>
          All off.{' '}
          {paired
            ? 'Switching a category on comes with a later update.'
            : 'Each category can be switched on only after this PC is paired with the office server.'}
        </p>
        <ul class="lib">
          {page.sharing.map((s) => (
            <li key={s.category}>
              <label class="check">
                <input type="checkbox" role="switch" checked={s.enabled} disabled />
                {s.label}: {s.enabled ? 'on' : 'off'}
              </label>
            </li>
          ))}
        </ul>
        <p class="note">
          Details are never sent: meditation, recovery plans, reflections, individual events and
          camera images stay on this PC.
        </p>
      </section>
    </main>
  );
}

function ServerLink({
  link,
  onChange,
  onError,
}: {
  link: LinkView;
  onChange: (v: LinkView) => void;
  onError: (e: string) => void;
}) {
  const [slip, setSlip] = useState('');
  const [busy, setBusy] = useState(false);
  const run = (p: Promise<LinkView>) => {
    setBusy(true);
    onError('');
    p.then(onChange)
      .catch((err: unknown) => onError(String(err)))
      .finally(() => setBusy(false));
  };
  if (link.state === 'none') {
    return (
      <section aria-labelledby="pair-h">
        <h3 id="pair-h">Pair with the office server</h3>
        <p>
          Your administrator gives you a pairing slip. It works once, for 24 hours. After pairing,
          read the six words and the check code shown here to your administrator. Nothing is
          shared until you switch a category on.
        </p>
        <form
          onSubmit={(e) => {
            e.preventDefault();
            if (!slipLooksComplete(slip) || busy) return;
            run(call('server_pair', { slip: slipInput(slip) }).then((v) => {
              setSlip('');
              return v;
            }));
          }}
        >
          <label class="field">
            <span>Pairing slip</span>
            <textarea
              rows={3}
              spellcheck={false}
              autocomplete="off"
              value={slip}
              aria-describedby="slip-note"
              onInput={(e) => setSlip(e.currentTarget.value)}
            />
          </label>
          <p class="note" id="slip-note">
            Paste it, or type it with or without the dashes. A typing mistake is caught here before
            anything is sent.
          </p>
          <button type="submit" class="btn primary" disabled={!slipLooksComplete(slip) || busy}>
            Pair
          </button>
        </form>
        {link.message ? <p class="status" role="status">{link.message}</p> : null}
      </section>
    );
  }
  const fresh = newPcs(link);
  return (
    <section aria-labelledby="server-h">
      <h3 id="server-h">Office server</h3>
      {link.state === 'pending' ? (
        <>
          <p>
            Read these to your administrator, in person or on a call they place to you. If the
            words they see are different, do not continue: the slip may have been altered.
          </p>
          <p class="words" aria-label="Six words">
            <strong>{link.words.join(' ')}</strong>
          </p>
          <p>
            Check code: <strong class="mono">{checkCodeText(link.check_code)}</strong>
          </p>
        </>
      ) : null}
      {fresh.length > 0 ? (
        <div class="notice" role="alert">
          <p>
            A new PC was paired to you: <strong>{fresh.map((d) => d.label).join(', ')}</strong>. If
            this was not you, tell your administrator.
          </p>
          <button
            type="button"
            class="btn"
            onClick={() => {
              const first = fresh[0];
              if (first) run(call('server_notice_seen', { deviceRef: first.device_ref }));
            }}
          >
            OK
          </button>
        </div>
      ) : null}
      {link.state === 'confirmed' && link.devices.length > 0 ? (
        <>
          <h4>PCs paired to you</h4>
          <ul class="lib">
            {link.devices.map((d) => (
              <li key={d.device_ref}>
                {d.label}
                {d.this_pc ? ' (this PC)' : ''}
                {d.confirmed ? '' : ' (waiting for confirmation)'}
              </li>
            ))}
          </ul>
        </>
      ) : null}
      {link.message ? <p class="status" role="status">{link.message}</p> : null}
      <p>
        <button type="button" class="btn" disabled={busy} onClick={() => run(call('server_refresh'))}>
          Check again
        </button>{' '}
        <button
          type="button"
          class="btn"
          disabled={busy}
          onClick={() => {
            if (window.confirm('Unpair this PC? Sharing stops and the server is told when it can be reached.')) {
              run(call('server_unpair'));
            }
          }}
        >
          Unpair this PC
        </button>
      </p>
    </section>
  );
}

function dayText(iso: string): string {
  const [y, m, d] = iso.split('-').map(Number) as [number, number, number];
  const months = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
  return `${d} ${months[m - 1]} ${y}`;
}

function held(s: BackupSummary): string {
  const range = s.from && s.to ? `, ${dayText(s.from)} to ${dayText(s.to)}` : '';
  return `${s.events} entries and ${s.days} days of activity${range}`;
}

function Export({ dir }: { dir: string | null }) {
  const [password, setPassword] = useState('');
  const [again, setAgain] = useState('');
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState('');
  const long = password.length >= MIN_PASSWORD;
  const same = password === again;
  return (
    <section aria-labelledby="export-h">
      <h3 id="export-h">Export my data</h3>
      <p>
        One encrypted file with your settings, goals, pet, reminders and logs, daily activity,
        summaries, deletions and consent history. Pairing and sharing choices are not included:
        this PC, or a new one, pairs again.
      </p>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          if (!long || !same || busy) return;
          setBusy(true);
          setStatus('Writing the backup...');
          call('backup_export', { password })
            .then((d) => {
              setStatus(`Saved ${held(d.summary)} to ${d.path}. Keep the password safe: without it the file cannot be opened.`);
              setPassword('');
              setAgain('');
            })
            .catch((err: unknown) => setStatus(String(err)))
            .finally(() => setBusy(false));
        }}
      >
        <label class="field">
          <span>Password for this file</span>
          <input
            type="password"
            autocomplete="new-password"
            value={password}
            aria-describedby="pw-note"
            onInput={(e) => setPassword(e.currentTarget.value)}
          />
        </label>
        <label class="field">
          <span>Type it again</span>
          <input
            type="password"
            autocomplete="new-password"
            value={again}
            onInput={(e) => setAgain(e.currentTarget.value)}
          />
        </label>
        <p class="note" id="pw-note">
          At least {MIN_PASSWORD} characters. HealthierYou cannot recover a forgotten password.
          {dir ? ` The file is saved in ${dir}.` : ''}
        </p>
        {again !== '' && !same ? (
          <p class="note" role="alert">
            The two passwords differ.
          </p>
        ) : null}
        <button type="submit" class="btn primary" disabled={!long || !same || busy}>
          Export
        </button>
      </form>
      <p class="status" role="status">
        {status}
      </p>
    </section>
  );
}

function Import() {
  const [file, setFile] = useState<string | null>(null);
  const [password, setPassword] = useState('');
  const [confirm, setConfirm] = useState(false);
  const [busy, setBusy] = useState(false);
  const [pending, setPending] = useState(false);
  const [status, setStatus] = useState('');
  // A slow import (a costly key or a large file) can outlast the wait. Its outcome is
  // not known until the engine thread reports it here: until then nothing is claimed.
  useEffect(() => {
    on('hy://backup-import', (f: ImportFinished) => {
      if (f.ok && f.summary) {
        setStatus(`Imported ${held(f.summary)}. HealthierYou is restarting...`);
      } else {
        setStatus(f.error ?? 'The import was refused. Nothing was changed.');
        setBusy(false);
      }
      setPending(false);
    }).catch(report);
  }, []);
  return (
    <section aria-labelledby="import-h">
      <h3 id="import-h">Import a backup</h3>
      <p>
        Replaces all HealthierYou data on this PC with the backup's. It never merges, so nothing
        is counted twice. Dates you deleted from sharing, here or in the backup, stay deleted.
        HealthierYou restarts afterwards.
      </p>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          if (!file || password === '' || !confirm || busy) return;
          setBusy(true);
          setStatus('Checking the backup...');
          call('backup_import', { password })
            .then((o) => {
              if (o.state === 'done') {
                setStatus(`Imported ${held(o.summary)}. HealthierYou is restarting...`);
              } else {
                setPending(true);
                setStatus(
                  'Still importing. Keep HealthierYou open: it restarts by itself when the import is done, or says here if the file was refused.',
                );
              }
            })
            .catch((err: unknown) => {
              setStatus(String(err));
              setBusy(false);
            });
        }}
      >
        <label class="field">
          <span>Backup file (.hybackup)</span>
          <input
            type="file"
            accept=".hybackup"
            onChange={(e) => {
              const f = e.currentTarget.files?.[0];
              setFile(null);
              if (!f) return;
              f.arrayBuffer()
                .then((buf) => stageBackup(new Uint8Array(buf)))
                .then(() => {
                  setFile(f.name);
                  setStatus('');
                })
                .catch((err: unknown) => setStatus(String(err)));
            }}
          />
        </label>
        <label class="field">
          <span>The file's password</span>
          <input
            type="password"
            autocomplete="off"
            value={password}
            onInput={(e) => setPassword(e.currentTarget.value)}
          />
        </label>
        <label class="check block">
          <input type="checkbox" checked={confirm} onChange={(e) => setConfirm(e.currentTarget.checked)} />
          <span>Replace the data on this PC with this backup</span>
        </label>
        <button type="submit" class="btn" disabled={!file || password === '' || !confirm || busy}>
          Import
        </button>
      </form>
      <p class="status" role="status" aria-busy={pending}>
        {status}
      </p>
    </section>
  );
}

export function BackupTab({ page }: { page: SettingsPage }) {
  return (
    <main class="pane">
      <h2>Backup</h2>
      <Export dir={page.backup_dir} />
      <Import />
      <p class="note">
        Updates and installing a previous version from a file come with the installer in a later
        update.
      </p>
    </main>
  );
}
