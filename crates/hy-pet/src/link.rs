//! The link between the coaching engine ([`hy_core::coach::Coach`], slice V1-2) and the pet
//! shell ([`PetShell`], slice V1-1).
//!
//! Inputs go in through [`CoachLink::input`] (the companion's 5-second idle poll, lock,
//! unlock, suspend, resume and presentation changes) and [`CoachLink::command`] (what
//! [`route`] made of a shell event). After each step the link draws the engine's
//! [`CoachView`] on the shell, changing only what differs:
//!
//! - the pet's state and tag ([`present::pet_look`]), with a short celebration after a
//!   done answer;
//! - at most one bubble, the engine's ([`present::bubble`]). Each show gets a new bubble
//!   id ([`present::BubbleKind::id`]); the link closes it when the engine's view no longer
//!   has it, and never shows anything the engine did not ask for, so a bubble the engine
//!   withdrew (a hold, a presentation) is never replayed;
//! - `hide(HideReason::Presentation)` while the engine's view is not visible, and
//!   `unhide` when it is again. The engine decides (presentation detection feeds it, and
//!   the user's setting may keep the pet shown); the shell only follows;
//! - the engine's pet menu, panel and tray entries ([`present::menu_actions`]).
//!
//! Answers come back from the shell through [`LinkSink`] (the shell's
//! [`ShellEventSink`]) on the UI thread: it routes the event against the [`RouteState`]
//! the link wrote just before changing the shell, decides at once whether a click on the
//! pet is handled, and forwards the resulting [`Command`] without blocking. An answer on
//! a bubble carries that bubble's id, and the link applies it on the engine thread only
//! while that bubble is still the one on show ([`Command::Answer`]): two quick clicks on
//! one card, or a click queued before the card changed, count at most once.
//!
//! The user's own entries (water, phone interruptions; wireframes B3 and B8) are written
//! by the link through [`Store::append_event`], which keeps the correction rules (a
//! change supersedes the entry, Undo and Remove retract it), and the engine is then told
//! what was recorded ([`WaterInput`], [`PhoneInput`]) with the stored entry's id, so it
//! shows the confirmation, the panel or the note. A change asked on a card is applied
//! only while that card is on show ([`Command::Change`]), and its amounts and ids come
//! from the engine's view at that moment.
//!
//! If the shell refuses a bubble because the user hid the pet, the reminder is collapsed
//! to its waiting badge ([`Input::CloseBubble`]) instead of popping up later.

pub mod present;
pub mod route;
pub mod signals;

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use hy_core::coach::Coach;
use hy_core::coach::{
    Bubble as CoachBubble, CoachView, Input, InputOutcome, Notice, PhoneInput, StepOutput,
    WaterInput,
};
use hy_core::error::Result;
use hy_core::schema::{EventPayload, NewEvent, PhoneLabel};
use hy_core::store::Store;
use uuid::Uuid;

pub use route::{Command, EntryChange, RouteState, Routed, review_week, route};

use self::present::BubbleKind;
use crate::shell::{
    Bubble, BubbleOutcome, Handled, HideReason, MenuAction, PetShell, ShellEvent, ShellEventSink,
};
use crate::state::PetState;

/// How long the celebration pose lasts after a done answer (wireframe section 4: two
/// small hops, then Idle). It ends on the first step after this.
pub const CELEBRATE_FOR: Duration = Duration::from_secs(3);

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The shared routing snapshot. Cheap to clone.
#[derive(Debug, Clone, Default)]
pub struct Router(Arc<Mutex<RouteState>>);

impl Router {
    /// Route one shell event against the latest snapshot.
    #[must_use]
    pub fn route(&self, event: &ShellEvent) -> Routed {
        route(&lock(&self.0), event)
    }

    /// The latest snapshot.
    #[must_use]
    pub fn state(&self) -> RouteState {
        lock(&self.0).clone()
    }

    fn set(&self, state: RouteState) {
        *lock(&self.0) = state;
    }
}

/// The shell's event sink: routes on the UI thread and hands the command on.
pub struct LinkSink {
    router: Router,
    forward: Box<dyn Fn(Command) + Send + Sync>,
}

impl std::fmt::Debug for LinkSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LinkSink")
            .field("router", &self.router)
            .finish_non_exhaustive()
    }
}

