//! What the pages are sent. Every view is a complete snapshot, so a page can always
//! redraw from the latest one.

use chrono::NaiveDate;
use hy_core::coach::{CoachConfig, RecoveryPlan, Servings};
use hy_core::prefs::Accessibility;
use hy_core::store::PetProfile;
use hy_core::time::WorkSchedule;
use hy_pet::hotkey::HOTKEY_CHOICES;
use hy_pet::messages::{MessageId, message};
use hy_pet::pack::{ArtStatus, ColourVariant, Hitbox, Library, Pack, StatePose};
use hy_pet::placement::{SCALE_MAX, SCALE_MIN, SCALE_STEP, SIZE_PRESETS, Side, SizePreset};
use hy_pet::shell::{Bubble, MenuAction, Open};
use hy_pet::state::{BaseState, StateLook};
use serde::Serialize;

/// A review page's data (wireframes W13 and W14) and whether it may animate: the
/// graphics animate once on opening unless motion is off (here or in Windows).
#[derive(Debug, Clone, Serialize)]
pub struct ReviewPage<T> {
    /// The review.
    pub review: T,
    /// Animation allowed: the pet's Motion switch is on and reduced motion is off
    /// (Settings, Accessibility).
    pub motion: bool,
    /// The display preferences.
    pub a11y: Accessibility,
}

/// One day of the movement programme (wireframe W6).
#[derive(Debug, Clone, Serialize)]
pub struct ProgrammeDay {
    /// "Mon" to "Sat".
    pub day: &'static str,
    /// The three movements' names.
    pub movements: Vec<&'static str>,
}

/// One movement of the library (wireframe W6).
#[derive(Debug, Clone, Serialize)]
pub struct LibraryMovement {
    /// Its name.
    pub name: &'static str,
    /// Seated, standing, or both.
    pub place: &'static str,
    /// Repetitions or seconds, and about how long.
    pub dose: &'static str,
    /// The seated replacement's name, if it has one.
    pub seated: Option<&'static str>,
}

/// The settings window's coaching, water, workweek, movement and accessibility tabs
/// (wireframes W4 to W7 and W12). The Pet tab keeps its own view ([`SettingsView`]).
#[derive(Debug, Clone, Serialize)]
pub struct SettingsPage {
    /// Counts the settings changes made since the app started; a page keeps the newest
    /// answer it has seen when answers to quick successive changes arrive out of order.
    pub revision: u64,
    /// Reminders, movement version, Pomodoro and the review time.
    pub coach: CoachConfig,
    /// Workdays and hours.
    pub schedule: WorkSchedule,
    /// The IANA time zone.
    pub zone: &'static str,
    /// Every zone offered.
    pub zones: Vec<&'static str>,
    /// Today's water goal, if one is set.
    pub water_goal_ml: Option<u32>,
    /// Accepted goal range, ml.
    pub goal_min_ml: u32,
    /// Accepted goal range, ml.
    pub goal_max_ml: u32,
    /// The serving buttons.
    pub servings: Servings,
    /// Accepted serving range, ml.
    pub serving_min_ml: u32,
    /// Accepted serving range, ml.
    pub serving_max_ml: u32,
    /// At most this many serving buttons.
    pub max_servings: usize,
    /// Display preferences.
    pub a11y: Accessibility,
    /// The Monday to Saturday programme.
    pub programme: Vec<ProgrammeDay>,
    /// The movement library.
    pub library: Vec<LibraryMovement>,
    /// Shown on the movement tab while the content is provisional.
    pub provisional: Option<&'static str>,
    /// The panel hotkey, for the keyboard table.
    pub hotkey: Option<String>,
    /// The sharing categories and their state (wireframe W8).
    pub sharing: Vec<SharingRow>,
    /// Where exports are written (wireframe W9).
    pub backup_dir: Option<String>,
}

/// One optional sharing category (wireframe W8).
#[derive(Debug, Clone, Serialize)]
pub struct SharingRow {
    /// The stored name.
    pub category: &'static str,
    /// Its name in words.
    pub label: &'static str,
    /// On (only after the office server accepted it).
    pub enabled: bool,
}

