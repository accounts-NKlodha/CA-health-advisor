// Parts of an engine card shared by the pet's bubble and the keyboard panel: the progress
// bar and the rows with their own actions (the water panel, wireframe B3). Everything is
// built with DOM calls and classes; no inline styles (the CSP allows none).
import type { Bubble, BubbleAction } from './types';

/** Makes the button for one action; `describedBy` names the element giving its context. */
export type ActionButton = (a: BubbleAction, describedBy: string | null) => HTMLButtonElement;

let nextId = 0;

/** The card's progress bar: a native `<progress>` with its value in words beside it. */
export function renderProgress(b: Bubble): HTMLElement | null {
  if (!b.progress) return null;
  const wrap = document.createElement('div');
  wrap.className = 'meter';
  const bar = document.createElement('progress');
  bar.max = b.progress.max;
  bar.value = Math.min(b.progress.value, b.progress.max);
  bar.setAttribute('aria-label', b.progress.label);
  const text = document.createElement('span');
  text.className = 'meter-label';
  text.textContent = b.progress.label;
  text.setAttribute('aria-hidden', 'true');
  wrap.append(bar, text);
  return wrap;
}

/**
 * The card's rows (today's entries): each a line of text and its own actions, which
 * are described by that line so "Edit" says which entry it edits.
 */
export function renderRows(b: Bubble, button: ActionButton): HTMLElement | null {
  if (b.rows.length === 0) return null;
  const ul = document.createElement('ul');
  ul.className = 'entries';
  for (const r of b.rows) {
    const li = document.createElement('li');
    li.className = 'entry';
    const text = document.createElement('span');
    text.className = 'entry-text';
    text.id = `entry-${++nextId}`;
    text.textContent = r.text;
    li.append(text);
    if (r.actions.length > 0) {
      const actions = document.createElement('span');
      actions.className = 'entry-actions';
      for (const a of r.actions) actions.append(button(a, text.id));
      li.append(actions);
    }
    ul.append(li);
  }
  return ul;
}
