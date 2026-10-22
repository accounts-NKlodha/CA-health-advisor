// The pet menu (wireframe B1), shared by the pet's bubble and the keyboard panel (K1).
// It lists the engine's actions (added by the coaching engine in a later slice), then the
// shell's own: size S/M/L/XL, Bring pet above clock, Hide pet and Settings.
import { icon } from './icons';
import type { MenuView } from './types';

export interface MenuHandlers {
  action(id: string): void;
  size(pct: number): void;
  bring(): void;
  hide(): void;
  settings(): void;
}

function button(label: string, onClick: () => void, cls = 'btn'): HTMLButtonElement {
  const b = document.createElement('button');
  b.type = 'button';
  b.className = cls;
  b.textContent = label;
  b.addEventListener('click', onClick);
  return b;
}

/** Build the menu's content. */
export function renderMenu(menu: MenuView, h: MenuHandlers, headingId: string): HTMLElement {
  const root = document.createElement('div');
  root.className = 'menu';

  const title = document.createElement('h2');
  title.id = headingId;
  title.textContent = menu.greeting;
  root.append(title);

  if (menu.actions.length > 0) {
    const row = document.createElement('div');
    row.className = 'row';
    for (const a of menu.actions) row.append(button(a.label, () => h.action(a.id)));
    root.append(row);
  }

  const sizeRow = document.createElement('div');
  sizeRow.className = 'size-row';
  const sizeLabel = document.createElement('span');
  sizeLabel.textContent = 'Size';
  sizeLabel.id = `${headingId}-size`;
  const seg = document.createElement('div');
  seg.className = 'seg';
  seg.setAttribute('role', 'group');
  seg.setAttribute('aria-labelledby', sizeLabel.id);
  for (const p of menu.presets) {
    const b = button(p.short, () => h.size(p.pct), 'seg-btn');
    b.setAttribute('aria-pressed', String(menu.size_pct === p.pct));
    b.setAttribute('aria-label', `${p.label} (${p.pct}%)`);
    b.title = `${p.label} (${p.pct}%)`;
    seg.append(b);
  }
  const pct = document.createElement('span');
  pct.className = 'muted';
  pct.textContent = `${menu.size_pct}%`;
  sizeRow.append(sizeLabel, seg, pct);
  root.append(sizeRow);

  const links = document.createElement('div');
  links.className = 'row';
  const bring = button('Bring pet above clock', h.bring, 'btn');
  bring.prepend(icon('pin', 14));
  links.append(bring, button('Hide pet', h.hide, 'btn'), button('Settings', h.settings, 'btn'));
  root.append(links);

  const hint = document.createElement('p');
  hint.className = 'hint';
  hint.textContent = menu.hint;
  root.append(hint);
  return root;
}
