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
  /** The posture check's entry (V2). */
  camera: CameraMenu;
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

/** Display preferences (Settings, Accessibility; wireframe W12). */
export interface Accessibility {
  /** 100, 125 or 150. */
  text_pct: number;
  reduced_motion: boolean;
  high_contrast: boolean;
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
  a11y: Accessibility;
  pet_px: number;
  open: OpenView;
  layout: LayoutView;
  /** The "CAM" badge while the camera may be capturing (V2), else null. */
  camera: CameraBadge | null;
}

export interface PanelView {
  /** The engine's bubble open beside the pet, so it can be answered from the keyboard. */
  bubble: Bubble | null;
  menu: MenuView;
  panel_hint: string;
  accessible_name: string;
  a11y: Accessibility;
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
  /** "Try next week" (up to 3). */
  suggestions: Suggestion[];
  previous_week: string | null;
  next_week: string | null;
}

/** How a monthly bar is drawn: decided by the engine from the metric's own count. */
export type BarKind = 'no_data' | 'nothing_to_count' | 'zero' | 'measured';

export interface MonthBar {
  month: string;
  in_progress: boolean;
  coverage: Coverage;
  movement: Ratio;
  breaks: Ratio;
  water: Ratio;
  kind: Record<Metric, BarKind>;
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
  /** Animation allowed (the pet's Motion switch on, reduced motion off); Windows'
   *  reduced-motion setting is respected as well. */
  motion: boolean;
  a11y: Accessibility;
}

// "Try next week" on the weekly review (wireframe W13; hy_core::review::suggest).

export type SuggestionKind = 'serving' | 'seated' | 'standing' | 'lighter' | 'keep';
export type SuggestionAnswer = 'apply' | 'undo' | 'hide';

export type Suggestion = { kind: SuggestionKind; text: string } & (
  | { state: 'open' }
  | { state: 'info' }
  | { state: 'applied'; now: string; was: string }
  | { state: 'hidden' }
);

// Settings tabs (wireframes W4 to W7 and W12; SettingsPage in view.rs).

export type Intensity = 'light' | 'balanced' | 'more_often';
export type MovementVersion = 'ask' | 'seated' | 'standing';
export type WeekdayName = 'mon' | 'tue' | 'wed' | 'thu' | 'fri' | 'sat' | 'sun';

export interface CoachConfig {
  intensity: Intensity;
  categories: { movement: boolean; eye_rest: boolean; water: boolean };
  hide_during_presentation: boolean;
  media_in_use_hint: boolean;
  long_day_check_in: { enabled: boolean; hours: number };
  pomodoro_auto_start: boolean;
  movement_version: MovementVersion;
  /** "HH:MM". */
  review: { day: WeekdayName; at: string };
  /** The meditation pause's daily invitation, "HH:MM", or null while off. */
  meditation: { at: string | null };
}

export interface WorkSchedule {
  workdays: WeekdayName[];
  /** "HH:MM". */
  start: string;
  end: string;
}

/** One coaching setting, as a page changes it (hy_core::settings::CoachChange). A page
 *  never sends its whole cached copy: another window may have changed other fields. */
export type CoachChange =
  | { intensity: Intensity }
  | { movement: boolean }
  | { eye_rest: boolean }
  | { water: boolean }
  | { hide_during_presentation: boolean }
  | { media_in_use_hint: boolean }
  | { long_day_check_in: boolean }
  | { long_day_hours: number }
  | { pomodoro_auto_start: boolean }
  | { movement_version: MovementVersion }
  | { review: { day: WeekdayName; at: string } }
  | { meditation: { at: string | null } };

/** One Water tab change (hy_core::settings::WaterChange). */
export type WaterChange =
  | { goal: number }
  | { add_serving: number }
  | { remove_serving: number }
  | { edit_servings: { add: number[]; remove: number[] } };

/** The workweek fields a save changed; the others keep what is saved now. */
export interface WorkweekChange {
  workdays?: WeekdayName[];
  /** "HH:MM". */
  hours?: { start: string; end: string };
  zone?: string;
}

