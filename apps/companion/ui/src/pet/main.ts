// The pet page (architecture 4.1 and 7). It stays plain TypeScript to keep idle memory
// low. The Rust shell owns every decision (placement, what is open, state); this page
// draws the latest view, measures the bubble, and reports pointer input. The window never
// takes focus (WS_EX_NOACTIVATE), so clicks here never move the user's caret.
import { applyA11y } from '../shared/a11y';
import { insertArt, setPalette } from '../shared/art';
import { renderProgress, renderRows } from '../shared/bubble';
import { renderBadge } from '../shared/icons';
import { renderFigure } from '../shared/figures';
import { call, on, report } from '../shared/ipc';
import { renderMenu } from '../shared/menu';
import type { Bubble, BubbleAction, OpenView, PetView } from '../shared/types';

const VIEW_BOX = 128;
/** Pointer travel (CSS px) before a press becomes a drag. */
const DRAG_THRESHOLD = 4;

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  cls: string,
  parent?: Element,
): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  e.className = cls;
  parent?.append(e);
  return e;
}

const root = document.getElementById('pet') as HTMLElement;
const pet = el('div', 'pet', root);
const art = el('div', 'art', pet);
const hit = el('div', 'hit', pet);
const badge = el('span', 'badge', pet);
const tag = el('span', 'tag', pet);
const live = el('div', 'sr-only', root);
live.setAttribute('aria-live', 'polite');
badge.hidden = true;
tag.hidden = true;

let packKey = '';
let openKey = '';
let bubbleEl: HTMLElement | null = null;
let lastView: PetView | null = null;

function px(n: number): string {
  return `${Math.round(n)}px`;
}

// ------------------------------------------------------------------- pet drawing

function drawPet(v: PetView): void {
  if (v.pack_key !== packKey) {
    insertArt(art, v.art_svg);
    packKey = v.pack_key;
  }
  setPalette(pet, v.colours.a, v.colours.b);
  pet.style.left = px(v.layout.pet_x);
  pet.style.top = px(v.layout.pet_y);
  pet.style.width = px(v.pet_px);
  pet.style.height = px(v.pet_px);

  const unit = v.pet_px / VIEW_BOX;
  hit.style.left = px(v.hitbox.x * unit);
  hit.style.top = px(v.hitbox.y * unit);
  hit.style.width = px(v.hitbox.width * unit);
  hit.style.height = px(v.hitbox.height * unit);

  const anim = v.motion ? v.pose.animation : 'none';
  pet.className = [
    'pet',
    `anim-${anim}`,
    v.pose.eyes === 'closed' || v.look.eyes_closed ? 'eyes-closed' : '',
    v.pose.filter === 'desaturate' ? 'desaturate' : '',
    v.look.dim ? 'dim' : '',
  ]
    .filter(Boolean)
    .join(' ');
  pet.style.setProperty('--hy-scale', String(v.pet_px / VIEW_BOX));

  renderBadge(badge, v.look.badge);
  tag.hidden = !v.look.tag;
  tag.textContent = v.look.tag ?? '';
  root.setAttribute('aria-label', v.accessible_name);
}

// ------------------------------------------------------------------ bubble drawing

function button(label: string, cls: string, onClick: () => void): HTMLButtonElement {
  const b = el('button', cls);
  b.type = 'button';
  b.textContent = label;
  b.addEventListener('click', onClick);
  return b;
}

function bubbleBody(b: Bubble, engine: boolean): HTMLElement {
  const body = el('div', 'bubble-body');
  const h = el('h2', '', body);
  h.textContent = b.title;
  const action = (a: BubbleAction, describedBy: string | null): HTMLButtonElement => {
    const cls = a.style === 'primary' ? 'btn primary' : a.style === 'link' ? 'link' : 'btn';
    const btn = button(a.label, cls, () => {
      const p = engine
        ? call('bubble_action', { bubbleId: b.id, actionId: a.id })
        : call('bubble_close');
      p.catch(report);
    });
    if (describedBy) btn.setAttribute('aria-describedby', describedBy);
    return btn;
  };
  const meter = renderProgress(b);
  if (meter) body.append(meter);
  if (b.steps.length > 0 || b.figure) {
    // The movement guide: the illustration beside the numbered steps.
    const guide = el('div', 'guide', body);
    if (b.figure) {
      const fig = renderFigure(b.figure.id, b.figure.label);
      if (fig) guide.append(fig);
    }
    if (b.steps.length > 0) {
      const ol = el('ol', 'steps', guide);
      for (const s of b.steps) el('li', '', ol).textContent = s;
    }
  }
  for (const line of b.lines) el('p', '', body).textContent = line;
  if (b.chips.length > 0) {
    const ul = el('ul', 'chips', body);
    for (const c of b.chips) {
      const li = el('li', c.highlighted ? 'chip on' : 'chip', ul);
      const name = el('span', 'chip-label', li);
      name.textContent = c.label;
      const status = el('span', 'chip-status', li);
      status.textContent = c.status;
    }
  }
  const rows = renderRows(b, action);
  if (rows) body.append(rows);
  if (b.actions.length > 0) {
    const row = el('div', 'row', body);
    for (const a of b.actions) row.append(action(a, null));
  }
  if (b.note) el('p', 'note', body).textContent = b.note;
  return body;
}

