//! The guided movement routine (slice V1-3; wireframe B4, flow 3, MOV-01 and MOV-02).
//!
//! The guide is the one bubble while it runs, and an exercise for precedence (level 3):
//! no reminder or card appears over it. It opens from "Start movement" on a movement
//! reminder or from "Today's movement" on the pet menu, and shows today's routine from
//! the bundled, provisional content ([`crate::coach::movement`]).
//!
//! - First screen: the routine's three movements, the seating check, and Seated,
//!   Standing, Later, Skip today or Other day. Sunday is optional: any day's routine may be
//!   chosen.
//! - Each movement: Done, Skip this movement or Stop routine. Each answer is a
//!   `movement.step` event.
//! - The result is a `movement.session` event: every movement done is "done"; some done is
//!   "partial" (never complete); none done (an immediate Stop, or every movement skipped)
//!   is "none". Done and partial restart the movement timer; none keeps the calm gap and
//!   the next due time and does not restart it (wireframe 10.1). "Skip today" is a skip.
//!   No catch-up is ever owed.
//! - Closing the card (its close button, a click on the pet, hiding the pet) only puts it
//!   behind the pet; a click brings it back at the same movement.
//! - A hold that was not in force when the guide opened (a lock, sleep, pause, focus,
//!   presentation, meeting or the end of work hours), the app restarting, or 30 minutes
//!   without a result ends the routine. Movements already answered are kept: the result
//!   is recorded from them as a stop (D). With nothing answered, nothing is recorded and a
//!   routine started from a reminder shows "none recorded", as before.

use chrono::{Datelike, Weekday};
use serde::{Deserialize, Serialize};
use uuid::{Builder, Uuid};

use super::{
    CALM_GAP, ChipStatus, Context, Engine, EventPayload, HoldSet, InputOutcome, NewEvent,
    OfferPhase, Out, ROUTINE_TIMEOUT, RoutineOutcome, ms,
};
use crate::coach::movement::{CONTENT_VERSION, MOVEMENTS_PER_ROUTINE, routine_for};
use crate::coach::types::{GuideAction, GuideStage, GuideView};
use crate::schema::{Activity, RoutineVariant, StepOutcome};

/// How long the result card stays before it closes by itself (D).
pub const RESULT_SHOWN_FOR: std::time::Duration = std::time::Duration::from_secs(60);

/// Where the guide is (saved with the engine state).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Stage {
    Intro,
    ChooseDay,
    Step,
    Result {
        outcome: RoutineOutcome,
        stopped: bool,
    },
}

/// The open guided routine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Guide {
    /// The routine's id, shared by its step and result events.
    id: Uuid,
    /// The movement reminder it was started from, if any.
    offer: Option<Uuid>,
    /// The routine's day (Sunday until one is chosen on a Sunday).
    day: Weekday,
    variant: Option<RoutineVariant>,
    stage: Stage,
    /// Zero-based index of the movement on screen.
    step: u32,
    done: u32,
    skipped: u32,
    /// The card is shown; false while it waits behind the pet.
    pub(super) open: bool,
    /// Engine time it opened (the 30-minute limit).
    since: u64,
    /// Engine time the result appeared.
    result_since: u64,
    /// The holds in force when it opened; it ends when any other hold starts.
    opened_under: HoldSet,
}

impl Guide {
    const fn running(&self) -> bool {
        !matches!(self.stage, Stage::Result { .. })
    }
}

impl Engine {
    /// The id of the `seq`-th guided routine. It cannot equal a reminder id: those use
    /// the salt itself in their first half.
    fn guide_id(&self, seq: u64) -> Uuid {
        let mut bytes = [0u8; 16];
        bytes[..8].copy_from_slice(&(self.s.salt ^ 0x9e37_79b9_7f4a_7c15).to_be_bytes());
        bytes[8..].copy_from_slice(&seq.to_be_bytes());
        Builder::from_random_bytes(bytes).into_uuid()
    }

    /// Open the guide on today's routine.
    pub(super) fn open_guide(&mut self, ctx: &Context, offer: Option<Uuid>, out: &mut Out) {
        self.s.guide_seq += 1;
        let now = self.s.now;
        self.s.guide = Some(Guide {
            id: self.guide_id(self.s.guide_seq),
            offer,
            day: ctx.zone.local_date(ctx.wall).weekday(),
            variant: None,
            stage: Stage::Intro,
            step: 0,
            done: 0,
            skipped: 0,
            open: true,
            since: now,
            result_since: now,
            opened_under: self.holds_above_pomodoro(ctx),
        });
        // One bubble: an open Pomodoro card closes (its phase stays for the menu).
        if let Some(p) = self.s.pomodoro.as_mut() {
            p.card_open = false;
            p.card_opened_under = HoldSet::EMPTY;
        }
        out.significant = true;
    }