/// One date the recovery day can move to (wireframe W15).
#[derive(Debug, Clone, Copy, Serialize)]
pub struct PlanDate {
    /// The date.
    pub date: NaiveDate,
    /// It is a workday.
    pub workday: bool,
}

/// The recovery-day plan window (wireframe W15).
#[derive(Debug, Clone, Serialize)]
pub struct PlanPage {
    /// The saved plan, if one is pending (its date is after today).
    pub plan: Option<RecoveryPlan>,
    /// A starting plan for the next workday, used when none is saved.
    pub draft: RecoveryPlan,
    /// The dates offered (tomorrow onward).
    pub dates: Vec<PlanDate>,
    /// The user's own water goal, which stays the same.
    pub water_goal_ml: Option<u32>,
    /// Display preferences.
    pub a11y: Accessibility,
}

/// The pet page's view.
#[derive(Debug, Clone, Serialize)]
pub struct PetView {
    /// Pet name.
    pub name: String,
    /// Pack id and version; the page re-inserts the art only when these change.
    pub pack_key: String,
    /// The checked art.
    pub art_svg: String,
    /// The chosen colour variant.
    pub colours: ColourVariant,
    /// The clickable part of the art, view-box units.
    pub hitbox: Hitbox,
    /// State, badge, tag.
    pub look: StateLook,
    /// The pose for the state's base, from the pack.
    pub pose: StatePose,
    /// Accessible name ("Shelly the pet, Idle").
    pub accessible_name: String,
    /// Animation allowed.
    pub motion: bool,
    /// Display preferences (text size, reduced motion, contrast).
    pub a11y: Accessibility,
    /// The pet window's side in CSS pixels (logical).
    pub pet_px: f64,
    /// What is open beside the pet.
    pub open: OpenView,
    /// Where things go in the window, CSS pixels.
    pub layout: LayoutView,
}

/// The open bubble or menu.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OpenView {
    /// Nothing.
    None,
    /// The engine's bubble.
    Bubble {
        /// Changes every time something new opens; measurements carry it back.
        token: u64,
        /// The bubble.
        bubble: Bubble,
    },
    /// A shell notice.
    Notice {
        /// See above.
        token: u64,
        /// The notice.
        bubble: Bubble,
    },
    /// The pet menu.
    Menu {
        /// See above.
        token: u64,
        /// The menu.
        menu: MenuView,
    },
}

/// The pet menu and the keyboard panel (wireframe B1 and K1).
#[derive(Debug, Clone, Serialize)]
pub struct MenuView {
    /// "Hi, I'm Shelly."
    pub greeting: String,
    /// The engine's actions (none until the engine registers some).
    pub actions: Vec<MenuAction>,
    /// The current size.
    pub size_pct: u16,
    /// S, M, L, XL.
    pub presets: [SizePreset; 4],
    /// Footer hint.
    pub hint: String,
    /// The panel hotkey label, if one is registered.
    pub hotkey: Option<String>,
}

/// The keyboard panel's view.
#[derive(Debug, Clone, Serialize)]
pub struct PanelView {
    /// The engine's bubble open beside the pet, so it can be answered from the keyboard
    /// (the same bubble, not a second one).
    pub bubble: Option<Bubble>,
    /// The same content as the pet menu.
    pub menu: MenuView,
    /// "Tab moves between actions. Esc closes and returns you to your work."
    pub panel_hint: String,
    /// The pet's accessible name and state.
    pub accessible_name: String,
    /// Display preferences (text size, reduced motion, contrast).
    pub a11y: Accessibility,
}

/// Placement inside the pet window, CSS pixels.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct LayoutView {
    /// The pet's left offset in the window.
    pub pet_x: f64,
    /// The pet's top offset.
    pub pet_y: f64,
    /// The bubble's left offset, once measured.
    pub bubble_x: f64,
    /// The bubble's top offset, once measured.
    pub bubble_y: f64,
    /// The token the layout was measured for (0 = not measured).
    pub measured_token: u64,
    /// The side the bubble opened on.
    pub side: Side,
    /// The largest bubble the page may draw.
    pub bubble_max_w: f64,
    /// See above.
    pub bubble_max_h: f64,
}

