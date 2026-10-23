// Typed wrappers over Tauri IPC. Each window may call only the commands its capability
// file grants (apps/companion/src-tauri/capabilities). Argument names are camelCase, as
// Tauri maps them to the Rust snake_case parameters.
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type {
  Accessibility,
  BackupDone,
  CoachChange,
  ImportFinished,
  ImportOutcome,
  LinkView,
  MonthReview,
  PanelView,
  PetEdit,
  PetView,
  PhoneLabel,
  PlanPage,
  RecoveryPlan,
  ReviewPage,
  SettingsPage,
  SettingsView,
  SuggestionAnswer,
  SuggestionKind,
  WaterChange,
  WeekReview,
  WorkweekChange,
} from './types';

interface Commands {
  pet_bootstrap: { args: undefined; result: PetView };
  pet_ready: { args: undefined; result: void };
  pet_click: { args: undefined; result: void };
  bubble_measured: { args: { token: number; width: number; height: number }; result: void };
  bubble_close: { args: undefined; result: void };
  bubble_action: { args: { bubbleId: string; actionId: string }; result: void };
  menu_action: { args: { actionId: string }; result: void };
  drag_start: { args: undefined; result: void };
  drag_move: { args: undefined; result: void };
  drag_end: { args: { moved: boolean }; result: void };
  pet_edit: { args: { edit: PetEdit }; result: SettingsView };
  bring_pet: { args: undefined; result: void };
  hide_pet: { args: undefined; result: void };
  open_settings: { args: undefined; result: void };
  panel_bootstrap: { args: undefined; result: PanelView };
  panel_close: { args: { restore: boolean }; result: void };
  settings_bootstrap: { args: undefined; result: SettingsView };
  set_hotkey: { args: { hotkey: string | null }; result: SettingsView };
  open_review: { args: undefined; result: void };
  review_week: { args: { day: string | null }; result: ReviewPage<WeekReview> };
  review_month: { args: { month: string | null }; result: ReviewPage<MonthReview> };
  review_target: { args: undefined; result: string | null };
  review_phone_label: {
    args: { week: string | null; entry: string; label: PhoneLabel };
    result: ReviewPage<WeekReview>;
  };
  review_suggestion: {
    args: { week: string; kind: SuggestionKind; answer: SuggestionAnswer };
    result: ReviewPage<WeekReview>;
  };
  settings_page: { args: undefined; result: SettingsPage };
  save_coach_settings: { args: { change: CoachChange }; result: SettingsPage };
  save_water_settings: { args: { change: WaterChange }; result: SettingsPage };
  save_workweek: { args: { change: WorkweekChange }; result: SettingsPage };
  save_accessibility: { args: { a11y: Accessibility }; result: SettingsPage };
  plan_page: { args: undefined; result: PlanPage };
  plan_save: { args: { plan: RecoveryPlan }; result: PlanPage };
  plan_cancel: { args: undefined; result: PlanPage };
  open_setup: { args: undefined; result: void };
  setup_page: { args: undefined; result: SettingsPage };
  setup_sample: { args: undefined; result: void };
  setup_finish: { args: undefined; result: void };
  backup_export: { args: { password: string }; result: BackupDone };
  backup_import: { args: { password: string }; result: ImportOutcome };
  server_status: { args: undefined; result: LinkView };
  server_pair: { args: { slip: string }; result: LinkView };
  server_refresh: { args: undefined; result: LinkView };
  server_unpair: { args: undefined; result: LinkView };
  server_notice_seen: { args: { deviceRef: string }; result: LinkView };
  server_set_address: { args: { address: string }; result: LinkView };
}

type Name = keyof Commands;

/** Call a shell command. */
export function call<N extends Name>(
  name: N,
  ...args: Commands[N]['args'] extends undefined ? [] : [Commands[N]['args']]
): Promise<Commands[N]['result']> {
  return invoke<Commands[N]['result']>(name, args[0] as Record<string, unknown> | undefined);
}

/** Hand the shell the bytes of a backup file the user chose (sent as the raw body). */
export function stageBackup(bytes: Uint8Array): Promise<number> {
  return invoke<number>('backup_stage', bytes);
}

interface Events {
  'pet://view': PetView;
  'panel://view': PanelView;
  'settings://changed': SettingsView;
  /** A settings change was saved (from any window); the payload is its revision. */
  'hy://settings': number;
  /** "Open" on the weekly review offer named a week; fetch it with review_target. */
  'hy://review-target': string;
  /** An import finished (also one the page stopped waiting for). */
  'hy://backup-import': ImportFinished;
}

/** Listen for a view the shell pushes to this window. */
export function on<E extends keyof Events>(
  event: E,
  handler: (payload: Events[E]) => void,
): Promise<UnlistenFn> {
  return listen<Events[E]>(event, (e) => handler(e.payload));
}

/** Report an IPC failure without throwing into the page. */
export function report(err: unknown): void {
  console.error('HealthierYou:', err);
}