export interface SettingsPage {
  /** Settings changes counted since the app started; the newest answer wins. */
  revision: number;
  coach: CoachConfig;
  schedule: WorkSchedule;
  zone: string;
  zones: string[];
  water_goal_ml: number | null;
  goal_min_ml: number;
  goal_max_ml: number;
  servings: number[];
  serving_min_ml: number;
  serving_max_ml: number;
  max_servings: number;
  a11y: Accessibility;
  programme: { day: string; movements: string[] }[];
  library: { name: string; place: string; dose: string; seated: string | null }[];
  provisional: string | null;
  hotkey: string | null;
  /** Wireframe W8. */
  sharing: { category: string; label: string; enabled: boolean }[];
  /** Where exports are written (wireframe W9). */
  backup_dir: string | null;
}

// Pairing with the office server (wireframe W8; LinkView in server_link.rs).

export interface PairedPc {
  device_ref: string;
  label: string;
  confirmed: boolean;
  this_pc: boolean;
  /** Newly paired since the user last looked: show the new-PC notice. */
  new: boolean;
}

/** One optional category once this PC is confirmed (ShareState in server_link.rs). */
export interface ShareState {
  category: string;
  label: string;
  enabled: boolean;
  /** Shared from this date while on. */
  share_from: string | null;
  /** Switched on here, waiting for the office server. */
  waiting: boolean;
  /** This version builds records for it; false: shown as "not yet sharing". */
  available: boolean;
}

export interface LinkView {
  state: 'none' | 'pending' | 'confirmed';
  alias: string | null;
  host: string | null;
  /** The six words of the server's fingerprint, to read out. */
  words: string[];
  /** This PC's check code, to read out. */
  check_code: string | null;
  confirmed_on: string | null;
  /** The address the user entered for when the server's IP changed. */
  manual_address: string | null;
  devices: PairedPc[];
  sharing: ShareState[];
  /** Non-blocking sharing notices (another PC switched a category on, and so on). */
  notices: { id: number; text: string }[];
  /** The server is recovering: sharing paused. */
  hold: boolean;
  /** The server's history did not verify: uploads paused. */
  mismatch: boolean;
  last_sync: string | null;
  /** About the last action (offline, revoked, history mismatch...). */
  message: string | null;
}

// The full personal backup (wireframe W9; hy_core::backup::Summary).

export interface BackupSummary {
  exported_at: string;
  events: number;
  days: number;
  from: string | null;
  to: string | null;
}

/** "Import": done (the app restarts), or still running after the wait. */
export type ImportOutcome = { state: 'done'; summary: BackupSummary } | { state: 'pending' };

/** An import's outcome, sent by the engine thread when it finishes ("hy://backup-import"). */
export interface ImportFinished {
  ok: boolean;
  summary: BackupSummary | null;
  error: string | null;
}

export interface BackupDone {
  path: string;
  summary: BackupSummary;
}

// The recovery-day plan window (wireframe W15; PlanPage in view.rs).

export interface RecoveryPlan {
  date: string;
  more_breaks: boolean;
  /** "HH:MM:SS" or null. */
  walk_at: string | null;
  walk_repeat: boolean;
  lunch_away: boolean;
  calm_pause: boolean;
  finish_at: string | null;
  reflection: boolean;
}

export interface PlanPage {
  plan: RecoveryPlan | null;
  draft: RecoveryPlan;
  dates: { date: string; workday: boolean }[];
  water_goal_ml: number | null;
  a11y: Accessibility;
}

// Settings > App & updates (slice V1-6; AppPage in update.rs).

export type UpdatePlan = 'upgrade' | 'rollback';
export type UpdateOutcomeKind = 'updated' | 'rolled_back' | 'blocked' | 'failed';

export interface UpdateCheck {
  current: string;
  version: string;
  plan: UpdatePlan;
  size_mb: number;
  /** Windows marked the file as downloaded: it may show its own warning. */
  marked_from_internet: boolean;
}

export interface UpdateOutcome {
  kind: UpdateOutcomeKind;
  from: string;
  to: string;
  /** Local time, "YYYY-MM-DD HH:MM". */
  at: string;
  detail: string;
}

export interface AppPage {
  version: string;
  install_dir: string | null;
  data_dir: string;
  /** Why update files cannot be installed here, if they cannot. */
  update_blocked: string | null;
  previous_version: string | null;
  in_progress: boolean;
  last: UpdateOutcome | null;
  autostart: { enabled: boolean; switched_off_in_windows: boolean } | null;
}

// ------------------------------------------------------------------ posture check (V2-1)

