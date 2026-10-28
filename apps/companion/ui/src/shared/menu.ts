// The pet menu (wireframe B1), shared by the pet's bubble and the keyboard panel (K1).
// Under the greeting, today's screen time (DEC13). The engine's actions are grouped
// under short headings (Water, Movement & breaks, Focus, Pause; see menuGroups.ts), then the shell's own: Pet (the size slider with
// S/M/L/XL quick picks, Bring pet above clock, Hide pet) and More (Weekly review, the
// posture check's entry (V2: "Posture check" while off, "Turn camera off" while on),
// Settings).
import { icon } from './icons';
import { type Section, groupActions, sizeCommitter, snapSize, wheelSize } from './menuGroups';
import { screenToday } from './screenTime';
import type { MenuAction, MenuView } from './types';

export interface MenuHandlers {
  action(id: string): void;
  size(pct: number): void;
  bring(): void;
  hide(): void;
  review(): void;
  settings(): void;
  /** The posture check's entry; `on` is whether the camera is on now. */
  camera(on: boolean): void;
}

function button(label: string, onClick: () => void, cls = 'btn'): HTMLButtonElement {
  const b = document.createElement('button');
  b.type = 'button';
  b.className = cls;
  b.textContent = label;
  b.addEventListener('click', onClick);
  return b;
}

/** How long the wheel waits after the last notch before it saves the size. */
const WHEEL_SETTLE_MS = 250;
/** Every size save from the menu goes through here, so the latest interaction wins (a
 * preset chosen while a wheel save is pending cancels it). Shared across redraws. */
const sizes = sizeCommitter(
  {
    set: (fn, ms) => setTimeout(fn, ms),
    clear: (t) => clearTimeout(t as ReturnType<typeof setTimeout>),
  },
  WHEEL_SETTLE_MS,
);

function sectionEl(title: string, id: string): HTMLElement {
  const sec = document.createElement('section');
  sec.className = 'menu-section';
  if (title) {
    const h = document.createElement('h3');
    h.className = 'menu-heading';
    h.id = id;
    h.textContent = title;
    sec.setAttribute('aria-labelledby', id);
    sec.append(h);
  }
  return sec;
}

function actionButton(a: MenuAction, h: MenuHandlers, text: string, cls: string): HTMLButtonElement {
  const b = button(text, () => h.action(a.id), cls);
  // Focus returns to the same entry after a redraw.
  b.dataset.key = `menu:${a.id}`;
  if (text !== a.label) {
    // The full label stays the accessible name ("Focus 30 min", not "30 min").
    b.setAttribute('aria-label', a.label);
    b.title = a.label;
  }
  return b;
}

function renderSection(s: Section, h: MenuHandlers, headingId: string): HTMLElement {
  const sec = sectionEl(s.title, `${headingId}-${s.id}`);
  let row: HTMLElement | null = null;
  for (const item of s.items) {
    if (item.kind === 'button') {
      if (!row) {
        row = document.createElement('div');
        row.className = 'row';
        sec.append(row);
      }
      row.append(actionButton(item.action, h, item.action.label, 'btn'));
      continue;
    }
    row = null;
    const line = document.createElement('div');
    line.className = 'seg-line';
    const label = document.createElement('span');
    label.className = 'seg-label';
    label.id = `${headingId}-${s.id}-${item.segments[0]?.action.id ?? 'row'}`;
    label.textContent = item.label;
    const seg = document.createElement('div');
    seg.className = 'seg';
    seg.setAttribute('role', 'group');
    seg.setAttribute('aria-labelledby', label.id);
    for (const g of item.segments) seg.append(actionButton(g.action, h, g.short, 'seg-btn'));
    line.append(label, seg);
    sec.append(line);
  }
  return sec;
}

