// The keyboard pet panel (wireframe K1). The hotkey opens it and it takes focus on
// purpose; Tab moves between actions, Escape closes it and the shell hands focus back to
// the window that had it. Actions that open another window (Settings) leave focus there.
// When the engine's bubble is open beside the pet (a reminder, a Pomodoro card, the
// check-in), the panel shows the same bubble first so it can be answered by keyboard.
import { renderFigure } from '../shared/figures';
import { call, on, report } from '../shared/ipc';
import { renderMenu } from '../shared/menu';
import type { Bubble, PanelView } from '../shared/types';

const root = document.getElementById('panel') as HTMLElement;

function close(restore: boolean): void {
  call('panel_close', { restore }).catch(report);
}

function bubbleSection(b: Bubble): HTMLElement {
  const section = document.createElement('section');
  section.className = 'panel-bubble';
  section.setAttribute('aria-labelledby', 'panel-bubble-title');
  const h = document.createElement('h2');
  h.id = 'panel-bubble-title';
  h.textContent = b.title;
  section.append(h);
  if (b.figure) {
    const fig = renderFigure(b.figure.id, b.figure.label);
    if (fig) section.append(fig);
  }
  if (b.steps.length > 0) {
    const ol = document.createElement('ol');
    ol.className = 'steps';
    for (const s of b.steps) {
      const li = document.createElement('li');
      li.textContent = s;
      ol.append(li);
    }
    section.append(ol);
  }
  for (const line of b.lines) {
    const p = document.createElement('p');
    p.textContent = line;
    section.append(p);
  }
  if (b.chips.length > 0) {
    const ul = document.createElement('ul');
    ul.className = 'chips';
    for (const c of b.chips) {
      const li = document.createElement('li');
      li.className = c.highlighted ? 'chip on' : 'chip';
      li.textContent = `${c.label}: ${c.status}`;
      ul.append(li);
    }
    section.append(ul);
  }
  const row = document.createElement('div');
  row.className = 'row';
  for (const a of b.actions) {
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = a.style === 'primary' ? 'btn primary' : 'btn';
    btn.textContent = a.label;
    btn.addEventListener('click', () => {
      // A card that continues (the movement guide's next screen) keeps the panel open;
      // the new card arrives as a fresh panel view.
      call('bubble_action', { bubbleId: b.id, actionId: a.id })
        .then(() => {
          if (!b.continues) close(true);
        })
        .catch(report);
    });
    row.append(btn);
  }
  const later = document.createElement('button');
  later.type = 'button';
  later.className = 'btn';
  later.textContent = 'Close this card';
  later.addEventListener('click', () => {
    call('bubble_close')
      .then(() => close(true))
      .catch(report);
  });
  row.append(later);
  section.append(row);
  if (b.note) {
    const note = document.createElement('p');
    note.className = 'note';
    note.textContent = b.note;
    section.append(note);
  }
  return section;
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
  if (v.bubble) {
    root.replaceChildren(state, bubbleSection(v.bubble), menu, hint);
  } else {
    root.replaceChildren(state, menu, hint);
  }

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
