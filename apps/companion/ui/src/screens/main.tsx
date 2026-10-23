// The screens page uses Preact (architecture 7). It exists only while a screen is open:
// the window is created on demand and destroyed on close (A7).
// Settings (wireframes W3 to W9 and W12): Pet (slice V1-1), then Reminders, Water and
// workweek, Movement programme, Focus and Pomodoro, Sharing and server, Backup, and
// Accessibility (slice V1-4). The
// navigation collapses to a row above the content below 760 px. Every change goes to the
// shell, which validates it, saves it in the local store and answers with the new page;
// the pet and the coaching follow at once.
import { render } from 'preact';
import { useEffect, useState } from 'preact/hooks';
import { applyA11y, followA11y } from '../shared/a11y';
import { call, on, report } from '../shared/ipc';
import type { SettingsPage } from '../shared/types';
import { AccessibilityTab } from './Accessibility';
import { newer } from './changes';
import { FocusTab, MovementTab, RemindersTab } from './Coaching';
import { BackupTab, SharingTab } from './Data';
import { PetSettings } from './PetSettings';
import { WaterTab } from './Water';

type Tab = 'pet' | 'reminders' | 'water' | 'movement' | 'focus' | 'sharing' | 'backup' | 'a11y';

const TABS: [Tab, string][] = [
  ['pet', 'Pet'],
  ['reminders', 'Reminders'],
  ['water', 'Water & workweek'],
  ['movement', 'Movement programme'],
  ['focus', 'Focus & Pomodoro'],
  ['sharing', 'Sharing & server'],
  ['backup', 'Backup'],
  ['a11y', 'Accessibility'],
];

/** How a tab saves: the shell's answer becomes the page, and `done` the status line. */
export type Save = (p: Promise<SettingsPage>, done: string) => void;

function App() {
  const [tab, setTab] = useState<Tab>('pet');
  const [page, setPage] = useState<SettingsPage | null>(null);
  const [status, setStatus] = useState('');
  const [error, setError] = useState('');

  // Every answer goes through `newer`: answers to quick successive saves can arrive out
  // of order, and the page with the newest settings revision wins.
  const show = (v: SettingsPage) => setPage((p) => newer(p, v));
  const reload = () => {
    call('settings_page').then(show).catch(report);
  };

  useEffect(() => {
    call('settings_page')
      .then((p) => {
        show(p);
        applyA11y(p.a11y);
      })
      .catch((e: unknown) => setError(String(e)));
    followA11y((a) => setPage((p) => (p ? { ...p, a11y: a } : p)));
    // A change saved from another window (the weekly review's Apply or Undo) shows here
    // at once. Saves send only the changed field, so a page that is still stale never
    // writes back what it shows.
    on('hy://settings', reload).catch(report);
    window.addEventListener('focus', reload);
  }, []);

  const save: Save = (p, done) => {
    p.then((v) => {
      show(v);
      applyA11y(v.a11y);
      setStatus(done);
    }).catch((e: unknown) => {
      setStatus(String(e));
      // Show what is saved, not what was refused.
      reload();
    });
  };

  const body = () => {
    if (tab === 'pet') return <PetSettings />;
    if (error) {
      return (
        <main class="pane">
          <p class="warn" role="alert">
            {error} Close the window and open it again to retry.
          </p>
        </main>
      );
    }
    if (!page) return <main class="pane" aria-busy="true" />;
    switch (tab) {
      case 'reminders':
        return <RemindersTab page={page} save={save} />;
      case 'water':
        return <WaterTab page={page} save={save} />;
      case 'movement':
        return <MovementTab page={page} save={save} />;
      case 'focus':
        return <FocusTab page={page} save={save} />;
      case 'sharing':
        return <SharingTab page={page} />;
      case 'backup':
        return <BackupTab page={page} />;
      case 'a11y':
        return <AccessibilityTab page={page} save={save} />;
    }
  };

  return (
    <div class="shell">
      <header class="top">
        <h1>Settings</h1>
      </header>
      <div class="settings-layout">
        <nav class="settings-nav" aria-label="Settings">
          {TABS.map(([id, label]) => (
            <button
              type="button"
              class="nav-btn"
              key={id}
              aria-current={tab === id ? 'page' : undefined}
              onClick={() => {
                setTab(id);
                setStatus('');
              }}
            >
              {label}
            </button>
          ))}
          <span class="nav-sep" aria-hidden="true" />
          <button
            type="button"
            class="nav-btn"
            onClick={() => {
              call('open_review').catch(report);
            }}
          >
            Weekly review and progress
          </button>
          <button
            type="button"
            class="nav-btn"
            onClick={() => {
              call('open_setup').catch(report);
            }}
          >
            Run setup again
          </button>
        </nav>
        <div>
          {body()}
          {tab === 'pet' || tab === 'backup' ? null : (
            <p class="status" role="status">
              {status}
            </p>
          )}
        </div>
      </div>
    </div>
  );
}

const root = document.getElementById('app');
if (root) {
  render(<App />, root);
}

window.addEventListener('unhandledrejection', (e) => report(e.reason));
