//! The water confirmation, the water panel and the phone note (slice V1-3 follow-up;
//! wireframes B3 and B8, flow 2; WAT-01, WAT-02 and DIS-01).
//!
//! These cards answer something the user just did, so they are not reminders:
//!
//! - The shell records the entry first (`water.logged`, `phone.interruption`; Undo,
//!   Remove, Edit and a new label are new events that supersede it, so totals
//!   recalculate) and then tells the engine ([`WaterInput`], [`PhoneInput`]), which shows
//!   the card in the one bubble. Logging never needs the card: while the pet is hidden the
//!   entry is recorded and no card opens.
//! - A card the user asks for opens in place of the card on show, which closes as its
//!   close button would close it: a reminder waits as its badge, the movement guide waits
//!   behind the pet, a Pomodoro card stays in the menu, any other card closes (D).
//! - While one is open, no reminder or card opens on its own. A card the user opens (the
//!   movement guide, "Deadline day", a waiting reminder) replaces it. A Pomodoro card that
//!   falls due meanwhile waits until it closes.
//! - It stays under the holds in force when it opened, and closes when another hold
//!   starts or the pet is hidden. Nothing is replayed.
//! - It closes by itself [`NOTE_SHOWN_FOR`] after the last answer on it (the panel after
//!   [`WATER_PANEL_SHOWN_FOR`]). Its close counts as an answer: the 5-minute calm gap
//!   follows before a reminder (D). A restart closes it.
//! - "Already did it" on a water reminder opens the panel (wireframe B2), where the user
//!   can log what they drank.

use chrono::NaiveTime;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{CALM_GAP, Context, Engine, HoldSet, InputOutcome, Notice, OfferPhase, Out, ms};
use crate::coach::config::{NOTE_SHOWN_FOR, WATER_PANEL_SHOWN_FOR};
use crate::coach::types::{Bubble, PhoneInput, WaterEdit, WaterInput};
use crate::coach::water::{WATER_DEFAULT_SERVING_ML, step_amount};
use crate::schema::{Activity, MAX_WATER_ENTRY_ML, PhoneLabel};

/// Which card is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Note {
    /// The quick log's confirmation; `entry` is `None` once Undo removed it.
    Water { entry: Option<Uuid>, ml: u32 },
    /// The water panel.
    Panel {
        edit: Option<WaterEdit>,
        other_ml: u32,
    },
    /// The phone note; `entry` is `None` once Undo removed it.
    Phone {
        entry: Option<Uuid>,
        at: NaiveTime,
        label: Option<PhoneLabel>,
    },
}

/// The open card (saved with the engine state).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NoteCard {
    note: Note,
    /// Engine time of the last answer on it (its own close timer).
    since: u64,
    /// The holds in force when it opened; any other hold closes it.
    opened_under: HoldSet,
}

impl NoteCard {
    const fn shown_for(&self) -> std::time::Duration {
        match self.note {
            Note::Panel { .. } => WATER_PANEL_SHOWN_FOR,
            Note::Water { .. } | Note::Phone { .. } => NOTE_SHOWN_FOR,
        }
    }

    /// The card for the view.
    pub(super) const fn bubble(&self) -> Bubble {
        match self.note {
            Note::Water { entry, ml } => Bubble::WaterLogged { entry, ml },
            Note::Panel { edit, other_ml } => Bubble::WaterPanel { edit, other_ml },
            Note::Phone { entry, at, label } => Bubble::PhoneNote { entry, at, label },
        }
    }
}

impl Engine {
    /// Open `note` for the user in the one bubble. Whatever card is on show closes first,
    /// as its close button would close it. Nothing opens while the pet is hidden.
    fn open_note(&mut self, ctx: &Context, note: Note, out: &mut Out) -> bool {
        if !self.visible(ctx) {
            return false;
        }
        self.close_open_card(ctx, out);
        self.s.note = Some(NoteCard {
            note,
            since: self.s.now,
            opened_under: self.holds_above_pomodoro(ctx),
        });
        out.significant = true;
        true
    }

    /// Close the open note. Like an answer, the calm gap follows.
    pub(super) fn clear_note(&mut self, out: &mut Out) -> bool {
        if self.s.note.take().is_none() {
            return false;
        }
        self.s.gap_until = self.s.gap_until.max(self.s.now + ms(CALM_GAP));
        out.significant = true;
        true
    }

    /// The open note, with its close timer restarted by an answer.
    fn answered_note(&mut self) -> Option<&mut Note> {
        let now = self.s.now;
        self.s.note.as_mut().map(|c| {
            c.since = now;
            &mut c.note
        })
    }

    /// A brief celebration when this entry reaches the user's own goal (prototype), and
    /// nothing after it: no encouragement past the goal (WAT-01).
    fn celebrate_goal(ctx: &Context, ml: u32, out: &mut Out) {
        if let Some(goal) = ctx.water.goal_ml {
            let after = ctx.water.logged_ml;
            if after >= goal && after.saturating_sub(ml) < goal {
                out.notices.push(Notice::Celebrate(Activity::Water));
            }
        }
    }