export type CameraHold = 'away' | 'paused' | 'off_hours' | 'presentation' | 'meeting';
export type Unavailable = 'denied' | 'busy' | 'no_camera' | 'model_failed' | 'stalled' | 'failed';
export type FrameIssue = 'absent' | 'low_confidence' | 'out_of_frame';
export type NoReading = 'starting' | 'stalled' | { frame: FrameIssue };

export type CameraStatus =
  | { kind: 'off' }
  | { kind: 'starting' }
  | { kind: 'reading' }
  | { kind: 'no_reading'; detail: NoReading }
  | { kind: 'held'; detail: CameraHold }
  | { kind: 'unavailable'; detail: Unavailable };

export interface CameraBadge {
  /** "Camera on", "Cam: no reading" or "Camera starting". */
  words: string;
  /** Usable frames now (the dot is filled). */
  reading: boolean;
}

export interface CameraMenu {
  on: boolean;
  /** "Posture check" while off, "Turn camera off" while on. */
  label: string;
  status: string;
}

export interface CameraStats {
  frames: number;
  valid_frames: number;
  stalls: number;
  worker_restarts: number;
  last_stop_ms: number | null;
  last_stop_forced: boolean;
  model_ms: number | null;
  inference_ms: number | null;
  fps: number;
}

export interface CameraPage {
  status: CameraStatus;
  words: string;
  on: boolean;
  consented: boolean;
  notice_version: number;
  /** The page should be capturing now. */
  capture: boolean;
  fps: number;
  stats: CameraStats;
  /** Calibration and the baseline (slice V2-2). */
  posture: PostureView;
  selftest: boolean;
  a11y: Accessibility;
}

/** One frame's derived result (never the image or the landmarks). */
export interface FrameReport {
  valid: boolean;
  issue: FrameIssue | null;
  inference_ms: number;
  visibility: number;
  /** A valid frame's posture numbers (slice V2-2; hy_core::posture::Features). */
  features?: Features;
}

/** A few numbers from one valid frame (hy_core::posture::Features). */
export interface Features {
  head: 'ears' | 'eyes';
  gap: number;
  width: number;
  tilt: number;
  roll: number;
  mid_x: number;
  mid_y: number;
  span: number;
  turn: number;
}

export type CalibrationIssue =
  | 'too_few_frames'
  | 'out_of_view'
  | 'too_far'
  | 'too_close'
  | 'poorly_visible'
  | 'moved'
  | 'interrupted';

export type PostureReason =
  | 'absent'
  | 'low_confidence'
  | 'out_of_frame'
  | 'stalled'
  | 'app_restart'
  | 'camera_restart'
  | 'camera_moved'
  | 'viewpoint_changed'
  | 'no_calibration';

export type PostureState =
  | { kind: 'not_calibrated' }
  | { kind: 'calibrating'; detail: { valid: number; needed: number } }
  | { kind: 'not_usable'; detail: CalibrationIssue }
  | { kind: 'ready' }
  | { kind: 'suspect'; detail: PostureReason }
  | { kind: 'recalibrate' };

export interface PostureView {
  state: PostureState;
  words: string;
  issue: string | null;
  calibrated_at: string | null;
  can_calibrate: boolean;
  stats: {
    calibrations: number;
    episodes: number;
    cues_shown: number;
    cues_suppressed: number;
    suspects: number;
    invalidations: number;
  };
  /** Today's coverage, derived from the local records (slice V2-3). */
  today: PostureCoverage | null;
}

/** A day's posture coverage (slice V2-3): camera on = no reading + not scored + usable. */
export interface PostureCoverage {
  camera_on_s: number;
  no_reading_s: number;
  unscored_s: number;
  usable_s: number;
  usable_share: number | null;
  episodes: number;
  cues_shown: number;
}

export type CapturePath = 'processor' | 'video';

export type PageReport =
  | { kind: 'started'; width: number; height: number; cores: number; model_ms: number; path: CapturePath }
  | { kind: 'error'; name: string; model: boolean }
  | { kind: 'stopped' }
  | { kind: 'probe'; blocked: boolean; detail: string };

export type PageCommand =
  | { kind: 'start' }
  | { kind: 'stop' }
  | { kind: 'restart_worker' }
  | { kind: 'fps'; fps: number }
  | { kind: 'freeze' }
  | { kind: 'probe' }
  | { kind: 'hang' };