impl LinkSink {
    /// A sink that routes with `router` and passes each command to `forward`, which must
    /// not block (the companion sends it to the engine's thread).
    #[must_use]
    pub fn new(router: Router, forward: impl Fn(Command) + Send + Sync + 'static) -> Self {
        Self {
            router,
            forward: Box::new(forward),
        }
    }
}

impl ShellEventSink for LinkSink {
    fn on_event(&self, event: &ShellEvent) -> Handled {
        let routed = self.router.route(event);
        if let Some(c) = routed.command {
            (self.forward)(c);
        }
        routed.handled
    }
}

/// What the link last told the shell.
#[derive(Debug, Default)]
struct Shown {
    hidden: bool,
    look: Option<(PetState, Option<String>)>,
    menu: Option<Vec<MenuAction>>,
    bubble: Option<(BubbleKind, Bubble)>,
}

/// The engine bound to the shell.
pub struct CoachLink {
    coach: Coach,
    shell: Arc<dyn PetShell>,
    router: Router,
    shown: Shown,
    rev: u64,
    celebrate_until: Option<Duration>,
}

impl std::fmt::Debug for CoachLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CoachLink")
            .field("coach", &self.coach)
            .field("shown", &self.shown)
            .finish_non_exhaustive()
    }
}

impl CoachLink {
    /// Open the engine over `store` (see [`Coach::open`]) and draw its first view.
    ///
    /// # Errors
    /// Any error from [`Coach::open`].
    pub fn open(store: Store, shell: Arc<dyn PetShell>) -> Result<(Self, StepOutput)> {
        let (coach, out) = Coach::open(store)?;
        let mut link = Self {
            coach,
            shell,
            router: Router::default(),
            shown: Shown::default(),
            rev: 0,
            celebrate_until: None,
        };
        link.after(&out);
        Ok((link, out))
    }

    /// The routing snapshot, for the shell's sink.
    #[must_use]
    pub fn router(&self) -> Router {
        self.router.clone()
    }

    /// Apply one engine input now and draw the result.
    ///
    /// # Errors
    /// Any error from [`Coach::handle`]; the engine is rolled back and the shell is left
    /// as it was.
    pub fn input(&mut self, input: Input) -> Result<StepOutput> {
        let out = self.coach.handle(input)?;
        self.after(&out);
        Ok(out)
    }

    /// Apply a routed command. An answer from a bubble that is no longer the one shown
    /// (see [`Command::Answer`]) is ignored.
    ///
    /// # Errors
    /// Any store or engine error.
    pub fn command(&mut self, command: Command) -> Result<StepOutput> {
        match command {
            Command::Coach(i) => self.input(i),
            Command::Answer { bubble_id, input } => {
                if self.shown_bubble_id() == Some(bubble_id.as_str()) {
                    self.input(input)
                } else {
                    // The card was answered, replaced or withdrawn after this answer was
                    // routed: nothing changes, and nothing needs redrawing.
                    Ok(self.unchanged("that card was already answered"))
                }
            }
            Command::LogWater { ml } => {
                let input = self.log_water(ml)?;
                self.input(input)
            }
            Command::NotePhone => {
                let entry = self.record(EventPayload::PhoneInterruption {
                    label: PhoneLabel::Unlabelled,
                })?;
                self.input(Input::Phone(PhoneInput::Noted { entry }))
            }
            Command::Change { bubble_id, change } => {
                if self.shown_bubble_id() != Some(bubble_id.as_str()) {
                    return Ok(self.unchanged("that card was already answered"));
                }
                match self.change(change)? {
                    Some(input) => self.input(input),
                    None => Ok(self.unchanged("nothing to change")),
                }
            }
        }
    }

    /// The output of a command that changed nothing.
    fn unchanged(&self, why: &'static str) -> StepOutput {
        StepOutput {
            outcome: InputOutcome::Ignored(why),
            view: self.coach.view().clone(),
            notices: Vec::new(),
            events: Vec::new(),
            touched_days: Vec::new(),
            significant: false,
        }
    }

    /// Record a new self-reported entry now; returns its id.
    fn record(&mut self, payload: EventPayload) -> Result<Uuid> {
        let now = self.coach.store().clock().now();
        let event = NewEvent::manual(payload).at(now);
        Ok(self.coach.store_mut().append_event(event)?.id)
    }

