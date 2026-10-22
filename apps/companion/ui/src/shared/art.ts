// Pet art insertion. The Rust loader already refuses anything outside a strict SVG
// subset (crates/hy-pet/src/svg.rs); this second check keeps the page safe even if a
// future loader were wrong. The HTML parser puts <svg> in the SVG namespace itself.

const ELEMENTS = new Set([
  'svg',
  'g',
  'path',
  'ellipse',
  'circle',
  'rect',
  'line',
  'polyline',
  'polygon',
  'text',
  'title',
  'desc',
]);

/** Parse checked art and return the <svg> element, or null if it is not usable. */
export function parseArt(svgText: string): SVGSVGElement | null {
  const tpl = document.createElement('template');
  tpl.innerHTML = svgText.trim();
  const root = tpl.content.firstElementChild;
  if (!(root instanceof SVGSVGElement)) return null;
  const all = [root, ...Array.from(root.querySelectorAll('*'))];
  for (const el of all) {
    if (!ELEMENTS.has(el.localName)) {
      el.remove();
      continue;
    }
    for (const attr of Array.from(el.attributes)) {
      const n = attr.name.toLowerCase();
      if (n.startsWith('on') || n === 'style' || n.includes('href')) el.removeAttribute(attr.name);
    }
  }
  root.setAttribute('aria-hidden', 'true');
  root.setAttribute('focusable', 'false');
  return root;
}

/** Put art into `container`, replacing what was there. */
export function insertArt(container: Element, svgText: string): void {
  const svg = parseArt(svgText);
  container.replaceChildren(...(svg ? [svg] : []));
}

/** Set the palette slots used by the art's hy-a and hy-b classes. */
export function setPalette(el: HTMLElement, a: string, b: string): void {
  el.style.setProperty('--pal-a', a);
  el.style.setProperty('--pal-b', b);
}
