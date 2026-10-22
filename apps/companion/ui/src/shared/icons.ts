// Badge icons (24 px view box, drawn in currentColor). Each badge is always paired with
// text (the tag or the accessible name), so status never depends on an icon or colour
// alone (wireframe rule 5).
import type { Badge } from './types';

const ICON: Record<string, string> = {
  bell: '<path d="M12 3a6 6 0 0 0-6 6v4l-2 3h16l-2-3V9a6 6 0 0 0-6-6zm-2 15a2 2 0 0 0 4 0z" fill="currentColor"/>',
  moon: '<path d="M14 3a8 8 0 1 0 7 11A7 7 0 0 1 14 3z" fill="currentColor"/>',
  snooze:
    '<text x="3" y="18" font-size="15" font-weight="700" fill="currentColor">z</text><text x="12" y="12" font-size="10" font-weight="700" fill="currentColor">z</text>',
  plug: '<path d="M4 4l16 16M9 7v4m6-4v4M7 11h10v2a5 5 0 0 1-10 0z" stroke="currentColor" stroke-width="2.2" fill="none" stroke-linecap="round"/>',
  pause:
    '<rect x="6" y="5" width="4" height="14" fill="currentColor"/><rect x="14" y="5" width="4" height="14" fill="currentColor"/>',
  focus:
    '<circle cx="12" cy="12" r="8" stroke="currentColor" stroke-width="2.4" fill="none"/><circle cx="12" cy="12" r="3" fill="currentColor"/>',
  timer:
    '<circle cx="12" cy="13" r="8" stroke="currentColor" stroke-width="2.2" fill="none"/><path d="M12 13V8M9 2h6" stroke="currentColor" stroke-width="2.2" stroke-linecap="round"/>',
  mute: '<path d="M3 9h4l5-4v14l-5-4H3zM16 9l5 6m0-6l-5 6" stroke="currentColor" stroke-width="2" fill="none" stroke-linejoin="round"/>',
  star: '<path d="M12 3l2.6 5.8 6.4.6-4.8 4.3 1.4 6.3L12 16.8 6.4 20l1.4-6.3L3 9.4l6.4-.6z" fill="currentColor"/>',
  move: '<circle cx="12" cy="4" r="2.5" fill="currentColor"/><path d="M12 7v7m0 0l-4 7m4-7l4 7M6 10h12" stroke="currentColor" stroke-width="2.4" stroke-linecap="round"/>',
  pin: '<path d="M12 2a6 6 0 0 0-6 6c0 4.5 6 12 6 12s6-7.5 6-12a6 6 0 0 0-6-6z" fill="currentColor"/><circle cx="12" cy="8" r="2.2" fill="#fff"/>',
};

/** An icon as an SVG element. */
export function icon(name: string, size = 16): SVGSVGElement {
  const tpl = document.createElement('template');
  tpl.innerHTML = `<svg width="${size}" height="${size}" viewBox="0 0 24 24" aria-hidden="true" focusable="false">${ICON[name] ?? ''}</svg>`;
  return tpl.content.firstElementChild as SVGSVGElement;
}

/** Fill `el` with the badge: a number, "!", or an icon. */
export function renderBadge(el: HTMLElement, badge: Badge | null): void {
  if (!badge) {
    el.hidden = true;
    el.replaceChildren();
    return;
  }
  el.hidden = false;
  if (badge.kind === 'count') el.textContent = String(badge.value);
  else if (badge.kind === 'exclaim') el.textContent = '!';
  else el.replaceChildren(icon(badge.kind));
}