    /// Record a new version of entry `id` (a correction, or a retraction); returns its id.
    fn supersede(&mut self, id: Uuid, payload: EventPayload) -> Result<Uuid> {
        let event = NewEvent::manual(payload).superseding(id);
        Ok(self.coach.store_mut().append_event(event)?.id)
    }

    fn log_water(&mut self, ml: u32) -> Result<Input> {
        let entry = self.record(EventPayload::WaterLogged { amount_ml: ml })?;
        Ok(Input::Water(WaterInput::Logged { entry, ml }))
    }

    /// Record `change` against the card on show; returns what to tell the engine, or
    /// `None` if the card offers nothing to change (an entry already undone, the label
    /// already chosen).
    fn change(&mut self, change: EntryChange) -> Result<Option<Input>> {
        let view = self.coach.view();
        let bubble = view.bubble;
        let retract = EventPayload::EntryRetracted {};
        let input = match (change, bubble) {
            (EntryChange::LogWater { ml }, Some(CoachBubble::WaterPanel { .. })) => {
                self.log_water(ml)?
            }
            (EntryChange::AddOther, Some(CoachBubble::WaterPanel { other_ml, .. })) => {
                self.log_water(other_ml)?
            }
            (
                EntryChange::UndoWater,
                Some(CoachBubble::WaterLogged {
                    entry: Some(id), ..
                }),
            ) => {
                self.supersede(id, retract)?;
                Input::Water(WaterInput::Undone)
            }
            (
                EntryChange::SaveEdit,
                Some(CoachBubble::WaterPanel {
                    edit: Some(edit), ..
                }),
            ) => {
                let Some(now) = view.water_entries.iter().find(|e| e.id == edit.entry) else {
                    return Ok(None);
                };
                if now.amount_ml != edit.ml {
                    self.supersede(edit.entry, EventPayload::WaterLogged { amount_ml: edit.ml })?;
                }
                Input::Water(WaterInput::Changed)
            }
            (EntryChange::RemoveWater(id), Some(CoachBubble::WaterPanel { .. })) => {
                if !view.water_entries.iter().any(|e| e.id == id) {
                    return Ok(None);
                }
                self.supersede(id, retract)?;
                Input::Water(WaterInput::Changed)
            }
            (
                EntryChange::LabelPhone(label),
                Some(CoachBubble::PhoneNote {
                    entry: Some(id),
                    label: chosen,
                    ..
                }),
            ) => {
                if chosen == Some(label) {
                    return Ok(None);
                }
                let entry = self.supersede(id, EventPayload::PhoneInterruption { label })?;
                Input::Phone(PhoneInput::Labelled { entry, label })
            }
            (
                EntryChange::UndoPhone,
                Some(CoachBubble::PhoneNote {
                    entry: Some(id), ..
                }),
            ) => {
                self.supersede(id, retract)?;
                Input::Phone(PhoneInput::Undone)
            }
            _ => return Ok(None),
        };
        Ok(Some(input))
    }

    /// Change settings through the engine's store connection (a settings screen, or a
    /// suggestion applied from the weekly review), then refresh so the engine and the pet
    /// follow them at once. Nothing is refreshed if `f` fails.
    ///
    /// # Errors
    /// What `f` returns, or a store error from the refresh.
    pub fn change_settings<T>(&mut self, f: impl FnOnce(&mut Store) -> Result<T>) -> Result<T> {
        let out = f(self.coach.store_mut())?;
        self.input(Input::Refresh)?;
        Ok(out)
    }

    /// Save anything not yet saved (on exit).
    ///
    /// # Errors
    /// Any store error.
    pub fn flush(&mut self) -> Result<()> {
        self.coach.flush()
    }

    /// The engine's last view.
    #[must_use]
    pub fn view(&self) -> &CoachView {
        self.coach.view()
    }

    /// The engine.
    #[must_use]
    pub const fn coach(&self) -> &Coach {
        &self.coach
    }

    /// The id of the bubble the link last showed and has not closed.
    #[must_use]
    pub fn shown_bubble_id(&self) -> Option<&str> {
        self.shown.bubble.as_ref().map(|(_, b)| b.id.as_str())
    }