    /// The guide for the view.
    pub(super) fn guide_view(&self, ctx: &Context) -> Option<GuideView> {
        let g = self.s.guide?;
        let total = MOVEMENTS_PER_ROUTINE;
        let stage = match g.stage {
            Stage::Intro if routine_for(g.day).is_none() => GuideStage::Sunday,
            Stage::Intro => GuideStage::Intro,
            Stage::ChooseDay => GuideStage::ChooseDay,
            Stage::Step => GuideStage::Step {
                index: g.step,
                total,
            },
            Stage::Result { outcome, stopped } => GuideStage::Result {
                outcome,
                done: g.done,
                skipped: g.skipped,
                total,
                stopped,
            },
        };
        Some(GuideView {
            day: g.day,
            today: ctx.zone.local_date(ctx.wall).weekday(),
            variant: g.variant,
            stage,
            from_reminder: g.offer.is_some(),
            open: g.open,
        })
    }

    /// An answer in the guide.
    pub(super) fn guide_input(
        &mut self,
        ctx: &Context,
        action: GuideAction,
        out: &mut Out,
    ) -> InputOutcome {
        use InputOutcome::{Applied, Ignored};
        let Some(mut g) = self.s.guide else {
            return Ignored("no movement guide");
        };
        let scheduled = routine_for(g.day).is_some();
        match (action, g.stage) {
            (GuideAction::Close, Stage::Result { .. }) => {
                self.s.guide = None;
                return Applied;
            }
            (_, Stage::Result { .. }) => return Ignored("the routine is over"),
            (GuideAction::Variant(v), Stage::Intro) if scheduled => {
                g.variant = Some(v);
                g.stage = Stage::Step;
            }
            (GuideAction::Day(d), Stage::Intro | Stage::ChooseDay) => {
                if routine_for(d).is_none() {
                    return InputOutcome::Invalid("choose a day from Monday to Saturday");
                }
                g.day = d;
                g.stage = Stage::Intro;
            }
            (GuideAction::OtherDay, Stage::Intro) => g.stage = Stage::ChooseDay,
            (GuideAction::Back, Stage::ChooseDay) => g.stage = Stage::Intro,
            (GuideAction::Later, Stage::Intro | Stage::ChooseDay) => {
                self.guide_later(g, out);
                return Applied;
            }
            (GuideAction::SkipToday, Stage::Intro | Stage::ChooseDay) if scheduled => {
                g.open = true;
                self.s.guide = Some(g);
                self.finish_guide(ctx, RoutineOutcome::Skipped, false, out);
                return Applied;
            }
            (GuideAction::Done | GuideAction::Skip, Stage::Step) => {
                let done = action == GuideAction::Done;
                if let Some(e) = self.step_event(g, done) {
                    out.events.push(e.at(ctx.wall));
                }
                if done {
                    g.done += 1;
                } else {
                    g.skipped += 1;
                }
                g.step += 1;
                g.open = true;
                self.s.guide = Some(g);
                if g.step >= MOVEMENTS_PER_ROUTINE {
                    let outcome = self.outcome_so_far(g);
                    self.finish_guide(ctx, outcome, false, out);
                }
                return Applied;
            }
            (GuideAction::Stop, Stage::Step) => {
                let outcome = self.outcome_so_far(g);
                self.finish_guide(ctx, outcome, true, out);
                return Applied;
            }
            _ => return Ignored("not on this screen"),
        }
        g.open = true;
        self.s.guide = Some(g);
        Applied
    }

    fn step_event(&self, g: Guide, done: bool) -> Option<NewEvent> {
        let r = routine_for(g.day)?;
        Some(NewEvent::manual(EventPayload::MovementStep {
            session_id: g.id,
            routine_id: r.id.to_owned(),
            routine_version: CONTENT_VERSION,
            variant: g.variant?,
            step_index: g.step,
            steps_total: MOVEMENTS_PER_ROUTINE,
            outcome: if done {
                StepOutcome::Done
            } else {
                StepOutcome::Skipped
            },
        }))
    }

    const fn outcome_so_far(&self, g: Guide) -> RoutineOutcome {
        if g.done >= MOVEMENTS_PER_ROUTINE {
            RoutineOutcome::Done
        } else if g.done > 0 {
            RoutineOutcome::Partial
        } else {
            RoutineOutcome::None
        }
    }

