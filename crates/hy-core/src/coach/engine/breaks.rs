//! The combined 5-minute break in the engine (DEC15; the owner's request of 2 Nov 2026).
//!
//! While [`crate::coach::ReminderStyle::Combined`] is on (the default), it replaces the
//! separate movement, eye-rest and water reminders: none of them is offered, and water
//! that falls behind pace is still shown but raises no reminder of its own.
//!
//! - **Cadence:** one offer every [`crate::coach::CoachConfig::break_minutes`] (30 by
//!   default) of time at the PC inside work hours, not paused: the DEC13 presence (the
//!   app running, the session unlocked, the PC awake), whatever the keyboard and mouse
//!   say. The timer starts afresh with each work window and after a confirmed break (a
//!   lock or sleep of 2 minutes or more), as the movement and eye-rest timers do.
//! - **Offer:** "Time for a 5-minute break" in the one bubble, with Start, Snooze 10 min,
//!   Skip and Not now. It follows every reminder rule: the holds, the calm gap, the grace
//!   period, the waiting badge, "offered, no response", and nothing replayed after a hold.
//!   It is recorded as a movement reminder (`reminder.offered` and `reminder.response`
//!   with activity `movement`), so the reviews count it as a break offered or taken and
//!   the shared outcomes count it as a reminder.
//! - **Steps**, in order, each skippable: water (10 s, with "Log 250 ml", a real water
//!   entry), eyes (15 s), the guided movement routine (the movement guide, marked
//!   provisional), and closed-eye breathing for what is left of the 5 minutes (60 to
//!   90 s). A timed step whose time runs out on screen counts as done. Steps whose
//!   category is switched off are left out; breathing always stays.
//! - **Result:** one `break.session` event with the steps done. Any step done restarts the
//!   break timer, and the eye-rest or movement timer when that step was done (all three
//!   for a finished break). None done (an immediate Stop, or every step skipped) is
//!   "none": the next break comes a full interval later and the movement timer is not
//!   restarted (wireframe 10.1).
//! - **Ending early:** Stop, the close button, any hold that starts (a lock, sleep, pause,
//!   focus, presentation, meeting, the end of work hours), the pet being hidden, a
//!   Pomodoro started, the app closing, or 30 minutes in all end it as stopped, with the
//!   steps done so far.

use serde::{Deserialize, Serialize};
use uuid::{Builder, Uuid};

use super::guide::RESULT_SHOWN_FOR;
use super::{
    ActivityState, ChipStatus, Context, Engine, EventPayload, HoldSet, InputOutcome,
    MeasurementStatus, NewEvent, Offer, OfferPhase, Out, ROUTINE_TIMEOUT, ReminderOutcome,
    Response, SNOOZE, dur, ms,
};
use crate::coach::break_routine::{BreakStep, EYES_HOLD, WATER_HOLD, breathe_for};
use crate::coach::types::{BreakAction, BreakStage, BreakView, Notice};
use crate::coach::water::WATER_DEFAULT_SERVING_ML;
use crate::schema::{Activity, RoutineOutcome};

/// The combined break's timer and the break in progress (saved with the engine state).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Breaks {
    /// Time at the PC since the last break, its snooze, and its last outcome today.
    pub(super) timer: ActivityState,
    /// Breaks started (with the engine's salt, makes each break's id unique).
    seq: u64,
    /// The break, running or on its last screen.
    session: Option<Session>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Session {
    id: Uuid,
    /// The break reminder it was started from.
    offer: Uuid,
    /// The steps offered and the steps done, one bit each.
    steps: u8,
    done: u8,
    /// The step on screen; `None` on the last screen.
    step: Option<BreakStep>,
    /// Engine time the break started (the 5 minutes and the 30-minute limit).
    started: u64,
    /// Engine time the step on screen began, and its time on screen (0: untimed).
    step_started: u64,
    step_ms: u64,
    /// Water logged on the water step.
    water_ml: Option<u32>,
    /// The holds in force when it started; any other hold ends it.
    opened_under: HoldSet,
    /// Engine time the last screen appeared, and the result it shows.
    done_at: Option<u64>,
    outcome: RoutineOutcome,
}

const fn bit(step: BreakStep) -> u8 {
    match step {
        BreakStep::Water => 1,
        BreakStep::Eyes => 1 << 1,
        BreakStep::Movement => 1 << 2,
        BreakStep::Breathe => 1 << 3,
    }
}