    /// Whether the link has the pet hidden for a presentation.
    #[must_use]
    pub const fn hidden_for_presentation(&self) -> bool {
        self.shown.hidden
    }

    fn after(&mut self, out: &StepOutput) {
        let now = self.coach.store().clock().monotonic();
        if out
            .notices
            .iter()
            .any(|n| matches!(n, Notice::Celebrate(_)))
        {
            self.celebrate_until = Some(now + CELEBRATE_FOR);
        }
        let celebrating = self.celebrate_until.is_some_and(|t| now < t);
        if !celebrating {
            self.celebrate_until = None;
        }
        let view = out.view.clone();
        if self.render(&view, celebrating) {
            // The shell refused the bubble because the user hid the pet: collapse it to
            // the waiting badge rather than show it later. The engine then has no bubble,
            // so this does not repeat.
            if let Ok(next) = self.coach.handle(Input::CloseBubble) {
                self.render(&next.view, celebrating);
            }
        }
    }

    /// Draw `view` on the shell. Returns true if the shell refused a new bubble.
    fn render(&mut self, view: &CoachView, celebrating: bool) -> bool {
        let look = present::pet_look(view, celebrating);
        let menu = present::menu_actions(view);
        let want = view
            .bubble
            .as_ref()
            .map(|b| (BubbleKind::of(b), present::bubble(b, view)));

        // The bubble to show: the same reminder stays as it is (its counted minutes are
        // not redrawn every tick) unless its chips or actions changed; any other card is
        // redrawn when anything in it changed (the guide's next movement, a new date).
        let show = match (&self.shown.bubble, want) {
            (Some((k, old)), Some((k2, new))) if *k == k2 && same_card(*k, old, &new) => None,
            (_, Some((k, mut b))) => {
                self.rev += 1;
                b.id = k.id(self.rev);
                Some((k, b))
            }
            (_, None) => None,
        };
        let close = match (&self.shown.bubble, &show, view.bubble.is_some()) {
            (Some((_, old)), None, false) => Some(old.id.clone()),
            _ => None,
        };

        // Routing first, so an event from the shell is judged against what it now shows.
        let bubble_id = match (&show, &close) {
            (Some((_, b)), _) => Some(b.id.clone()),
            (None, Some(_)) => None,
            (None, None) => self.shown.bubble.as_ref().map(|(_, b)| b.id.clone()),
        };
        self.router.set(RouteState {
            bubble_id,
            click_reopens: present::click_reopens(view),
        });

        if !view.visible && !self.shown.hidden {
            if let Some((_, b)) = self.shown.bubble.take() {
                self.shell.close_bubble(&b.id);
            }
            self.shell.hide(HideReason::Presentation);
            self.shown.hidden = true;
        }
        if self.shown.look.as_ref() != Some(&look) {
            self.shell.set_state(look.0, look.1.clone());
            self.shown.look = Some(look);
        }
        if self.shown.menu.as_ref() != Some(&menu) {
            self.shell.set_menu_actions(menu.clone());
            self.shown.menu = Some(menu);
        }
        if view.visible && self.shown.hidden {
            self.shell.unhide(HideReason::Presentation);
            self.shown.hidden = false;
        }
        if let Some(id) = close {
            self.shell.close_bubble(&id);
            self.shown.bubble = None;
        }
        if let Some((k, b)) = show {
            match self.shell.show_bubble(b.clone()) {
                BubbleOutcome::Shown => self.shown.bubble = Some((k, b)),
                BubbleOutcome::Suppressed | BubbleOutcome::Invalid => {
                    self.shown.bubble = None;
                    let mut state = self.router.state();
                    state.bubble_id = None;
                    self.router.set(state);
                    return true;
                }
            }
        }
        false
    }
}

/// Whether `new` (without an id) shows the same card as `old`.
fn same_card(kind: BubbleKind, old: &Bubble, new: &Bubble) -> bool {
    match kind {
        BubbleKind::Reminder { .. } => old.chips == new.chips && old.actions == new.actions,
        _ => {
            Bubble {
                id: String::new(),
                ..old.clone()
            } == *new
        }
    }
}

#[cfg(test)]
mod tests;
