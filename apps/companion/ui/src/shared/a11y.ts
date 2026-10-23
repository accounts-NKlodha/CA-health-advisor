// Display preferences (Settings, Accessibility; wireframe W12 and section 6): text size
// 100, 125 or 150%, reduced motion and higher contrast. Each page applies them to its
// root element as classes and a `--ts` text scale; the stylesheets do the rest. The pet
// and panel receive them in their views; the other windows also follow the "hy://a11y"
// event, so an open window changes as soon as the setting is saved.
import { listen } from '@tauri-apps/api/event';
import { report } from './ipc';
import type { Accessibility } from './types';

/** Apply the preferences to this page. */
export function applyA11y(a: Accessibility): void {
  const root = document.documentElement;
  root.classList.toggle('text-125', a.text_pct === 125);
  root.classList.toggle('text-150', a.text_pct === 150);
  root.classList.toggle('reduce-motion', a.reduced_motion);
  root.classList.toggle('more-contrast', a.high_contrast);
  root.style.setProperty('--ts', String(a.text_pct / 100));
}

/** Follow later changes made in Settings while this window is open. */
export function followA11y(onChange?: (a: Accessibility) => void): void {
  listen<Accessibility>('hy://a11y', (e) => {
    applyA11y(e.payload);
    onChange?.(e.payload);
  }).catch(report);
}
