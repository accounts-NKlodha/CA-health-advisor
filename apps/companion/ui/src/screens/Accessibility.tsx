// Settings > Accessibility (wireframe W12 and section 6): text size in bubbles and
// screens (100, 125 or 150%, on top of Windows display scaling), reduced motion, higher
// contrast, and the keyboard keys. Changes apply at once to the pet, the panel and every
// open window.
import { call } from '../shared/ipc';
import type { Accessibility, SettingsPage } from '../shared/types';
import type { Save } from './main';

const SIZES = [100, 125, 150];

export function AccessibilityTab({ page, save }: { page: SettingsPage; save: Save }) {
  const a = page.a11y;
  const set = (change: Partial<Accessibility>, done: string) =>
    save(call('save_accessibility', { a11y: { ...a, ...change } }), done);
  return (
    <main class="pane">
      <h2>Accessibility</h2>
      <section aria-labelledby="text-h">
        <h3 id="text-h">Text size in bubbles and screens</h3>
        <div class="seg" role="group" aria-labelledby="text-h">
          {SIZES.map((pct) => (
            <button
              type="button"
              class="seg-btn"
              key={pct}
              aria-pressed={a.text_pct === pct}
              onClick={() => set({ text_pct: pct }, `Text size ${pct}%.`)}
            >
              {pct}%
            </button>
          ))}
        </div>
        <p class="note">On top of Windows display scaling. Windows scroll when the text needs more room.</p>
        <label class="check block">
          <input
            type="checkbox"
            role="switch"
            checked={a.reduced_motion}
            onChange={(e) => {
              const on = e.currentTarget.checked;
              set({ reduced_motion: on }, on ? 'Reduced motion on.' : 'Reduced motion off.');
            }}
          />
          <span>Reduced motion: still poses and no animation (Windows' "Show animations" setting is followed as well)</span>
        </label>
        <label class="check block">
          <input
            type="checkbox"
            role="switch"
            checked={a.high_contrast}
            onChange={(e) => {
              const on = e.currentTarget.checked;
              set({ high_contrast: on }, on ? 'Higher contrast on.' : 'Higher contrast off.');
            }}
          />
          <span>Higher contrast text and borders; the pet is drawn solid</span>
        </label>
      </section>
      <section aria-labelledby="keys-h">
        <h3 id="keys-h">Keyboard</h3>
        <table class="plain">
          <tbody>
            <tr>
              <td>{page.hotkey ? <kbd>{page.hotkey}</kbd> : 'Tray menu'}</td>
              <td>
                Open the pet panel (it takes focus on purpose){page.hotkey ? '' : '; the shortcut is off (Settings, Pet)'}
              </td>
            </tr>
            <tr>
              <td>
                <kbd>Tab</kbd> / <kbd>Shift+Tab</kbd>
              </td>
              <td>Move between actions</td>
            </tr>
            <tr>
              <td>
                <kbd>Enter</kbd> / <kbd>Space</kbd>
              </td>
              <td>Choose</td>
            </tr>
            <tr>
              <td>
                <kbd>Esc</kbd>
              </td>
              <td>Close the panel and return focus to your previous app</td>
            </tr>
          </tbody>
        </table>
        <p class="note">Status always uses an icon or words, never colour alone. Charts have a table view.</p>
      </section>
    </main>
  );
}
