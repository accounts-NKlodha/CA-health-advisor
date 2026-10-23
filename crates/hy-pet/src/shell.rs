//! The pet shell's interface for the coaching engine, and its pure state model.
//!
//! The engine (slice V1-2) decides every reminder; the shell only shows what it is told
//! (architecture 4.2: the engine is "the only component that shows bubbles"). The engine
//! talks to the shell through [`PetShell`] and hears back through [`ShellEventSink`]:
//!
//! - [`PetShell::set_state`] sets the pet's state, badge and tag;
//! - [`PetShell::show_bubble`] shows one bubble beside the pet. At most one bubble is
//!   visible: a new one replaces the old (the old one's owner is told). While the pet is
//!   hidden, bubbles are suppressed and never replayed (owner-approved default, 21 Oct);
//! - [`PetShell::hide`] and [`PetShell::unhide`] hide the pet and its bubble for a reason,
//!   such as a presentation or full-screen app (detection is V1-2's; the tray stays and
//!   its "Show pet" overrides every reason);
//! - [`PetShell::set_menu_actions`] adds the engine's actions (water, movement, focus and
//!   so on) to the pet menu, the keyboard panel and, if asked, the tray.
//!
//! [`ShellModel`] holds the rules above without any windows, so they are unit tested;
//! the companion turns its [`Effect`]s into window calls.

use std::collections::BTreeSet;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::state::PetState;

/// Why the pet is hidden. It is visible only when no reason is set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HideReason {
    /// The user chose "Hide pet" in the tray or pet menu.
    User,
    /// A presentation or full-screen app is running (set by the engine's suppression).
    Presentation,
}

/// A status chip in a bubble (wireframe B2: movement, eye rest and water each show their
/// own status in words).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BubbleChip {
    /// For example "Water".
    pub label: String,
    /// For example "due" or "done (self-reported)".
    pub status: String,
    /// The chip of the activity on offer.
    #[serde(default)]
    pub highlighted: bool,
}

/// How an action button looks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionStyle {
    /// An ordinary button.
    #[default]
    Normal,
    /// The main action.
    Primary,
    /// A quiet text link (Not now, Skip).
    Link,
}

/// An action in a bubble.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BubbleAction {
    /// Returned in [`ShellEvent::BubbleAction`].
    pub id: String,
    /// Button text.
    pub label: String,
    /// Look.
    #[serde(default)]
    pub style: ActionStyle,
}

/// An illustration drawn by the pet page (the movement guide's figures).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BubbleFigure {
    /// Which drawing, for example "neck". The page draws only figures it knows.
    pub id: String,
    /// The drawing in words (its text equivalent).
    pub label: String,
}

/// Most numbered steps in one bubble.
pub const MAX_BUBBLE_STEPS: usize = 5;

/// A line of a list in a bubble with its own actions (the water panel's entries, each
/// with Edit and Remove, and its amount chooser).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BubbleRow {
    /// For example "09:50 - 250 ml".
    pub text: String,
    /// The row's actions; their ids are unique across the whole bubble.
    #[serde(default)]
    pub actions: Vec<BubbleAction>,
}

/// A progress bar with its value in words (the water panel's total against the user's
/// own goal). The page draws a native progress element, so nothing depends on colour.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BubbleProgress {
    /// The value, for example 750.
    pub value: u32,
    /// The full bar, for example 2000. At least 1; a value above it shows a full bar.
    pub max: u32,
    /// The value in words, for example "750 of 2000 ml".
    pub label: String,
}

/// Most rows in one bubble.
pub const MAX_BUBBLE_ROWS: usize = 24;
/// Most actions in one row.
pub const MAX_ROW_ACTIONS: usize = 4;

/// One bubble (the engine's reminder card, offer or notice).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bubble {
    /// Chosen by the engine; returned in events.
    pub id: String,
    /// Heading.
    pub title: String,
    /// Body lines.
    #[serde(default)]
    pub lines: Vec<String>,
    /// Status chips.
    #[serde(default)]
    pub chips: Vec<BubbleChip>,
    /// Actions.
    #[serde(default)]
    pub actions: Vec<BubbleAction>,
    /// Numbered steps, one action each (the movement guide).
    #[serde(default)]
    pub steps: Vec<String>,
    /// An illustration beside the steps.
    #[serde(default)]
    pub figure: Option<BubbleFigure>,
    /// A small note under the card, for example that its content is provisional.
    #[serde(default)]
    pub note: Option<String>,
    /// The card continues after an answer (the next movement replaces it), so the keyboard
    /// panel stays open instead of closing.
    #[serde(default)]
    pub continues: bool,
    /// A progress bar under the title.
    #[serde(default)]
    pub progress: Option<BubbleProgress>,
    /// A list of rows with their own actions, after the lines.
    #[serde(default)]
    pub rows: Vec<BubbleRow>,
}

