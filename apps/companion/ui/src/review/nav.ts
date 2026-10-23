// Where the review window is: its tab, the selected week and month. Every load (the first,
// a focus refresh, a tab change) asks for exactly this, so the week the review offer's
// "Open" named stays selected until the user moves. Pure (no imports at run time), tested
// with node:test in ui/test.
export type Tab = 'week' | 'month';

export interface Nav {
  tab: Tab;
  /** A day of the selected week (null: this week). */
  day: string | null;
  /** The selected month, "YYYY-MM" (null: this month). */
  month: string | null;
}

export type NavAction =
  | { type: 'target'; week: string }
  | { type: 'tab'; tab: Tab }
  | { type: 'week'; day: string | null }
  | { type: 'month'; month: string | null };

export const START: Nav = { tab: 'week', day: null, month: null };

export function navigate(nav: Nav, a: NavAction): Nav {
  switch (a.type) {
    case 'target':
      // "Open" on the weekly review offer: that week, kept in the page's own state so a
      // focus refresh or a tab change never falls back to this week.
      return { ...nav, tab: 'week', day: a.week };
    case 'tab':
      return { ...nav, tab: a.tab };
    case 'week':
      return { ...nav, day: a.day };
    case 'month':
      return { ...nav, month: a.month };
  }
}

/** What to load for `nav` (the first load, a focus refresh and a tab change alike). */
export function request(nav: Nav): { command: 'review_week'; day: string | null } | { command: 'review_month'; month: string | null } {
  return nav.tab === 'week' ? { command: 'review_week', day: nav.day } : { command: 'review_month', month: nav.month };
}
