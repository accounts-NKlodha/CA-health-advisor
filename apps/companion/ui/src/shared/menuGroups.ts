// How the pet menu and keyboard panel group the engine's entries into short sections
// (owner review: one long wrap of buttons read as clutter). Pure, so `node --test` can
// check it: every entry lands in exactly one place and keeps its label as its
// accessible name; durations that share a word ("Focus 30 min", "Pause 1 hour") become
// one row of small segmented buttons under that word.
import type { MenuAction } from './types';

export type SectionId = 'top' | 'water' | 'movement' | 'focus' | 'pause' | 'more';

/** One segment of a segmented row: the short text shown and the entry behind it. */
export interface Segment {
  action: MenuAction;
  /** What the button shows ("30 min"); the entry's full label stays its accessible name. */
  short: string;
}

/** A row of small buttons under a word ("Focus", "Pomodoro", "Pause"). */
export interface SegmentRow {
  kind: 'segments';
  /** The row's word, plus a qualifier every segment shares ("Focus (ends Pomodoro)"). */
  label: string;
  segments: Segment[];
}

/** An ordinary full button. */
export interface ButtonItem {
  kind: 'button';
  action: MenuAction;
}

export type Item = SegmentRow | ButtonItem;

export interface Section {
  id: SectionId;
  /** The section heading; empty for the untitled top row (a waiting reminder). */
  title: string;
  items: Item[];
}

export const SECTION_TITLES: Record<SectionId, string> = {
  top: '',
  water: 'Water',
  movement: 'Movement & breaks',
  focus: 'Focus',
  pause: 'Pause',
  more: 'More',
};

type Row = 'focus' | 'pomodoro' | 'pause';

interface Place {
  section: SectionId;
  /** Order inside the section (lower first); entries keep the engine's order within. */
  rank: number;
  /** The segmented row this entry joins, if any. */
  row?: Row;
}

const ROW_WORD: Record<Row, string> = { focus: 'Focus', pomodoro: 'Pomodoro', pause: 'Pause' };

/** Where an entry goes, from its id (the ids are the engine's stable action protocol). */
export function placeOf(id: string): Place {
  if (id === 'open_pending') return { section: 'top', rank: 0 };
  if (id.startsWith('water_')) return { section: 'water', rank: 0 };
  if (id.startsWith('movement_') || id === 'break') return { section: 'movement', rank: 0 };
  if (id === 'phone_note') return { section: 'movement', rank: 1 };
  if (/^focus_\d+/.test(id)) return { section: 'focus', rank: 0, row: 'focus' };
  if (id === 'focus_end') return { section: 'focus', rank: 1 };
  if (/^pomodoro_\d+/.test(id)) return { section: 'focus', rank: 2, row: 'pomodoro' };
  if (id.startsWith('pomodoro_')) return { section: 'focus', rank: 3 };
  if (id === 'deadline_day' || id === 'recovery_show') return { section: 'focus', rank: 4 };
  if (id === 'meeting_on' || id === 'meeting_off') return { section: 'pause', rank: 0 };
  if (id === 'pause_30' || id === 'pause_60' || id === 'pause_today' || id === 'pause_on') {
    return { section: 'pause', rank: 1, row: 'pause' };
  }
  if (id === 'resume') return { section: 'pause', rank: 2 };
  // Anything new the engine adds still shows, under More.
  return { section: 'more', rank: 0 };
}

function capitalise(s: string): string {
  return s.length > 0 ? s[0]!.toUpperCase() + s.slice(1) : s;
}

/** Split "Focus 30 min (ends Pomodoro)" into the short text and its qualifier. */
function split(label: string, word: string): { short: string; note: string } {
  let rest = label.startsWith(`${word} `) ? label.slice(word.length + 1) : label;
  let note = '';
  const m = /^(.*?)\s*(\([^()]*\))$/.exec(rest);
  if (m) {
    rest = m[1]!;
    note = m[2]!;
  }
  return { short: capitalise(rest), note };
}

function segmentRow(row: Row, actions: MenuAction[]): SegmentRow {
  const word = ROW_WORD[row];
  const parts = actions.map((a) => ({ a, ...split(a.label, word) }));
  const shared = parts.every((p) => p.note === parts[0]!.note) ? parts[0]!.note : null;
  return {
    kind: 'segments',
    label: shared ? `${word} ${shared}` : word,
    segments: parts.map((p) => ({
      action: p.a,
      // A qualifier that is not shared stays on its own segment, so no warning is lost.
      short: shared !== null || !p.note ? p.short : `${p.short} ${p.note}`,
    })),
  };
}

const ORDER: SectionId[] = ['top', 'water', 'movement', 'focus', 'pause', 'more'];

/** Group the engine's entries into sections, in display order, dropping empty ones. */
export function groupActions(actions: MenuAction[]): Section[] {
  const buckets = new Map<SectionId, { rank: number; seq: number; item: Item }[]>();
  const rows = new Map<Row, { rank: number; seq: number; actions: MenuAction[] }>();
  actions.forEach((a, seq) => {
    const place = placeOf(a.id);
    const list = buckets.get(place.section) ?? [];
    buckets.set(place.section, list);
    if (place.row) {
      const existing = rows.get(place.row);
      if (existing) {
        existing.actions.push(a);
        return;
      }
      const entry = { rank: place.rank, seq, actions: [a] };
      rows.set(place.row, entry);
      // Placeholder replaced once the row is complete.
      list.push({ rank: place.rank, seq, item: { kind: 'segments', label: place.row, segments: [] } });
      return;
    }
    list.push({ rank: place.rank, seq, item: { kind: 'button', action: a } });
  });
  const out: Section[] = [];
  for (const id of ORDER) {
    const list = buckets.get(id);
    if (!list || list.length === 0) continue;
    list.sort((x, y) => x.rank - y.rank || x.seq - y.seq);
    const items = list.map(({ item }) => {
      if (item.kind === 'segments' && item.segments.length === 0) {
        const row = item.label as Row;
        return segmentRow(row, rows.get(row)!.actions);
      }
      return item;
    });
    out.push({ id, title: SECTION_TITLES[id], items });
  }
  return out;
}

/** Clamp and snap a size to the slider's steps. */
export function snapSize(pct: number, min: number, max: number, step: number): number {
  const snapped = Math.round((pct - min) / step) * step + min;
  return Math.min(max, Math.max(min, snapped));
}

/**
 * The size after a wheel turn: wheel down (positive deltaY) makes the pet smaller, one
 * step per notch.
 */
export function wheelSize(
  current: number,
  deltaY: number,
  min: number,
  max: number,
  step: number,
): number {
  if (deltaY === 0) return snapSize(current, min, max, step);
  return snapSize(current + (deltaY > 0 ? -step : step), min, max, step);
}