/// Most actions in one bubble.
pub const MAX_BUBBLE_ACTIONS: usize = 8;
/// Most chips in one bubble.
pub const MAX_BUBBLE_CHIPS: usize = 4;

impl Bubble {
    /// A bubble with a title and one line, no actions.
    #[must_use]
    pub fn notice(id: &str, title: &str) -> Self {
        Self {
            id: id.to_owned(),
            title: title.to_owned(),
            ..Self::default()
        }
    }

    /// Whether the bubble offers the action `id`, in its action row or in one of its rows.
    #[must_use]
    pub fn has_action(&self, id: &str) -> bool {
        self.actions
            .iter()
            .chain(self.rows.iter().flat_map(|r| &r.actions))
            .any(|a| a.id == id)
    }

    /// Check the bubble's limits.
    ///
    /// # Errors
    /// A message naming the broken limit.
    pub fn validate(&self) -> Result<(), String> {
        let short = |s: &str, max: usize| !s.trim().is_empty() && s.chars().count() <= max;
        if !short(&self.id, 64) {
            return Err("bubble id must be 1 to 64 characters".into());
        }
        if !short(&self.title, 120) {
            return Err("bubble title must be 1 to 120 characters".into());
        }
        if self.lines.len() > 6 || self.lines.iter().any(|l| l.chars().count() > 300) {
            return Err("at most 6 lines of 300 characters".into());
        }
        if self.chips.len() > MAX_BUBBLE_CHIPS {
            return Err(format!("at most {MAX_BUBBLE_CHIPS} chips"));
        }
        if self.actions.len() > MAX_BUBBLE_ACTIONS {
            return Err(format!("at most {MAX_BUBBLE_ACTIONS} actions"));
        }
        if self.rows.len() > MAX_BUBBLE_ROWS {
            return Err(format!("at most {MAX_BUBBLE_ROWS} rows"));
        }
        if self
            .rows
            .iter()
            .any(|r| !short(&r.text, 120) || r.actions.len() > MAX_ROW_ACTIONS)
        {
            return Err(format!(
                "rows need text of 1 to 120 characters and at most {MAX_ROW_ACTIONS} actions"
            ));
        }
        let mut ids = BTreeSet::new();
        for a in self
            .actions
            .iter()
            .chain(self.rows.iter().flat_map(|r| &r.actions))
        {
            if !short(&a.id, 64) || !short(&a.label, 40) || !ids.insert(a.id.as_str()) {
                return Err("actions need unique ids and labels of 1 to 40 characters".into());
            }
        }
        if let Some(p) = &self.progress
            && (p.max == 0 || !short(&p.label, 80))
        {
            return Err(
                "a progress bar needs a maximum of at least 1 and a label of 1 to 80 characters"
                    .into(),
            );
        }
        if self.steps.len() > MAX_BUBBLE_STEPS || !self.steps.iter().all(|s| short(s, 160)) {
            return Err(format!(
                "at most {MAX_BUBBLE_STEPS} steps of 1 to 160 characters"
            ));
        }
        if let Some(f) = &self.figure {
            let id_ok = short(&f.id, 32)
                && f.id
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
            if !id_ok || !short(&f.label, 200) {
                return Err(
                    "a figure needs a short lowercase id and a label of 1 to 200 characters".into(),
                );
            }
        }
        if self.note.as_deref().is_some_and(|n| !short(n, 120)) {
            return Err("the note must be 1 to 120 characters".into());
        }
        Ok(())
    }
}

/// An engine action in the pet menu, keyboard panel and optionally the tray.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MenuAction {
    /// Returned in [`ShellEvent::MenuAction`].
    pub id: String,
    /// Button text.
    pub label: String,
    /// Also list it in the tray menu (for example "Pause...").
    #[serde(default)]
    pub in_tray: bool,
    /// It opens a card (for example the water confirmation), so the keyboard panel stays
    /// open and shows that card instead of closing.
    #[serde(default)]
    pub continues: bool,
}