/// Build the menu view.
pub fn menu_view(profile: &PetProfile, actions: &[MenuAction], hotkey: Option<String>) -> MenuView {
    MenuView {
        greeting: message(MessageId::Greeting, profile.tone, &profile.display_name),
        actions: actions.to_vec(),
        size_pct: profile.scale_pct,
        presets: SIZE_PRESETS,
        hint: message(MessageId::MenuHint, profile.tone, &profile.display_name),
        hotkey,
    }
}

/// The pose for a base state.
pub fn pose_for(pack: &Pack, base: BaseState) -> StatePose {
    let s = &pack.manifest.states;
    match base {
        BaseState::Idle => s.idle,
        BaseState::Cue => s.cue,
        BaseState::Exercise => s.exercise,
        BaseState::Waiting => s.waiting,
        BaseState::Celebration => s.celebration,
        BaseState::Snoozed => s.snoozed,
        BaseState::Focus => s.focus,
        BaseState::Disconnected => s.disconnected,
    }
}

/// The open part of the pet view.
pub fn open_view(open: &Open, token: u64, menu: impl FnOnce() -> MenuView) -> OpenView {
    match open {
        Open::None => OpenView::None,
        Open::Bubble { bubble } => OpenView::Bubble {
            token,
            bubble: bubble.clone(),
        },
        Open::Notice { bubble } => OpenView::Notice {
            token,
            bubble: bubble.clone(),
        },
        Open::Menu => OpenView::Menu {
            token,
            menu: menu(),
        },
    }
}

/// One card in the Settings pet library.
#[derive(Debug, Clone, Serialize)]
pub struct PackCard {
    /// Pack id.
    pub id: String,
    /// Library name.
    pub name: String,
    /// Default display name.
    pub default_name: String,
    /// Drawn or pending review.
    pub art_status: ArtStatus,
    /// Whether it can be chosen.
    pub selectable: bool,
    /// The art.
    pub art_svg: String,
    /// Colour variants.
    pub palette: Vec<ColourVariant>,
    /// Provenance and review note, shown as a tooltip.
    pub review: String,
}

/// The Settings page's view.
#[derive(Debug, Clone, Serialize)]
pub struct SettingsView {
    /// Chosen pet id.
    pub pet_id: String,
    /// Name.
    pub name: String,
    /// Size, percent.
    pub size_pct: u16,
    /// Colour variant.
    pub colour_variant: u8,
    /// "left", "right" or "free".
    pub edge: hy_core::schema::PetEdge,
    /// Motion on.
    pub motion: bool,
    /// Tone.
    pub tone: hy_core::schema::PetTone,
    /// The library, in grid order.
    pub packs: Vec<PackCard>,
    /// S, M, L, XL.
    pub presets: [SizePreset; 4],
    /// Slider limits.
    pub size_min: u16,
    /// See above.
    pub size_max: u16,
    /// See above.
    pub size_step: u16,
    /// The panel hotkey, or none.
    pub hotkey: Option<String>,
    /// Whether Windows accepted the hotkey.
    pub hotkey_registered: bool,
    /// Hotkeys offered.
    pub hotkey_choices: [&'static str; 4],
}

/// Build the Settings view.
pub fn settings_view(
    profile: &PetProfile,
    library: &Library,
    hotkey: Option<String>,
    hotkey_registered: bool,
) -> SettingsView {
    SettingsView {
        pet_id: profile.pet_id.clone(),
        name: profile.display_name.clone(),
        size_pct: profile.scale_pct,
        colour_variant: profile.colour_variant,
        edge: profile.edge,
        motion: profile.motion,
        tone: profile.tone,
        packs: library
            .packs()
            .iter()
            .map(|p| PackCard {
                id: p.id().to_owned(),
                name: p.manifest.name.clone(),
                default_name: p.manifest.default_name.clone(),
                art_status: p.manifest.art_status,
                selectable: p.selectable(),
                art_svg: p.art_svg.clone(),
                palette: p.manifest.palette.clone(),
                review: p.manifest.provenance.review.clone(),
            })
            .collect(),
        presets: SIZE_PRESETS,
        size_min: SCALE_MIN,
        size_max: SCALE_MAX,
        size_step: SCALE_STEP,
        hotkey,
        hotkey_registered,
        hotkey_choices: HOTKEY_CHOICES,
    }
}