fn listed(mask: u8) -> Vec<BreakStep> {
    BreakStep::ALL
        .iter()
        .copied()
        .filter(|s| mask & bit(*s) != 0)
        .collect()
}

impl Session {
    const fn running(&self) -> bool {
        self.done_at.is_none()
    }

    /// The first step offered after `after` (from the start with `None`).
    fn next_step(&self, after: Option<BreakStep>) -> Option<BreakStep> {
        BreakStep::ALL
            .iter()
            .copied()
            .filter(|s| after.is_none_or(|a| *s > a))
            .find(|s| self.steps & bit(*s) != 0)
    }

    fn outcome_so_far(&self) -> RoutineOutcome {
        if self.done == 0 {
            RoutineOutcome::None
        } else if self.done & self.steps == self.steps {
            RoutineOutcome::Done
        } else {
            RoutineOutcome::Partial
        }
    }
}

impl Engine {
    /// The id of the `seq`-th break. It cannot equal a reminder, routine or pause id:
    /// those use the salt itself, or the salt mixed with other constants.
    fn break_id(&self, seq: u64) -> Uuid {
        let mut bytes = [0u8; 16];
        bytes[..8].copy_from_slice(&(self.s.salt ^ 0x2545_f491_4f6c_dd1d).to_be_bytes());
        bytes[8..].copy_from_slice(&seq.to_be_bytes());
        Builder::from_random_bytes(bytes).into_uuid()
    }

    /// A break is running (not on its last screen): an exercise, level 3.
    pub(super) fn break_running(&self) -> bool {
        self.s.breaks.session.is_some_and(|b| b.running())
    }

    /// A break (running or on its last screen) holds the bubble.
    pub(super) const fn break_open(&self) -> bool {
        self.s.breaks.session.is_some()
    }

    /// The break is due: combined breaks are on and the interval has passed, or a snooze
    /// has ended.
    pub(super) fn break_due(&self, ctx: &Context) -> bool {
        if !ctx.config.combined() {
            return false;
        }
        let t = &self.s.breaks.timer;
        match t.snooze_until {
            Some(u) => self.s.now >= u,
            None => t.accum_ms >= t.base_ms.saturating_add(ms(ctx.config.break_every())),
        }
    }

    /// Time at the PC counts towards the next break.
    pub(super) const fn count_break_time(&mut self, len: u64) {
        self.s.breaks.timer.accum_ms = self.s.breaks.timer.accum_ms.saturating_add(len);
    }

    /// The next break a full interval from now (skip, not now, no response, none done,
    /// missed while quiet).
    pub(super) const fn reschedule_break(&mut self) {
        let t = &mut self.s.breaks.timer;
        t.base_ms = t.accum_ms;
        t.snooze_until = None;
        t.due_since = None;
    }

    /// The break timer starts afresh (a break taken, a confirmed break, a new work window).
    pub(super) const fn reset_break_timer(&mut self) {
        let t = &mut self.s.breaks.timer;
        t.accum_ms = 0;
        t.base_ms = 0;
        t.snooze_until = None;
        t.due_since = None;
    }

    /// Track when the break became due.
    pub(super) fn track_break_due(&mut self, ctx: &Context) {
        let due = self.break_due(ctx);
        let now = self.s.now;
        let t = &mut self.s.breaks.timer;
        match (due, t.due_since) {
            (true, None) => t.due_since = Some(now),
            (false, Some(_)) => t.due_since = None,
            _ => {}
        }
    }

    /// Show the break reminder.
    pub(super) fn show_break(&mut self, ctx: &Context, out: &mut Out) {
        self.s.offer_seq += 1;
        let seq = self.s.offer_seq;
        let id = self.offer_id(seq);
        let now = self.s.now;
        self.s.offer = Some(Offer {
            id,
            seq,
            activity: Activity::Movement,
            phase: OfferPhase::Showing { since: now },
            first_shown: now,
            combined: true,
        });
        // A snooze returns the break once.
        self.s.breaks.timer.snooze_until = None;
        out.events.push(
            NewEvent::inferred(
                EventPayload::ReminderOffered {
                    reminder_id: id,
                    activity: Activity::Movement,
                },
                MeasurementStatus::Measured,
            )
            .at(ctx.wall),
        );
        out.notices.push(Notice::ReminderShown {
            offer: id,
            activity: Activity::Movement,
        });
        out.significant = true;
    }

