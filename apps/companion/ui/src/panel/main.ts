// The keyboard pet panel (wireframe K1). The hotkey opens it and it takes focus on
// purpose; Tab moves between actions, Escape closes it and the shell hands focus back to
// the window that had it. Actions that open another window (Settings) leave focus there.
import { call, on, report } from '../shared/ipc';
import { renderMenu } from '../shared/menu';
import type { PanelView } from '../shared/types';

const root = document.getElementById('panel') as HTMLElement;

function close(restore: boolean): void {
  call('panel_close', { restore }).catch(report);
}

function draw(v: PanelView, keepFocus: boolean): void {
  // Remember which control had focus so a redraw (after a size change) keeps it.
  const active = document.activeElement;
  const focusLabel = active instanceof HTMLButtonElement ? active.getAttribute('aria-label') ?? active.textContent : null;

  const state = document.createElement('p');
  state.className = 'panel-state';
  state.textContent = v.accessible_name;
  const menu = renderMenu(
    v.menu,
    {
      action: (id) => {
        call('menu_action', { actionId: id })
          .then(() => close(true))
          .catch(report);
      },
      size: (pct) => void call('pet_edit', { edit: { kind: 'size', pct } }).catch(report),
      bring: () => {
        call('bring_pet')
          .then(() => close(true))
          .catch(report);
      },
      hide: () => {
        call('hide_pet')
          .then(() => close(true))
          .catch(report);
      },
      settings: () => {
        call('open_settings')
          .then(() => close(false))
          .catch(report);
      },
    },
    'panel-title',
  );
  const hint = document.createElement('p');
  hint.className = 'hint';
  hint.textContent = v.panel_hint;
  root.replaceChildren(state, menu, hint);

  const buttons = Array.from(root.querySelectorAll<HTMLButtonElement>('button'));
  const again =
    keepFocus && focusLabel
      ? buttons.find((b) => (b.getAttribute('aria-label') ?? b.textContent) === focusLabel)
      : undefined;
  (again ?? buttons[0])?.focus();
}

document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape') {
    e.preventDefault();
    close(true);
  }
});
document.addEventListener('contextmenu', (e) => e.preventDefault());

async function start(): Promise<void> {
  await on('panel://view', (v) => draw(v, true));
  draw(await call('panel_bootstrap'), false);
}

start().catch(report);
