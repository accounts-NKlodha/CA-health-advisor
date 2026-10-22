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

export interface Bubble {
  id: string;
  title: string;
  lines: string[];
  chips: BubbleChip[];
  actions: BubbleAction[];
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