/// Why a bubble closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseCause {
    /// The user closed it (the close button, or a click on the pet). This is not a
    /// dismissal or a completion; the engine decides what it means.
    User,
    /// Another bubble replaced it.
    Replaced,
    /// The pet was hidden (for example a presentation started).
    Hidden,
}

/// What the shell tells the engine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ShellEvent {
    /// The user chose an action in a bubble.
    BubbleAction {
        /// The bubble.
        bubble_id: String,
        /// The action.
        action_id: String,
    },
    /// A bubble closed without an action.
    BubbleClosed {
        /// The bubble.
        bubble_id: String,
        /// Why.
        cause: CloseCause,
    },
    /// The user chose an engine action in the pet menu, panel or tray.
    MenuAction {
        /// The action.
        action_id: String,
    },
    /// The pet was clicked with nothing open. If the engine does not handle it (for
    /// example to reopen a waiting reminder), the shell opens the pet menu.
    PetClicked {
        /// The state at the time.
        state: PetState,
    },
    /// The user chose the tray's "Show pet" while the pet was hidden for a presentation:
    /// the engine keeps the pet shown until the presentation ends.
    PetShownByUser,
}

/// Whether the engine handled an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handled {
    /// Handled; the shell does nothing more.
    Yes,
    /// Not handled; the shell applies its default (the pet menu for a click).
    No,
}

/// The engine's side of the conversation.
pub trait ShellEventSink: Send + Sync {
    /// Called on the UI thread; must not block.
    fn on_event(&self, event: &ShellEvent) -> Handled;
}

/// The result of [`PetShell::show_bubble`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BubbleOutcome {
    /// Shown.
    Shown,
    /// Not shown because the pet is hidden; it is not queued or replayed.
    Suppressed,
    /// Not shown because the bubble breaks a limit.
    Invalid,
}

/// The calls the coaching engine makes on the pet shell. The companion implements it;
/// every call can be made from any thread.
pub trait PetShell: Send + Sync {
    /// Set the state; `tag` replaces the default text tag (`Some("")` removes it).
    fn set_state(&self, state: PetState, tag: Option<String>);
    /// Show one bubble, replacing any open one.
    fn show_bubble(&self, bubble: Bubble) -> BubbleOutcome;
    /// Close the bubble with this id if it is open (no event is sent back).
    fn close_bubble(&self, bubble_id: &str);
    /// Hide the pet and its bubble for `reason`.
    fn hide(&self, reason: HideReason);
    /// Remove `reason`; the pet shows again when no reason is left. Nothing is replayed.
    fn unhide(&self, reason: HideReason);
    /// Whether the pet is visible now.
    fn is_visible(&self) -> bool;
    /// Replace the engine's menu actions.
    fn set_menu_actions(&self, actions: Vec<MenuAction>);
    /// Register the engine's event sink (replacing any earlier one).
    fn set_event_sink(&self, sink: Arc<dyn ShellEventSink>);
}

/// What is open beside the pet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Open {
    /// Nothing.
    None,
    /// The engine's bubble.
    Bubble {
        /// The bubble.
        bubble: Bubble,
    },
    /// The pet menu (wireframe B1).
    Menu,
    /// A short shell notice (for example "Your pet is back above the clock").
    Notice {
        /// The notice.
        bubble: Bubble,
    },
}

/// A window operation for the companion to carry out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Show the pet window without activating it.
    ShowWindow,
    /// Hide the pet window.
    HideWindow,
    /// Send the pet page a fresh view and lay the window out again.
    Render,
    /// Tell the engine.
    Emit(ShellEvent),
    /// Rebuild the tray menu and tooltip.
    UpdateTray,
}

/// The tray's labels.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TrayView {
    /// "Hide pet" or "Show pet".
    pub toggle_label: &'static str,
    /// Tooltip.
    pub tooltip: String,
    /// Engine actions shown in the tray.
    pub actions: Vec<MenuAction>,
}

/// The shell's rules, without windows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellModel {
    hidden: BTreeSet<HideReason>,
    state: PetState,
    tag: Option<String>,
    open: Open,
    menu_actions: Vec<MenuAction>,
}

impl Default for ShellModel {
    fn default() -> Self {
        Self::new()
    }
}