    /// An answer on the break reminder.
    pub(super) fn respond_break(
        &mut self,
        ctx: &Context,
        o: Offer,
        r: Response,
        out: &mut Out,
    ) -> InputOutcome {
        match r {
            Response::Done => {
                // "Already did it": a break taken on the user's own.
                out.events
                    .push(Self::response_event(ctx, o, ReminderOutcome::Done));
                self.reset_break_timer();
                self.reset_timer(Activity::EyeRest);
                self.reset_timer(Activity::Movement);
                self.finish_offer(o, ChipStatus::Done, out);
                out.notices.push(Notice::Celebrate(Activity::Movement));
            }
            Response::LogWater { .. } => {
                return InputOutcome::Ignored("the break logs water on its water step");
            }
            Response::Start => {
                out.events
                    .push(Self::response_event(ctx, o, ReminderOutcome::Started));
                if let Some(cur) = self.s.offer.as_mut() {
                    cur.phase = OfferPhase::InProgress { since: self.s.now };
                }
                self.start_break(ctx, o.id, out);
            }
            Response::Snooze => {
                out.events
                    .push(Self::response_event(ctx, o, ReminderOutcome::Snoozed));
                let t = &mut self.s.breaks.timer;
                t.snooze_until = Some(self.s.now + ms(SNOOZE));
                t.due_since = None;
                self.finish_offer(o, ChipStatus::Snoozed, out);
            }
            Response::Skip | Response::Dismiss => {
                let (outcome, status) = if r == Response::Skip {
                    (ReminderOutcome::Skipped, ChipStatus::Skipped)
                } else {
                    (ReminderOutcome::Dismissed, ChipStatus::Dismissed)
                };
                out.events.push(Self::response_event(ctx, o, outcome));
                self.reschedule_break();
                self.finish_offer(o, status, out);
                self.count_dismissal(ctx);
            }
        }
        InputOutcome::Applied
    }

    /// Start the break's first step.
    fn start_break(&mut self, ctx: &Context, offer: Uuid, out: &mut Out) {
        let c = ctx.config.categories;
        let steps = [
            (BreakStep::Water, c.water),
            (BreakStep::Eyes, c.eye_rest),
            (BreakStep::Movement, c.movement),
            (BreakStep::Breathe, true),
        ]
        .iter()
        .filter(|(_, on)| *on)
        .fold(0, |m, (s, _)| m | bit(*s));
        self.s.breaks.seq += 1;
        let now = self.s.now;
        let session = Session {
            id: self.break_id(self.s.breaks.seq),
            offer,
            steps,
            done: 0,
            step: None,
            started: now,
            step_started: now,
            step_ms: 0,
            water_ml: None,
            opened_under: self.holds_above_pomodoro(ctx),
            done_at: None,
            outcome: RoutineOutcome::None,
        };
        self.s.breaks.session = Some(session);
        // One bubble: an open Pomodoro card closes (its phase stays for the menu).
        if let Some(p) = self.s.pomodoro.as_mut() {
            p.card_open = false;
            p.card_opened_under = HoldSet::EMPTY;
        }
        self.enter_step(ctx, session.next_step(None), out);
    }

    /// Put `step` on screen, or finish the break after the last one.
    fn enter_step(&mut self, ctx: &Context, step: Option<BreakStep>, out: &mut Out) {
        let Some(mut b) = self.s.breaks.session else {
            return;
        };
        let Some(step) = step else {
            self.finish_break(ctx, false, out);
            return;
        };
        let now = self.s.now;
        b.step = Some(step);
        b.step_started = now;
        b.step_ms = ms(match step {
            BreakStep::Water => WATER_HOLD,
            BreakStep::Eyes => EYES_HOLD,
            BreakStep::Movement => std::time::Duration::ZERO,
            BreakStep::Breathe => breathe_for(dur(now.saturating_sub(b.started))),
        });
        self.s.breaks.session = Some(b);
        out.significant = true;
        if step == BreakStep::Movement {
            self.open_break_guide(ctx, out);
        }
    }

    /// The step on screen was answered: done or skipped. On to the next step.
    pub(super) fn break_step_answered(&mut self, ctx: &Context, done: bool, out: &mut Out) {
        let Some(mut b) = self.s.breaks.session.filter(Session::running) else {
            return;
        };
        let Some(step) = b.step else {
            return;
        };
        if done {
            b.done |= bit(step);
        }
        self.s.breaks.session = Some(b);
        self.enter_step(ctx, b.next_step(Some(step)), out);
    }

