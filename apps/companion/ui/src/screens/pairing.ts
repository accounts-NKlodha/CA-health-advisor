// Pairing with the office server (V1-5 contract 4.2 and 12; wireframe W8): the pure
// parts of the Sharing & server tab, kept apart from the Preact view so node --test can
// check them.

import type { LinkView } from '../shared/types';

/** The slip as the user pasted or typed it: spaces, line breaks and the dashes between
 *  the printed groups of four are kept as typed; only surrounding space is dropped. The
 *  shell parses it and reports a mistyped slip before anything is sent. */
export function slipInput(text: string): string {
  return text.trim().replace(/\s+/g, ' ');
}

/** True if the text could be a slip, so "Pair" is enabled (the full check is the
 *  shell's: lengths, characters and the checksum). */
export function slipLooksComplete(text: string): boolean {
  const t = slipInput(text).toLowerCase();
  return t.startsWith('hy1-') && t.replace(/[\s-]/g, '').length >= 77;
}

/** The check code as it is read out: two groups of two. */
export function checkCodeText(code: string | null): string {
  if (!code) return '';
  return code.length === 4 ? `${code.slice(0, 2)} ${code.slice(2)}` : code;
}

/** The status line at the top of the tab. */
export function statusLine(v: LinkView): string {
  switch (v.state) {
    case 'none':
      return 'Not paired. Everything works on this PC; nothing is shared or sent.';
    case 'pending':
      return `Waiting for your administrator to confirm this PC (paired as ${v.alias ?? ''} with ${v.host ?? 'the office server'}). Nothing is shared yet.`;
    case 'confirmed': {
      const paired = `Paired as ${v.alias ?? ''} with ${v.host ?? 'the office server'}${v.confirmed_on ? ` since ${v.confirmed_on}` : ''}.`;
      if (v.hold) return `${paired} Server recovering: sharing paused.`;
      if (v.mismatch) return `${paired} Server history mismatch: sharing is paused. Tell your administrator.`;
      const on = v.sharing.filter((s) => s.enabled).length;
      return on === 0
        ? `${paired} Sharing stays off until you switch a category on.`
        : `${paired} Sharing ${on} ${on === 1 ? 'category' : 'categories'}${v.last_sync ? `; last synced ${v.last_sync}` : ''}.`;
    }
  }
}

/** PCs to announce: newly paired to this user since the user last looked. */
export function newPcs(v: LinkView): LinkView['devices'] {
  return v.devices.filter((d) => d.new && !d.this_pc);
}

/** The notice shown before a category is switched on (SHARING_NOTICE in server_link.rs). */
export function enableNotice(label: string): string {
  return `Share "${label}" with the office server? Only daily totals in this category are sent, from today on. Your administrator can see them. You can switch it off at any time; switching off stops sending at once.`;
}

/** The switch's words for one category. */
export function shareLine(s: { label: string; enabled: boolean; share_from: string | null; waiting: boolean }): string {
  if (s.enabled) return `${s.label}: on${s.share_from ? ` (from ${s.share_from})` : ''}`;
  if (s.waiting) return `${s.label}: waiting for the office server`;
  return `${s.label}: off`;
}