    pub(super) fn water_input(
        &mut self,
        ctx: &Context,
        input: WaterInput,
        out: &mut Out,
    ) -> InputOutcome {
        use InputOutcome::{Applied, Ignored};
        let panel_open = matches!(
            self.s.note,
            Some(NoteCard {
                note: Note::Panel { .. },
                ..
            })
        );
        match input {
            WaterInput::Logged { entry, ml } => {
                if !(1..=MAX_WATER_ENTRY_ML).contains(&ml) {
                    return InputOutcome::Invalid("water amount must be 1 to 2000 ml");
                }
                Self::celebrate_goal(ctx, ml, out);
                if panel_open {
                    // Logged in the panel: it stays and lists the entry.
                    self.answered_note();
                } else {
                    self.open_note(
                        ctx,
                        Note::Water {
                            entry: Some(entry),
                            ml,
                        },
                        out,
                    );
                }
                Applied
            }
            WaterInput::Undone => match self.answered_note() {
                Some(Note::Water { entry, .. }) if entry.is_some() => {
                    *entry = None;
                    Applied
                }
                _ => Ignored("nothing to undo"),
            },
            WaterInput::OpenPanel => {
                let panel = Note::Panel {
                    edit: None,
                    other_ml: WATER_DEFAULT_SERVING_ML,
                };
                if self.open_note(ctx, panel, out) {
                    Applied
                } else {
                    Ignored("the pet is hidden")
                }
            }
            WaterInput::Edit(id) => {
                let Some(e) = ctx.water_entries.iter().find(|e| e.id == id).copied() else {
                    return Ignored("no such entry today");
                };
                match self.answered_note() {
                    Some(Note::Panel { edit, .. }) => {
                        *edit = Some(WaterEdit {
                            entry: e.id,
                            ml: e.amount_ml,
                        });
                        Applied
                    }
                    _ => Ignored("the water panel is not open"),
                }
            }
            WaterInput::StepEdit { up } => match self.answered_note() {
                Some(Note::Panel { edit: Some(e), .. }) => {
                    e.ml = step_amount(e.ml, up);
                    Applied
                }
                _ => Ignored("no entry is being edited"),
            },
            WaterInput::StepOther { up } => match self.answered_note() {
                Some(Note::Panel { other_ml, .. }) => {
                    *other_ml = step_amount(*other_ml, up);
                    Applied
                }
                _ => Ignored("the water panel is not open"),
            },
            WaterInput::CancelEdit => match self.answered_note() {
                Some(Note::Panel { edit, .. }) if edit.is_some() => {
                    *edit = None;
                    Applied
                }
                _ => Ignored("no entry is being edited"),
            },
            WaterInput::Changed => match self.answered_note() {
                Some(Note::Panel { edit, .. }) => {
                    *edit = None;
                    Applied
                }
                _ => Ignored("the water panel is not open"),
            },
        }
    }

    pub(super) fn phone_input(
        &mut self,
        ctx: &Context,
        input: PhoneInput,
        out: &mut Out,
    ) -> InputOutcome {
        use InputOutcome::{Applied, Ignored};
        match input {
            PhoneInput::Noted { entry } => {
                let at = ctx.zone.local_datetime(ctx.wall).time();
                self.open_note(
                    ctx,
                    Note::Phone {
                        entry: Some(entry),
                        at,
                        label: None,
                    },
                    out,
                );
                Applied
            }
            PhoneInput::Labelled { entry, label } => match self.answered_note() {
                Some(Note::Phone {
                    entry: current,
                    label: chosen,
                    ..
                }) if current.is_some() => {
                    *current = Some(entry);
                    *chosen = Some(label);
                    Applied
                }
                _ => Ignored("no phone note open"),
            },
            PhoneInput::Undone => match self.answered_note() {
                Some(Note::Phone { entry, .. }) if entry.is_some() => {
                    *entry = None;
                    Applied
                }
                _ => Ignored("nothing to undo"),
            },
        }
    }

    /// The note gives way to another card, a new hold, a hidden pet and its own time; an
    /// entry being edited that no longer stands stops being edited.
    pub(super) fn settle_note(&mut self, ctx: &Context, out: &mut Out) {
        let Some(card) = self.s.note else {
            return;
        };
        let other_card = self
            .s
            .offer
            .is_some_and(|o| matches!(o.phase, OfferPhase::Showing { .. }))
            || self.s.guide.is_some_and(|g| g.open)
            || self.s.prompt.is_some();
        let holds = self.holds_above_pomodoro(ctx);
        let expired = self.s.now.saturating_sub(card.since) >= ms(card.shown_for());
        if !self.visible(ctx) || other_card || !holds.within(card.opened_under) || expired {
            self.clear_note(out);
            return;
        }
        let mut next = card;
        // A hold that ended no longer covers the card if it begins again.
        next.opened_under = holds;
        if let Note::Panel {
            edit: Some(e),
            other_ml,
        } = card.note
            && !ctx.water_entries.iter().any(|w| w.id == e.entry)
        {
            next.note = Note::Panel {
                edit: None,
                other_ml,
            };
        }
        if next != card {
            self.s.note = Some(next);
            out.significant = true;
        }
    }

    /// "Already did it" on a water reminder: the panel, to log what was drunk.
    pub(super) fn open_water_panel_after_reminder(&mut self, ctx: &Context, out: &mut Out) {
        self.open_note(
            ctx,
            Note::Panel {
                edit: None,
                other_ml: WATER_DEFAULT_SERVING_ML,
            },
            out,
        );
    }
}