    /// An answer in the break.
    pub(super) fn break_input(
        &mut self,
        ctx: &Context,
        action: BreakAction,
        out: &mut Out,
    ) -> InputOutcome {
        use InputOutcome::{Applied, Ignored};
        let Some(mut b) = self.s.breaks.session else {
            return Ignored("no break");
        };
        if action == BreakAction::Close {
            if b.running() {
                return Ignored("the break is not over");
            }
            self.s.breaks.session = None;
            out.significant = true;
            return Applied;
        }
        let Some(step) = b.step.filter(|_| b.running()) else {
            return Ignored("the break is over");
        };
        if action == BreakAction::Stop {
            self.finish_break(ctx, true, out);
            return Applied;
        }
        if step == BreakStep::Movement {
            return Ignored("the movement step answers in the movement guide");
        }
        match action {
            BreakAction::LogWater => {
                if step != BreakStep::Water || b.water_ml.is_some() {
                    return Ignored("water is logged once, on the water step");
                }
                let ml = WATER_DEFAULT_SERVING_ML;
                out.events.push(
                    NewEvent::manual(EventPayload::WaterLogged { amount_ml: ml }).at(ctx.wall),
                );
                out.extra_water_ml = ml;
                b.water_ml = Some(ml);
                b.done |= bit(BreakStep::Water);
                self.s.breaks.session = Some(b);
                out.significant = true;
            }
            BreakAction::Next => self.break_step_answered(ctx, true, out),
            // Water logged on the step stays done.
            BreakAction::Skip => self.break_step_answered(ctx, false, out),
            BreakAction::Stop | BreakAction::Close => {}
        }
        Applied
    }

    /// Record the break's result and apply it to the timers and its reminder. A stopped
    /// break closes; a finished one shows its last screen.
    fn finish_break(&mut self, ctx: &Context, stopped: bool, out: &mut Out) {
        let Some(mut b) = self.s.breaks.session.filter(Session::running) else {
            return;
        };
        if b.step == Some(BreakStep::Movement) && self.end_break_guide(ctx, out) {
            // Movements done before the stop count as the movement step done.
            b.done |= bit(BreakStep::Movement);
        }
        let outcome = b.outcome_so_far();
        out.events.push(
            NewEvent::manual(EventPayload::BreakSession {
                session_id: b.id,
                reminder_id: b.offer,
                steps: listed(b.steps),
                done: listed(b.done & b.steps),
                outcome,
                stopped,
            })
            .at(ctx.wall),
        );
        if outcome == RoutineOutcome::None {
            // Nothing done: the next break a full interval later; the movement timer
            // is not restarted.
            self.reschedule_break();
        } else {
            self.reset_break_timer();
            if b.done & bit(BreakStep::Eyes) != 0 {
                self.reset_timer(Activity::EyeRest);
            }
            if b.done & bit(BreakStep::Movement) != 0 {
                self.reset_timer(Activity::Movement);
            }
        }
        let status = match outcome {
            RoutineOutcome::Done => ChipStatus::Done,
            RoutineOutcome::Partial => ChipStatus::Partial,
            RoutineOutcome::None | RoutineOutcome::Skipped => ChipStatus::NoneRecorded,
        };
        match self
            .current_offer(b.offer)
            .filter(|o| matches!(o.phase, OfferPhase::InProgress { .. }))
        {
            Some(o) => self.finish_offer(o, status, out),
            None => {
                self.s.breaks.timer.last = Some(status);
                self.s.gap_until = self.s.now + ms(super::CALM_GAP);
            }
        }
        if outcome == RoutineOutcome::Done {
            out.notices.push(Notice::Celebrate(Activity::Movement));
        }
        if stopped {
            self.s.breaks.session = None;
        } else {
            b.step = None;
            b.done_at = Some(self.s.now);
            b.outcome = outcome;
            self.s.breaks.session = Some(b);
        }
        out.significant = true;
    }

    /// End a running break the user did not finish (the app closing, a Pomodoro started),
    /// recorded as stopped, and close its last screen.
    pub(super) fn interrupt_break(&mut self, ctx: &Context, out: &mut Out) {
        if self.break_running() {
            self.finish_break(ctx, true, out);
        }
        if self.s.breaks.session.take().is_some() {
            out.significant = true;
        }
    }

    /// The close button: a running break stops (recorded), the last screen closes.
    /// Returns false if no break is open.
    pub(super) fn close_break_card(&mut self, ctx: &Context, out: &mut Out) -> bool {
        match self.s.breaks.session {
            Some(b) if b.running() => {
                self.finish_break(ctx, true, out);
                true
            }
            Some(_) => {
                self.s.breaks.session = None;
                out.significant = true;
                true
            }
            None => false,
        }
    }