function sizeControl(menu: MenuView, h: MenuHandlers, headingId: string): HTMLElement {
  const { size_min: min, size_max: max, size_step: step } = menu;
  const box = document.createElement('div');
  box.className = 'size-ctl';

  const line = document.createElement('div');
  line.className = 'size-row';
  const label = document.createElement('label');
  label.textContent = 'Size';
  label.id = `${headingId}-size`;
  const slider = document.createElement('input');
  slider.type = 'range';
  slider.className = 'size-slider';
  slider.id = `${headingId}-size-slider`;
  slider.dataset.key = 'menu:size';
  slider.min = String(min);
  slider.max = String(max);
  slider.step = String(step);
  slider.value = String(menu.size_pct);
  slider.setAttribute('aria-label', 'Pet size');
  slider.title = 'Pet size: drag, use the arrow keys, or turn the mouse wheel here';
  label.htmlFor = slider.id;
  const pct = document.createElement('output');
  pct.className = 'muted size-pct';
  pct.htmlFor.add(slider.id);
  const show = (v: number): void => {
    slider.value = String(v);
    pct.textContent = `${v}%`;
    const preset = menu.presets.find((p) => p.pct === v);
    slider.setAttribute('aria-valuetext', `${v}%${preset ? `, ${preset.label}` : ''}`);
  };
  show(menu.size_pct);
  const save = (v: number): void => {
    if (v !== menu.size_pct) h.size(v);
  };
  slider.addEventListener('input', () => show(snapSize(Number(slider.value), min, max, step)));
  slider.addEventListener('change', () => {
    sizes.now(snapSize(Number(slider.value), min, max, step), save);
  });
  line.append(label, slider, pct);

  const seg = document.createElement('div');
  seg.className = 'seg size-presets';
  seg.setAttribute('role', 'group');
  seg.setAttribute('aria-label', 'Size presets');
  for (const p of menu.presets) {
    const b = button(
      p.short,
      () => {
        show(p.pct);
        sizes.now(p.pct, h.size);
      },
      'seg-btn',
    );
    b.setAttribute('aria-pressed', String(menu.size_pct === p.pct));
    b.setAttribute('aria-label', `${p.label} (${p.pct}%)`);
    b.title = `${p.label} (${p.pct}%)`;
    seg.append(b);
  }

  // The mouse wheel over the size control: down is smaller, one step per notch. The
  // size is saved once the wheel settles, so a quick spin resizes the pet once.
  box.addEventListener(
    'wheel',
    (e) => {
      e.preventDefault();
      const next = wheelSize(Number(slider.value), e.deltaY, min, max, step);
      show(next);
      sizes.later(next, save);
    },
    { passive: false },
  );

  box.append(line, seg);
  return box;
}

/** Build the menu's content. */
export function renderMenu(menu: MenuView, h: MenuHandlers, headingId: string): HTMLElement {
  const root = document.createElement('div');
  root.className = 'menu';

  const title = document.createElement('h2');
  title.id = headingId;
  title.textContent = menu.greeting;
  root.append(title);
  // Today's screen time, shown apart from the actions (DEC13).
  const screen = screenToday(menu.screen_time_min);
  if (screen) {
    const p = document.createElement('p');
    p.className = 'menu-screen muted';
    p.textContent = screen;
    root.append(p);
  }

  const groups = groupActions(menu.actions);
  const more = groups.find((g) => g.id === 'more');
  for (const s of groups) if (s !== more) root.append(renderSection(s, h, headingId));

  const pet = sectionEl('Pet', `${headingId}-pet`);
  pet.append(sizeControl(menu, h, headingId));
  const petRow = document.createElement('div');
  petRow.className = 'row';
  const bring = button('Bring pet above clock', h.bring, 'btn');
  bring.prepend(icon('pin', 14));
  petRow.append(bring, button('Hide pet', h.hide, 'btn'));
  pet.append(petRow);
  root.append(pet);

  const moreSec = more ? renderSection(more, h, headingId) : sectionEl('More', `${headingId}-more`);
  const links = document.createElement('div');
  links.className = 'row';
  const cam = button(menu.camera.label, () => h.camera(menu.camera.on), 'btn');
  cam.title = menu.camera.status;
  cam.setAttribute('aria-description', menu.camera.status);
  links.append(button('Weekly review', h.review, 'btn'), cam, button('Settings', h.settings, 'btn'));
  moreSec.append(links);
  root.append(moreSec);

  const hint = document.createElement('p');
  hint.className = 'hint';
  hint.textContent = menu.hint;
  root.append(hint);
  return root;
}
