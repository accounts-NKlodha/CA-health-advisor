// Settings > Sharing & server (wireframe W8) and Backup (W9).
// Sharing: this build has no office server yet, so the status reads "Not paired" and
// every optional category is off; pairing arrives with the office server.
// Backup: "Export my data" writes one encrypted file to the Documents folder; "Import a
// backup" replaces all HealthierYou data on this PC with a file's (never merges), keeps
// every deletion from both sides, and restarts the app.
import { useState } from 'preact/hooks';
import { call, stageBackup } from '../shared/ipc';
import type { BackupSummary, SettingsPage } from '../shared/types';

const MIN_PASSWORD = 8;

export function SharingTab({ page }: { page: SettingsPage }) {
  return (
    <main class="pane">
      <h2>Sharing & server</h2>
      <p class="saved-note" role="status">
        Not paired. Everything works on this PC; nothing is shared or sent.
      </p>
      <section aria-labelledby="share-h">
        <h3 id="share-h">Optional sharing</h3>
        <p>
          All off. Each category can be switched on only after this PC is paired with the office
          server, which comes with a later update.
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
  const [status, setStatus] = useState('');
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
            .then((s) => {
              setStatus(`Imported ${held(s)}. HealthierYou is restarting...`);
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
      <p class="status" role="status">
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