impl ShellModel {
    /// Visible, idle, nothing open.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            hidden: BTreeSet::new(),
            state: PetState::Idle,
            tag: None,
            open: Open::None,
            menu_actions: Vec::new(),
        }
    }

    /// Whether the pet is visible.
    #[must_use]
    pub fn is_visible(&self) -> bool {
        self.hidden.is_empty()
    }

    /// The hide reasons in force.
    #[must_use]
    pub const fn hidden_reasons(&self) -> &BTreeSet<HideReason> {
        &self.hidden
    }

    /// The state and tag override.
    #[must_use]
    pub fn state(&self) -> (PetState, Option<&str>) {
        (self.state, self.tag.as_deref())
    }

    /// What is open beside the pet.
    #[must_use]
    pub const fn open(&self) -> &Open {
        &self.open
    }

    /// The engine's menu actions.
    #[must_use]
    pub fn menu_actions(&self) -> &[MenuAction] {
        &self.menu_actions
    }

    /// Set the state and tag.
    pub fn set_state(&mut self, state: PetState, tag: Option<String>) -> Vec<Effect> {
        self.state = state;
        self.tag = tag;
        vec![Effect::Render]
    }

    /// Show the engine's bubble, replacing whatever is open.
    pub fn show_bubble(&mut self, bubble: Bubble) -> (BubbleOutcome, Vec<Effect>) {
        if bubble.validate().is_err() {
            return (BubbleOutcome::Invalid, Vec::new());
        }
        if !self.is_visible() {
            return (BubbleOutcome::Suppressed, Vec::new());
        }
        let mut fx = Vec::new();
        if let Open::Bubble { bubble: old } = &self.open {
            fx.push(Effect::Emit(ShellEvent::BubbleClosed {
                bubble_id: old.id.clone(),
                cause: CloseCause::Replaced,
            }));
        }
        self.open = Open::Bubble { bubble };
        fx.push(Effect::Render);
        (BubbleOutcome::Shown, fx)
    }

    /// The engine closes its bubble (no event goes back to it).
    pub fn close_bubble(&mut self, bubble_id: &str) -> Vec<Effect> {
        match &self.open {
            Open::Bubble { bubble } if bubble.id == bubble_id => {
                self.open = Open::None;
                vec![Effect::Render]
            }
            _ => Vec::new(),
        }
    }

    /// The user closed what is open (close button, Escape in the panel, or a click on
    /// the pet while something is open).
    pub fn user_close(&mut self) -> Vec<Effect> {
        match std::mem::replace(&mut self.open, Open::None) {
            Open::None => Vec::new(),
            Open::Bubble { bubble } => vec![
                Effect::Emit(ShellEvent::BubbleClosed {
                    bubble_id: bubble.id,
                    cause: CloseCause::User,
                }),
                Effect::Render,
            ],
            Open::Menu | Open::Notice { .. } => vec![Effect::Render],
        }
    }

    /// The user chose an action in the open bubble. A stale click (the bubble was
    /// already replaced) is ignored. The bubble stays until the engine closes or
    /// replaces it.
    pub fn bubble_action(&self, bubble_id: &str, action_id: &str) -> Vec<Effect> {
        match &self.open {
            Open::Bubble { bubble } if bubble.id == bubble_id && bubble.has_action(action_id) => {
                vec![Effect::Emit(ShellEvent::BubbleAction {
                    bubble_id: bubble_id.to_owned(),
                    action_id: action_id.to_owned(),
                })]
            }
            _ => Vec::new(),
        }
    }

    /// The user chose an engine menu action. Unknown ids are ignored. The menu closes.
    pub fn menu_action(&mut self, action_id: &str) -> Vec<Effect> {
        if !self.menu_actions.iter().any(|a| a.id == action_id) {
            return Vec::new();
        }
        let mut fx = Vec::new();
        if self.open == Open::Menu {
            self.open = Open::None;
            fx.push(Effect::Render);
        }
        fx.push(Effect::Emit(ShellEvent::MenuAction {
            action_id: action_id.to_owned(),
        }));
        fx
    }

    /// A click on the pet: closes what is open, otherwise asks the engine (the
    /// companion then calls [`Self::open_menu`] if the engine did not handle it).
    pub fn pet_clicked(&mut self) -> Vec<Effect> {
        if self.open == Open::None {
            vec![Effect::Emit(ShellEvent::PetClicked { state: self.state })]
        } else {
            self.user_close()
        }
    }

    /// Open the pet menu, unless the engine's bubble is open or the pet is hidden.
    pub fn open_menu(&mut self) -> Vec<Effect> {
        if !self.is_visible() || matches!(self.open, Open::Bubble { .. }) {
            return Vec::new();
        }
        self.open = Open::Menu;
        vec![Effect::Render]
    }

    /// Close the pet menu if it is open (the keyboard panel opened instead).
    pub fn close_menu(&mut self) -> Vec<Effect> {
        if self.open == Open::Menu {
            self.open = Open::None;
            vec![Effect::Render]
        } else {
            Vec::new()
        }
    }

    /// Show a shell notice. It never replaces the engine's bubble, and is not shown
    /// while the pet is hidden.
    pub fn show_notice(&mut self, bubble: Bubble) -> Vec<Effect> {
        if !self.is_visible() || matches!(self.open, Open::Bubble { .. }) {
            return Vec::new();
        }
        self.open = Open::Notice { bubble };
        vec![Effect::Render]
    }

    /// The notice with this id timed out.
    pub fn notice_expired(&mut self, id: &str) -> Vec<Effect> {
        match &self.open {
            Open::Notice { bubble } if bubble.id == id => {
                self.open = Open::None;
                vec![Effect::Render]
            }
            _ => Vec::new(),
        }
    }

    /// Add a hide reason. When the pet goes from visible to hidden its window hides and
    /// whatever was open closes (the engine is told its bubble closed).
    pub fn hide(&mut self, reason: HideReason) -> Vec<Effect> {
        let was_visible = self.is_visible();
        if !self.hidden.insert(reason) {
            return Vec::new();
        }
        if !was_visible {
            return vec![Effect::UpdateTray];
        }
        let mut fx = Vec::new();
        if let Open::Bubble { bubble } = &self.open {
            fx.push(Effect::Emit(ShellEvent::BubbleClosed {
                bubble_id: bubble.id.clone(),
                cause: CloseCause::Hidden,
            }));
        }
        self.open = Open::None;
        fx.extend([Effect::HideWindow, Effect::Render, Effect::UpdateTray]);
        fx
    }

    /// Remove a hide reason. The pet shows when none is left; nothing is replayed.
    pub fn unhide(&mut self, reason: HideReason) -> Vec<Effect> {
        if !self.hidden.remove(&reason) {
            return Vec::new();
        }
        if self.is_visible() {
            vec![Effect::ShowWindow, Effect::Render, Effect::UpdateTray]
        } else {
            vec![Effect::UpdateTray]
        }
    }

    /// The tray's "Show pet": the user's explicit choice overrides every reason.
    pub fn user_show(&mut self) -> Vec<Effect> {
        if self.hidden.is_empty() {
            return Vec::new();
        }
        let presentation = self.hidden.contains(&HideReason::Presentation);
        self.hidden.clear();
        let mut fx = vec![Effect::ShowWindow, Effect::Render, Effect::UpdateTray];
        if presentation {
            // The engine hid the pet; tell it the user overrode that.
            fx.push(Effect::Emit(ShellEvent::PetShownByUser));
        }
        fx
    }

    /// "Bring pet above clock" clears the user's own hiding (it works for a hidden or
    /// lost pet) but does not override a presentation (prototype: the pet stays hidden
    /// and comes back when the presentation ends). The companion moves the window
    /// either way.
    pub fn bring_back(&mut self) -> Vec<Effect> {
        self.unhide(HideReason::User)
    }

    /// Replace the engine's menu actions.
    pub fn set_menu_actions(&mut self, actions: Vec<MenuAction>) -> Vec<Effect> {
        self.menu_actions = actions;
        vec![Effect::Render, Effect::UpdateTray]
    }

    /// The tray's labels.
    #[must_use]
    pub fn tray(&self) -> TrayView {
        let tooltip = if self.hidden.contains(&HideReason::Presentation) {
            "HealthierYou - pet hidden: presentation".to_owned()
        } else if self.hidden.contains(&HideReason::User) {
            "HealthierYou - pet hidden".to_owned()
        } else {
            "HealthierYou".to_owned()
        };
        TrayView {
            toggle_label: if self.is_visible() {
                "Hide pet"
            } else {
                "Show pet"
            },
            tooltip,
            actions: self
                .menu_actions
                .iter()
                .filter(|a| a.in_tray)
                .cloned()
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests;
