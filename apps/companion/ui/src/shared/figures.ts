// The movement guide's illustrations (wireframe B4), ported from the approved prototype
// (revision 3). The movements are provisional content pending the content review; these
// drawings only show the motion: solid is the start pose, dashed is where to move, the
// orange arrow is the direction. With reduced motion the animation stops and the solid
// and dashed poses still explain the movement.
//
// The page's content security policy refuses inline styles, so every colour is a
// presentation attribute or a class in pet.css, and transform origins are classes too.
// The drawings are fixed strings in this file, parsed with the same checked SVG subset as
// pet art.
import { parseArt } from './art';

const FC = '#2d6a6e';
const FC2 = '#1f4f52';
const CH = '#b8c4c9';
const TXT = '#55646c';

const arrowHead = (x: number, y: number, a: number): string =>
  `<path class="fig-arh" d="M0 0 L-8 -4.5 L-8 4.5 Z" transform="translate(${x} ${y}) rotate(${a})"/>`;

const FIGURES: Record<string, () => string> = {
  neck: () => `<rect x="24" y="54" width="72" height="62" rx="12" fill="${CH}"/>
    <path d="M28 80 Q30 68 46 66 L74 66 Q90 68 92 80 L88 118 H32 Z" fill="${FC}"/><rect x="55" y="54" width="10" height="16" rx="4" fill="${FC2}"/>
    <g transform="rotate(-24 60 66)"><circle class="fig-gh" cx="60" cy="40" r="14"/></g><g transform="rotate(24 60 66)"><circle class="fig-gh" cx="60" cy="40" r="14"/></g>
    <g class="fig-tilt"><circle cx="60" cy="40" r="14" fill="${FC}"/><circle cx="55" cy="38" r="1.9" fill="#fff"/><circle cx="65" cy="38" r="1.9" fill="#fff"/><path d="M56 45 q4 3 8 0" stroke="#fff" stroke-width="1.6" fill="none" stroke-linecap="round"/></g>
    <path class="fig-ar" d="M53.9 18.4 A48 48 0 0 0 36 24.4"/>${arrowHead(36, 24.4, 152)}<path class="fig-ar" d="M66.1 18.4 A48 48 0 0 1 84 24.4"/>${arrowHead(84, 24.4, 28)}`,
  shoulders: () => `<rect x="28" y="50" width="7" height="50" rx="3" fill="${CH}"/><rect x="28" y="92" width="58" height="7" rx="3" fill="${CH}"/><rect x="54" y="99" width="5" height="17" fill="${CH}"/>
    <rect x="42" y="44" width="17" height="52" rx="8" fill="${FC}"/><circle cx="52" cy="29" r="11" fill="${FC}"/><circle cx="58.5" cy="27.5" r="1.7" fill="#fff"/>
    <rect x="44" y="84" width="40" height="11" rx="5" fill="${FC}"/><rect x="76" y="86" width="10" height="28" rx="5" fill="${FC}"/><rect x="76" y="110" width="17" height="6" rx="3" fill="${FC2}"/>
    <g transform="translate(1 -7)"><rect class="fig-gh" x="46" y="48" width="9" height="36" rx="4.5"/></g>
    <g class="fig-roll"><circle cx="50.5" cy="50" r="7" fill="${FC2}"/><rect x="46" y="48" width="9" height="36" rx="4.5" fill="${FC2}"/></g>
    <circle cx="50" cy="47" r="15" fill="none" stroke="#b86e00" stroke-width="1.6" stroke-dasharray="3 4" opacity=".8"/>
    <g class="fig-spin"><circle cx="50" cy="32" r="4" fill="#b86e00"/></g>
    <text x="92" y="40" font-size="9" fill="${TXT}">up</text><text x="12" y="44" font-size="9" fill="${TXT}">back</text><text x="64" y="72" font-size="9" fill="${TXT}">down</text>`,
  twist: () => `<text x="60" y="11" text-anchor="middle" font-size="9.5" fill="${TXT}">seen from above</text>
    <rect x="26" y="58" width="68" height="56" rx="12" fill="${CH}"/>
    <rect x="44" y="72" width="13" height="40" rx="6" fill="${FC}" opacity=".85"/><rect x="63" y="72" width="13" height="40" rx="6" fill="${FC}" opacity=".85"/>
    <ellipse cx="60" cy="73" rx="21" ry="9" fill="${FC2}"/>
    <g transform="rotate(35 60 66)"><ellipse class="fig-gh" cx="60" cy="62" rx="31" ry="10"/></g><g transform="rotate(-35 60 66)"><ellipse class="fig-gh" cx="60" cy="62" rx="31" ry="10"/></g>
    <g class="fig-twist"><ellipse cx="60" cy="62" rx="31" ry="10" fill="${FC}"/><circle cx="60" cy="60" r="10" fill="${FC2}"/><path d="M55.5 68 L60 76 L64.5 68Z" fill="${FC2}"/></g>
    <path class="fig-ar" d="M98.1 52.2 A40 40 0 0 1 100 66"/>${arrowHead(100, 66, 92)}<path class="fig-ar" d="M21.9 52.2 A40 40 0 0 0 20 66"/>${arrowHead(20, 66, 88)}
    <text x="60" y="30" text-anchor="middle" font-size="9" fill="${TXT}">hips stay facing forward</text>`,
  calf: () => `<line x1="8" y1="115" x2="114" y2="115" stroke="${CH}" stroke-width="2.5"/>
    <rect x="84" y="44" width="6" height="71" rx="3" fill="${CH}"/><rect x="84" y="76" width="28" height="6" rx="3" fill="${CH}"/><rect x="106" y="82" width="5" height="33" fill="${CH}"/>
    <g transform="translate(0 -7)"><circle class="fig-gh" cx="52" cy="20" r="10"/><rect class="fig-gh" x="45" y="32" width="14" height="40" rx="7"/></g>
    <g class="fig-rise"><circle cx="52" cy="20" r="10" fill="${FC}"/><circle cx="58" cy="19" r="1.7" fill="#fff"/><rect x="45" y="32" width="14" height="40" rx="7" fill="${FC}"/>
      <path d="M54 39 L70 55 L86 50" stroke="${FC2}" stroke-width="7" fill="none" stroke-linecap="round" stroke-linejoin="round"/><rect x="47" y="68" width="10" height="40" rx="5" fill="${FC}"/></g>
    <g class="fig-heel fig-heel-calf"><path d="M45 109 Q49 104 56 105 L70 110 Q72 114 68 114 L45 114 Z" fill="${FC2}"/></g>
    <path class="fig-ar" d="M26 82 V58"/>${arrowHead(26, 56, -90)}<text x="20" y="96" font-size="9" fill="${TXT}">rise</text>`,
  heel: () => `<line x1="8" y1="115" x2="114" y2="115" stroke="${CH}" stroke-width="2.5"/>
    <rect x="22" y="44" width="6" height="52" rx="3" fill="${CH}"/><rect x="22" y="86" width="56" height="6" rx="3" fill="${CH}"/><rect x="46" y="92" width="5" height="23" fill="${CH}"/>
    <rect x="32" y="38" width="15" height="50" rx="7" fill="${FC}"/><circle cx="40" cy="24" r="10" fill="${FC}"/><circle cx="46" cy="23" r="1.7" fill="#fff"/>
    <rect x="34" y="76" width="50" height="11" rx="5" fill="${FC}"/>
    <g transform="translate(0 -5)"><rect class="fig-gh" x="74" y="80" width="10" height="28" rx="5"/></g>
    <g class="fig-rise5"><rect x="74" y="80" width="10" height="28" rx="5" fill="${FC}"/></g>
    <g class="fig-heel fig-heel-seated"><path d="M72 109 Q76 104 83 105 L97 110 Q99 114 95 114 L72 114 Z" fill="${FC2}"/></g>
    <path class="fig-ar" d="M106 106 V88"/>${arrowHead(106, 86, -90)}<text x="94" y="80" font-size="9" fill="${TXT}">heels up</text>`,
};

/**
 * The figure for `id` with `label` as its text equivalent, or null for an unknown id (the
 * card then shows the steps and the label alone).
 */
export function renderFigure(id: string, label: string): HTMLElement | null {
  const draw = FIGURES[id];
  if (!draw) return null;
  const svg = parseArt(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 120 120">${draw()}</svg>`);
  if (!svg) return null;
  svg.setAttribute('class', 'fig-svg');
  // parseArt hides art from assistive technology; this drawing carries its own label.
  svg.removeAttribute('aria-hidden');
  svg.setAttribute('role', 'img');
  svg.setAttribute('aria-label', label);
  const wrap = document.createElement('figure');
  wrap.className = 'fig';
  wrap.append(svg);
  const cap = document.createElement('figcaption');
  cap.textContent = 'Solid: start. Dashed: move to. Arrow: direction.';
  wrap.append(cap);
  return wrap;
}