    /// Record the routine's result, apply it to the movement timer and the reminder it
    /// came from, and show the result.
    fn finish_guide(
        &mut self,
        ctx: &Context,
        outcome: RoutineOutcome,
        stopped: bool,
        out: &mut Out,
    ) {
        let Some(mut g) = self.s.guide else {
            return;
        };
        if let Some(r) = routine_for(g.day) {
            out.events.push(
                NewEvent::manual(EventPayload::MovementSession {
                    session_id: g.id,
                    routine_id: r.id.to_owned(),
                    routine_version: CONTENT_VERSION,
                    variant: g.variant,
                    outcome,
                    steps_done: g.done,
                    steps_skipped: g.skipped,
                    steps_total: MOVEMENTS_PER_ROUTINE,
                })
                .at(ctx.wall),
            );
        }
        let offer = g.offer.filter(|id| {
            self.current_offer(*id).is_some_and(|o| {
                o.activity == Activity::Movement && matches!(o.phase, OfferPhase::InProgress { .. })
            })
        });
        if g.offer.is_some() && offer.is_none() {
            // The reminder is no longer in progress: only the movement timer follows.
            if outcome.resets_movement_timer() {
                self.reset_timer(Activity::Movement);
            }
            self.s.gap_until = self.s.now + ms(CALM_GAP);
        } else {
            let _ = self.routine_finished(ctx, offer, outcome, out);
        }
        g.stage = Stage::Result { outcome, stopped };
        g.result_since = self.s.now;
        self.s.guide = Some(g);
        out.significant = true;
    }

    /// "Later": from a reminder it snoozes movement; from the menu it closes, recording
    /// nothing.
    fn guide_later(&mut self, g: Guide, out: &mut Out) {
        self.s.guide = None;
        out.significant = true;
        if let Some(o) = g.offer.and_then(|id| self.current_offer(id))
            && o.activity == Activity::Movement
            && matches!(o.phase, OfferPhase::InProgress { .. })
        {
            self.snooze(Activity::Movement);
            self.finish_offer(o, ChipStatus::Snoozed, out);
        } else {
            self.s.exercise_since = None;
            self.s.gap_until = self.s.now + ms(CALM_GAP);
        }
    }

    /// End a running routine that the user did not finish (a new hold, a restart, the
    /// 30-minute limit, a Pomodoro started). Answered movements are kept as a stop; with
    /// none answered nothing is recorded. No result card is shown.
    pub(super) fn interrupt_guide(&mut self, ctx: &Context, out: &mut Out) {
        let Some(g) = self.s.guide else {
            return;
        };
        out.significant = true;
        if g.running() && g.stage == Stage::Step && g.done + g.skipped > 0 {
            let outcome = self.outcome_so_far(g);
            self.finish_guide(ctx, outcome, true, out);
        } else if g.running() {
            if let Some(o) = self.s.offer
                && Some(o.id) == g.offer
                && matches!(o.phase, OfferPhase::InProgress { .. })
            {
                self.abandon_in_progress(out);
            } else {
                self.s.exercise_since = None;
                self.s.gap_until = self.s.now + ms(CALM_GAP);
            }
        }
        self.s.guide = None;
    }

    /// The close button, a click on the pet, or hiding the pet: a running guide waits
    /// behind the pet; a result closes. Returns false if no guide card is open.
    pub(super) fn close_guide_card(&mut self, out: &mut Out) -> bool {
        match self.s.guide.as_mut() {
            Some(g) if g.open && g.running() => {
                g.open = false;
                out.significant = true;
                true
            }
            Some(g) if g.open => {
                self.s.guide = None;
                out.significant = true;
                true
            }
            _ => false,
        }
    }

    /// A hold that was not in force when the guide opened ends it (or closes its result);
    /// a hold that ended no longer covers it if it begins again.
    pub(super) fn settle_guide(&mut self, ctx: &Context, out: &mut Out) {
        let Some(g) = self.s.guide else {
            return;
        };
        let holds = self.holds_above_pomodoro(ctx);
        if !holds.within(g.opened_under) {
            self.interrupt_guide(ctx, out);
        } else if g.opened_under != holds
            && let Some(g) = self.s.guide.as_mut()
        {
            g.opened_under = holds;
            out.significant = true;
        }
    }

    /// The 30-minute limit and the result card's own time.
    pub(super) fn run_guide_timers(&mut self, ctx: &Context, out: &mut Out) {
        let Some(g) = self.s.guide else {
            return;
        };
        let now = self.s.now;
        if g.running() && now.saturating_sub(g.since) >= ms(ROUTINE_TIMEOUT) {
            self.interrupt_guide(ctx, out);
        } else if !g.running() && now.saturating_sub(g.result_since) >= ms(RESULT_SHOWN_FOR) {
            self.s.guide = None;
            out.significant = true;
        }
    }
}