function openContent(open: OpenView): HTMLElement | null {
  switch (open.kind) {
    case 'none':
      return null;
    case 'bubble':
      return bubbleBody(open.bubble, true);
    case 'notice':
      return bubbleBody(open.bubble, false);
    case 'menu':
      return renderMenu(
        open.menu,
        {
          action: (id) => void call('menu_action', { actionId: id }).catch(report),
          size: (pct) => void call('pet_edit', { edit: { kind: 'size', pct } }).catch(report),
          bring: () => void call('bring_pet').catch(report),
          hide: () => void call('hide_pet').catch(report),
          review: () => void call('open_review').catch(report),
          settings: () => void call('open_settings').catch(report),
        },
        'menu-title',
      );
  }
}

function drawOpen(v: PetView): void {
  const open = v.open;
  const token = open.kind === 'none' ? 0 : open.token;
  // Rebuild the content only when something new opened or the menu's content changed.
  const key = open.kind === 'none' ? '' : `${token}:${JSON.stringify(open)}`;
  if (key !== openKey) {
    openKey = key;
    bubbleEl?.remove();
    bubbleEl = null;
    const content = openContent(open);
    if (content) {
      bubbleEl = el('section', 'bubble', root);
      bubbleEl.setAttribute('role', open.kind === 'menu' ? 'menu' : 'status');
      bubbleEl.append(content);
      bubbleEl.append(
        button('×', 'close', () => void call('bubble_close').catch(report)),
      );
      const close = bubbleEl.lastElementChild as HTMLButtonElement;
      close.setAttribute('aria-label', 'Close');
      close.title = 'Close';
      if (open.kind === 'bubble' || open.kind === 'notice') {
        const b = open.bubble;
        live.textContent = [b.title, ...b.steps, ...b.lines].join('. ');
      }
    }
  }
  if (!bubbleEl) return;
  bubbleEl.style.maxWidth = px(v.layout.bubble_max_w);
  bubbleEl.style.maxHeight = px(v.layout.bubble_max_h);
  bubbleEl.classList.toggle('side-left', v.layout.side === 'left');
  bubbleEl.classList.toggle('side-right', v.layout.side === 'right');
  if (v.layout.measured_token === token) {
    bubbleEl.classList.remove('measuring');
    bubbleEl.style.left = px(v.layout.bubble_x);
    bubbleEl.style.top = px(v.layout.bubble_y);
  } else {
    // Measure at the largest allowed width, off to the side and invisible, then tell
    // the shell, which grows the window and sends the bubble's place.
    bubbleEl.classList.add('measuring');
    bubbleEl.style.left = '0px';
    bubbleEl.style.top = '0px';
    const target = bubbleEl;
    requestAnimationFrame(() => {
      if (bubbleEl !== target) return;
      const r = target.getBoundingClientRect();
      call('bubble_measured', {
        token,
        width: Math.ceil(r.width),
        height: Math.ceil(r.height),
      }).catch(report);
    });
  }
}

function draw(v: PetView): void {
  lastView = v;
  applyA11y(v.a11y);
  drawPet(v);
  drawOpen(v);
}

// ------------------------------------------------------------------- pointer input

let press: { id: number; x: number; y: number; moved: boolean; frame: number } | null = null;

hit.addEventListener('pointerdown', (e) => {
  if (e.button !== 0) return;
  e.preventDefault();
  hit.setPointerCapture(e.pointerId);
  press = { id: e.pointerId, x: e.screenX, y: e.screenY, moved: false, frame: 0 };
  call('drag_start').catch(report);
});

hit.addEventListener('pointermove', (e) => {
  if (!press || e.pointerId !== press.id) return;
  if (!press.moved) {
    const dx = e.screenX - press.x;
    const dy = e.screenY - press.y;
    if (dx * dx + dy * dy < DRAG_THRESHOLD * DRAG_THRESHOLD) return;
    press.moved = true;
    pet.classList.add('dragging');
  }
  // At most one move per frame; the shell reads the cursor itself.
  if (press.frame === 0) {
    const p = press;
    p.frame = requestAnimationFrame(() => {
      p.frame = 0;
      call('drag_move').catch(report);
    });
  }
});

function endPress(e: PointerEvent, cancelled: boolean): void {
  if (!press || e.pointerId !== press.id) return;
  const p = press;
  press = null;
  if (p.frame !== 0) cancelAnimationFrame(p.frame);
  pet.classList.remove('dragging');
  if (hit.hasPointerCapture(e.pointerId)) hit.releasePointerCapture(e.pointerId);
  // A cancelled press counts as a drag so it never opens the menu by accident.
  call('drag_end', { moved: p.moved || cancelled }).catch(report);
}

hit.addEventListener('pointerup', (e) => endPress(e, false));
hit.addEventListener('pointercancel', (e) => endPress(e, true));
// Context menu: the pet menu is the left click; the browser's own menu never shows.
document.addEventListener('contextmenu', (e) => e.preventDefault());

// ------------------------------------------------------------------------- start

async function start(): Promise<void> {
  await on('pet://view', draw);
  draw(await call('pet_bootstrap'));
  await call('pet_ready');
}

// Reduced-motion changes take effect without a new view.
matchMedia('(prefers-reduced-motion: reduce)').addEventListener('change', () => {
  if (lastView) drawPet(lastView);
});

start().catch(report);
