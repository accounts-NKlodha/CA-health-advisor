// Views sent by the Rust shell (apps/companion/src-tauri/src/view.rs). Every view is a
// complete snapshot. Field names follow the Rust structs (snake_case); `Option` is `null`.

export type Edge = 'left' | 'right' | 'free';
export type Tone = 'gentle' | 'plain' | 'cheerful';
export type Side = 'left' | 'right';
export type Animation = 'none' | 'bob' | 'wiggle' | 'hop' | 'stretch';

export interface ColourVariant {
  name: string;
  a: string;
  b: string;
}

export interface Hitbox {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface StatePose {
  animation: Animation;
  eyes: 'open' | 'closed';
  filter: 'none' | 'desaturate';
}

export type Badge =
  | { kind: 'count'; value: number }
  | {
      kind:
        | 'exclaim'
        | 'bell'
        | 'move'
        | 'star'
        | 'snooze'
        | 'focus'
        | 'timer'
        | 'moon'
        | 'mute'
        | 'pause'
        | 'plug';
    };

export interface StateLook {
  state: string;
  base: string;
  badge: Badge | null;
  tag: string | null;
  dim: boolean;
  eyes_closed: boolean;
  label: string;
}

export interface SizePreset {
  pct: number;
  short: string;
  label: string;
}

export interface MenuAction {
  id: string;
  label: string;
  in_tray: boolean;
  /** The entry opens a card, so the keyboard panel stays open to show it. */
  continues: boolean;
}

export interface MenuView {
  greeting: string;
  actions: MenuAction[];
  size_pct: number;
  presets: SizePreset[];
  hint: string;
  hotkey: string | null;
}

export interface BubbleChip {
  label: string;
  status: string;
  highlighted: boolean;
}

export interface BubbleAction {
  id: string;
  label: string;
  style: 'normal' | 'primary' | 'link';
}

export interface BubbleRow {
  text: string;
  actions: BubbleAction[];
}

export interface BubbleProgress {
  value: number;
  max: number;
  /** The value in words, shown beside the bar and given as its accessible name. */
  label: string;
}

export interface BubbleFigure {
  id: string;
  label: string;
}

export interface Bubble {
  id: string;
  title: string;
  lines: string[];
  chips: BubbleChip[];
  actions: BubbleAction[];
  /** Numbered steps (the movement guide). */
  steps: string[];
  /** An illustration drawn by the page (the movement guide). */
  figure: BubbleFigure | null;
  /** A small note, for example that the content is provisional. */
  note: string | null;
  /** A progress bar (today's water against the user's own goal). */
  progress: BubbleProgress | null;
  /** Rows with their own actions (today's water entries). */
  rows: BubbleRow[];
  /** The card continues after an answer, so the keyboard panel stays open. */
  continues: boolean;
}

export type OpenView =
  | { kind: 'none' }
  | { kind: 'bubble'; token: number; bubble: Bubble }
  | { kind: 'notice'; token: number; bubble: Bubble }
  | { kind: 'menu'; token: number; menu: MenuView };

export interface LayoutView {
  pet_x: number;
  pet_y: number;
  bubble_x: number;
  bubble_y: number;
  measured_token: number;
  side: Side;
  bubble_max_w: number;
  bubble_max_h: number;
}

export interface PetView {
  name: string;
  pack_key: string;
  art_svg: string;
  colours: ColourVariant;
  hitbox: Hitbox;
  look: StateLook;
  pose: StatePose;
  accessible_name: string;
  motion: boolean;
  pet_px: number;
  open: OpenView;
  layout: LayoutView;
}

export interface PanelView {
  /** The engine's bubble open beside the pet, so it can be answered from the keyboard. */
  bubble: Bubble | null;
  menu: MenuView;
  panel_hint: string;
  accessible_name: string;
}

export interface PackCard {
  id: string;
  name: string;
  default_name: string;
  art_status: 'drawn' | 'pending_review';
  selectable: boolean;
  art_svg: string;
  palette: ColourVariant[];
  review: string;
}

export interface SettingsView {
  pet_id: string;
  name: string;
  size_pct: number;
  colour_variant: number;
  edge: Edge;
  motion: boolean;
  tone: Tone;
  packs: PackCard[];
  presets: SizePreset[];
  size_min: number;
  size_max: number;
  size_step: number;
  hotkey: string | null;
  hotkey_registered: boolean;
  hotkey_choices: string[];
}

export type PetEdit =
  | { kind: 'choose_pet'; pet_id: string }
  | { kind: 'rename'; name: string }
  | { kind: 'size'; pct: number }
  | { kind: 'colour'; variant: number }
  | { kind: 'edge'; edge: Edge }
  | { kind: 'motion'; on: boolean }
  | { kind: 'tone'; tone: Tone }
  | { kind: 'reset_defaults' };

// Reviews (wireframes W13 and W14; hy_core::review, sent as ReviewPage in view.rs).

export type Metric = 'movement' | 'breaks' | 'water';

/** How one day looks for one metric. "zero" is a measured zero; "no_data" is never zero. */
export type DayMark =
  | 'met'
  | 'partly'
  | 'zero'
  | 'nothing_due'
  | 'no_data'
  | 'off'
  | 'not_started'
  | 'today'
  | 'future';

export type PhoneLabel = 'unlabelled' | 'work' | 'personal' | 'unsure';

export interface Ratio {
  num: number;
  den: number;
  pct: number | null;
}

export interface Coverage {
  recorded: number;
  workdays: number;
}

export interface MetricRow {
  metric: Metric;
  marks: DayMark[];
  value: Ratio;
  met: number;
  partly: number;
  zero: number;
  no_data: number;
  nothing_due: number;
}

export interface PhoneCounts {
  work: number;
  personal: number;
  unsure: number;
  unlabelled: number;
}

export interface PhoneEntry {
  id: string;
  date: string;
  /** Local time, "HH:MM:SS". */
  time: string;
  label: PhoneLabel;
}

export interface WeekReview {
  start: string;
  end: string;
  today: string;
  status: 'before_start' | 'first_week' | 'not_enough_data' | 'normal';
  days: { date: string; workday: boolean }[];
  coverage: Coverage;
  rows: MetricRow[];
  tallies: {
    longest_block_s: number | null;
    focus_sessions: number;
    pomodoro_rounds: number;
    confirmed_breaks: number;
  };
  highlights: string[];
  comparison: {
    movement: Ratio;
    breaks: Ratio;
    water: Ratio;
    longest_block_s: number | null;
  } | null;
  phone: { show: boolean; counts: PhoneCounts; entries: PhoneEntry[] };
  previous_week: string | null;
  next_week: string | null;
}

export interface MonthBar {
  month: string;
  in_progress: boolean;
  coverage: Coverage;
  movement: Ratio;
  breaks: Ratio;
  water: Ratio;
}

export interface MonthDay {
  date: string;
  workday: boolean;
  movement: DayMark;
  breaks: DayMark;
  water: DayMark;
}

export interface MonthReview {
  month: string;
  today: string;
  status: 'too_early' | 'normal';
  days_used: number;
  bars: MonthBar[];
  baseline: { from: string; to: string; movement: Ratio; breaks: Ratio; water: Ratio } | null;
  days: MonthDay[];
  previous_month: string | null;
  next_month: string | null;
}

export interface ReviewPage<T> {
  review: T;
  /** The in-app motion setting; Windows' reduced-motion setting is respected as well. */
  motion: boolean;
}
