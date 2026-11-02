// Parts of an engine card shared by the pet's bubble and the keyboard panel: the progress
// bar and the rows with their own actions (the water panel, wireframe B3). Everything is
// built with DOM calls and classes; no inline styles (the CSP allows none).
import { countdownText, secondsLeft } from './breaks';
import type { Bubble, BubbleAction } from './types';

/** Makes the button for one action; `describedBy` names the element giving its context. */
export type ActionButton = (a: BubbleAction, describedBy: string | null) => HTMLButtonElement;

let nextId = 0;

/**
 * The card's countdown (a step of the 5-minute break): the page counts down by itself
 * from when the card is drawn, so the card is not redrawn every second. It is a timer
 * that screen readers do not announce every second; the step's title says what it is.
 */
export function renderCountdown(b: Bubble): HTMLElement | null {
  const c = b.countdown;
  if (!c) return null;
  const p = document.createElement('p');
  p.className = 'countdown';
  p.setAttribute('role', 'timer');
  p.setAttribute('aria-live', 'off');
  p.setAttribute('aria-label', c.label);
  const drawn = Date.now();
  const tick = (): boolean => {
    const left = secondsLeft(c.seconds, drawn, Date.now());
    p.textContent = countdownText(left);
    return left > 0;
  };
  tick();
  const timer = window.setInterval(() => {
    // Stop once the card is gone or the time is up.
    if (!p.isConnected || !tick()) window.clearInterval(timer);
  }, 1000);
  return p;
}

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