    /// A hold that was not in force when the break started, or the pet being hidden, ends
    /// a running break as stopped and closes the last screen.
    pub(super) fn settle_break(&mut self, ctx: &Context, out: &mut Out) {
        let Some(b) = self.s.breaks.session else {
            return;
        };
        let holds = self.holds_above_pomodoro(ctx);
        if holds.within(b.opened_under) && self.visible(ctx) {
            return;
        }
        self.interrupt_break(ctx, out);
    }

    /// A timed step moves on when its time on screen is up (counted as done); the break
    /// ends after 30 minutes in all; the last screen closes after a minute.
    pub(super) fn run_break_timers(&mut self, ctx: &Context, out: &mut Out) {
        let Some(b) = self.s.breaks.session else {
            return;
        };
        let now = self.s.now;
        if let Some(t) = b.done_at {
            if now.saturating_sub(t) >= ms(RESULT_SHOWN_FOR) {
                self.s.breaks.session = None;
                out.significant = true;
            }
            return;
        }
        if now.saturating_sub(b.started) >= ms(ROUTINE_TIMEOUT) {
            self.interrupt_break(ctx, out);
        } else if b.step_ms > 0 && now.saturating_sub(b.step_started) >= b.step_ms {
            self.break_step_answered(ctx, true, out);
        }
    }

    /// The break's card, except during the movement step (the movement guide shows it).
    pub(super) fn break_view(&self) -> Option<BreakView> {
        let b = self.s.breaks.session?;
        let stage = match b.step {
            None => BreakStage::Done {
                outcome: b.outcome,
                done: (b.done & b.steps).count_ones(),
                total: b.steps.count_ones(),
            },
            Some(BreakStep::Movement) => return None,
            Some(step) => BreakStage::Step {
                step,
                index: (b.steps & (bit(step) - 1)).count_ones(),
                total: b.steps.count_ones(),
                length: dur(b.step_ms),
                left: dur(b
                    .step_ms
                    .saturating_sub(self.s.now.saturating_sub(b.step_started))),
                water_ml: b.water_ml,
            },
        };
        Some(BreakView { stage })
    }

    /// The break reminder's chip status (the movement chip while combined breaks are on).
    pub(super) fn break_chip(&self, ctx: &Context) -> (ChipStatus, Option<u64>) {
        let t = &self.s.breaks.timer;
        let status = if let Some(o) = self.s.offer.filter(|o| o.combined) {
            match o.phase {
                OfferPhase::Showing { .. } => ChipStatus::Showing,
                OfferPhase::Waiting => ChipStatus::Waiting,
                OfferPhase::InProgress { .. } => ChipStatus::InProgress,
            }
        } else if t.snooze_until.is_some() {
            ChipStatus::Snoozed
        } else if self.break_due(ctx) {
            ChipStatus::Due
        } else {
            t.last.unwrap_or(ChipStatus::NotDue)
        };
        (status, t.snooze_until)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_run_in_order_and_skip_those_left_out() {
        let s = Session {
            id: Uuid::nil(),
            offer: Uuid::nil(),
            steps: bit(BreakStep::Water) | bit(BreakStep::Movement) | bit(BreakStep::Breathe),
            done: 0,
            step: None,
            started: 0,
            step_started: 0,
            step_ms: 0,
            water_ml: None,
            opened_under: HoldSet::EMPTY,
            done_at: None,
            outcome: RoutineOutcome::None,
        };
        assert_eq!(s.next_step(None), Some(BreakStep::Water));
        assert_eq!(
            s.next_step(Some(BreakStep::Water)),
            Some(BreakStep::Movement)
        );
        assert_eq!(
            s.next_step(Some(BreakStep::Movement)),
            Some(BreakStep::Breathe)
        );
        assert_eq!(s.next_step(Some(BreakStep::Breathe)), None);
        assert_eq!(s.outcome_so_far(), RoutineOutcome::None);
        let some = Session {
            done: bit(BreakStep::Water),
            ..s
        };
        assert_eq!(some.outcome_so_far(), RoutineOutcome::Partial);
        let all = Session { done: s.steps, ..s };
        assert_eq!(all.outcome_so_far(), RoutineOutcome::Done);
        assert_eq!(
            listed(s.steps),
            [BreakStep::Water, BreakStep::Movement, BreakStep::Breathe]
        );
    }
}
