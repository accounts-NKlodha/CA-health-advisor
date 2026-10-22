//! The deadline day and the recovery-day plan in the engine (slice V1-3; wireframe B7,
//! flow 4). See [`crate::coach::recovery`] for the rules.
//!
//! Every card here uses the single bubble, after the reminders and with their calm gap.
//! Cards the user asked for ("Deadline day" on the pet menu, and the cards that follow an
//! answer) stay under the holds in force when they opened; cards that open on their own
//! (the recovery day's morning card, the walk suggestion and the end-of-day question)
//! open only with no hold (the question also after work hours, like the check-in) and
//! close quietly after 10 minutes without an answer, recording nothing.

use std::time::Duration;

use chrono::{NaiveDate, TimeDelta};

use super::{CALM_GAP, Context, Engine, InputOutcome, Out, Prompt, ms, ms_between};
use crate::coach::config::Intensity;
use crate::coach::recovery::{
    DATE_CHOICES, PLAN_AHEAD_DAYS, RecoveryAction, RecoveryPlan, RecoveryView, Reflection,
    WALK_REPEAT_EVERY, WALK_WINDOW,
};
use crate::coach::types::{Bubble, CheckInAnswer, Notice};
use crate::schema::{
    Activity, CheckInOrigin, CheckInReply, EventPayload, NewEvent, PlanChange, ReflectionAnswer,
};

impl Engine {
    fn local_today(ctx: &Context) -> NaiveDate {
        ctx.zone.local_date(ctx.wall)
    }

    /// The pending plan (its date is today or later).
    fn pending_plan(&self, ctx: &Context) -> Option<RecoveryPlan> {
        let today = Self::local_today(ctx);
        self.s.plan.filter(|p| p.date >= today)
    }

    /// Today's plan, on the recovery day itself.
    fn todays_plan(&self, ctx: &Context) -> Option<RecoveryPlan> {
        let today = Self::local_today(ctx);
        self.s.plan.filter(|p| p.date == today)
    }

    /// A card the user asks for from the pet menu can open now: the pet is shown and the
    /// bubble is free (no reminder, routine or other card).
    fn can_open(&self, ctx: &Context) -> bool {
        self.visible(ctx)
            && self.s.away.is_none()
            && self.s.offer.is_none()
            && self.s.guide.is_none()
            && self.s.prompt.is_none()
    }

    /// The interval for movement and eye rest is one step more frequent on a recovery day
    /// with "short breaks a little more often" (Light becomes Balanced, Balanced becomes
    /// More often). Water stays at the user's own pace and goal.
    pub(super) fn every(&self, ctx: &Context, a: Activity) -> Duration {
        let more = a != Activity::Water && self.todays_plan(ctx).is_some_and(|p| p.more_breaks);
        let intensity = match (more, ctx.config.intensity) {
            (false, i) => i,
            (true, Intensity::Light) => Intensity::Balanced,
            (true, _) => Intensity::MoreOften,
        };
        match a {
            Activity::Movement => intensity.movement_every(),
            Activity::EyeRest => intensity.eye_rest_every(),
            Activity::Water => ctx.config.every(a),
        }
    }

    /// The next working days after today (up to five), for the invitation and the date
    /// choices. With no workdays set, the next calendar days.
    fn next_workdays(ctx: &Context) -> Vec<NaiveDate> {
        let today = Self::local_today(ctx);
        let days =
            (1..=PLAN_AHEAD_DAYS).filter_map(|d| today.checked_add_signed(TimeDelta::days(d)));
        let work: Vec<_> = days
            .clone()
            .filter(|d| ctx.schedule.window_for(*d, &ctx.zone).is_some())
            .take(DATE_CHOICES + 1)
            .collect();
        if work.is_empty() {
            days.take(DATE_CHOICES + 1).collect()
        } else {
            work
        }
    }

    /// After a "Yes": the planned day if one is pending, else the invitation.
    fn open_recovery_follow_up(&mut self, ctx: &Context) {
        let today = Self::local_today(ctx);
        let p = if self.s.plan.is_some_and(|p| p.date > today) {
            Prompt::RecoveryPlanned
        } else {
            Prompt::RecoveryInvite
        };
        self.set_prompt(p, true, ctx);
    }

    /// "Deadline day" on the pet menu.
    pub(super) fn deadline_day(&mut self, ctx: &Context, out: &mut Out) -> InputOutcome {
        if !self.can_open(ctx) {
            return InputOutcome::Ignored("another card is open");
        }
        let today = Self::local_today(ctx);
        if self.s.days.get(&today).is_some_and(|d| d.deadline) {
            self.open_recovery_follow_up(ctx);
        } else {
            self.set_prompt(Prompt::DeadlineDay, true, ctx);
        }
        out.significant = true;
        InputOutcome::Applied
    }

    /// An answer to the long-day check-in or to "Deadline day?".
    pub(super) fn check_in(
        &mut self,
        ctx: &Context,
        answer: CheckInAnswer,
        out: &mut Out,
    ) -> InputOutcome {
        let origin = match self.s.prompt {
            Some(Prompt::CheckIn) => CheckInOrigin::LongDay,
            Some(Prompt::DeadlineDay) => CheckInOrigin::Menu,
            _ => return InputOutcome::Ignored("no check-in open"),
        };
        let reply = match answer {
            CheckInAnswer::Yes => CheckInReply::Yes,
            CheckInAnswer::No => CheckInReply::No,
            CheckInAnswer::DontAskToday => CheckInReply::DontAskToday,
        };
        out.events.push(
            NewEvent::manual(EventPayload::RecoveryCheckIn {
                origin,
                answer: reply,
            })
            .at(ctx.wall),
        );
        self.clear_prompt();
        self.s.gap_until = self.s.now + ms(CALM_GAP);
        match answer {
            CheckInAnswer::Yes => {
                let day = self.today_state(ctx);
                day.deadline = true;
                day.check_in_done = true;
                out.notices
                    .push(Notice::DeadlineDayConfirmed(Self::local_today(ctx)));
                self.open_recovery_follow_up(ctx);
            }
            CheckInAnswer::DontAskToday => self.today_state(ctx).check_in_done = true,
            CheckInAnswer::No => {}
        }
        InputOutcome::Applied
    }

    fn plan_event(ctx: &Context, plan_date: NaiveDate, change: PlanChange) -> NewEvent {
        NewEvent::manual(EventPayload::RecoveryPlanChanged { plan_date, change }).at(ctx.wall)
    }

    /// Keep `plan` as the one pending plan (an earlier plan for another date is cancelled).
    fn save_plan(&mut self, ctx: &Context, plan: RecoveryPlan, out: &mut Out) {
        if let Some(old) = self.pending_plan(ctx)
            && old.date != plan.date
        {
            out.events
                .push(Self::plan_event(ctx, old.date, PlanChange::Cancelled));
        }
        out.events
            .push(Self::plan_event(ctx, plan.date, PlanChange::Saved));
        self.s.plan = Some(plan);
    }

    /// Close the open card and start the calm gap.
    fn close_card(&mut self) {
        self.clear_prompt();
        self.s.gap_until = self.s.now + ms(CALM_GAP);
    }

    /// An answer on a recovery card, or a plan saved from the plan screen.
    pub(super) fn recovery_input(
        &mut self,
        ctx: &Context,
        action: RecoveryAction,
        out: &mut Out,
    ) -> InputOutcome {
        use InputOutcome::{Applied, Ignored, Invalid};
        let today = Self::local_today(ctx);
        let prompt = self.s.prompt;
        let on = |ps: &[Prompt]| prompt.is_some_and(|p| ps.contains(&p));
        match action {
            RecoveryAction::PlanFor(date) => {
                if !on(&[Prompt::RecoveryInvite, Prompt::RecoveryDates]) {
                    return Ignored("no recovery invitation open");
                }
                let plan = RecoveryPlan::default_for(date, &ctx.schedule, &ctx.zone);
                if plan.validate(today).is_err() {
                    return Invalid("a recovery day is planned 1 to 30 days ahead");
                }
                self.save_plan(ctx, plan, out);
                self.set_prompt(Prompt::RecoveryPlanned, true, ctx);
            }
            RecoveryAction::ChooseDate => {
                if !on(&[Prompt::RecoveryInvite, Prompt::RecoveryPlanned]) {
                    return Ignored("no recovery invitation open");
                }
                self.set_prompt(Prompt::RecoveryDates, true, ctx);
            }
            RecoveryAction::Later | RecoveryAction::NoThanks => {
                if !on(&[Prompt::RecoveryInvite, Prompt::RecoveryDates]) {
                    return Ignored("no recovery invitation open");
                }
                self.close_card();
            }
            RecoveryAction::Ok => {
                if !on(&[Prompt::RecoveryPlanned, Prompt::RecoveryMorning]) {
                    return Ignored("no recovery card open");
                }
                self.close_card();
            }
            RecoveryAction::SavePlan(plan) => {
                if plan.validate(today).is_err() {
                    return Invalid(
                        "a recovery day is planned 1 to 30 days ahead; a repeating walk needs a time",
                    );
                }
                self.save_plan(ctx, plan, out);
            }
            RecoveryAction::CancelPlan => {
                let Some(plan) = self.pending_plan(ctx) else {
                    return Ignored("no recovery day planned");
                };
                out.events
                    .push(Self::plan_event(ctx, plan.date, PlanChange::Cancelled));
                self.s.plan = None;
                if on(&[
                    Prompt::RecoveryPlanned,
                    Prompt::RecoveryMorning,
                    Prompt::RecoveryWalk,
                ]) {
                    self.close_card();
                }
            }
            RecoveryAction::WalkNow | RecoveryAction::WalkNotNow => {
                if !on(&[Prompt::RecoveryWalk]) {
                    return Ignored("no walk suggestion open");
                }
                self.close_card();
            }
            RecoveryAction::Reflect(r) => {
                if !on(&[Prompt::RecoveryReflection]) {
                    return Ignored("no question open");
                }
                let answer = match r {
                    Reflection::Yes => Some(ReflectionAnswer::Yes),
                    Reflection::Somewhat => Some(ReflectionAnswer::Somewhat),
                    Reflection::No => Some(ReflectionAnswer::No),
                    Reflection::Skip => None,
                };
                if let Some(answer) = answer {
                    out.events.push(
                        NewEvent::manual(EventPayload::RecoveryReflection { answer }).at(ctx.wall),
                    );
                }
                self.close_card();
            }
            RecoveryAction::ShowPlan => {
                let Some(plan) = self.pending_plan(ctx) else {
                    return Ignored("no recovery day planned");
                };
                if !self.can_open(ctx) {
                    return Ignored("another card is open");
                }
                let p = if plan.date == today {
                    Prompt::RecoveryMorning
                } else {
                    Prompt::RecoveryPlanned
                };
                self.set_prompt(p, true, ctx);
            }
        }
        out.significant = true;
        Applied
    }

    /// On the recovery day, with no hold: the morning card once, then the walk suggestion
    /// at its time (and every 90 minutes if chosen). A walk time that passes while held
    /// or busy is used up, never replayed. Returns true if a card opened.
    pub(super) fn maybe_recovery_card(&mut self, ctx: &Context, out: &mut Out) -> bool {
        let Some(plan) = self.todays_plan(ctx) else {
            return false;
        };
        let day = self.today_state(ctx);
        if !day.recovery_morning {
            day.recovery_morning = true;
            self.set_prompt(Prompt::RecoveryMorning, false, ctx);
            out.significant = true;
            return true;
        }
        let Some(walk) = plan.walk_at else {
            return false;
        };
        let used = day.walk_slots;
        let first = ctx.zone.resolve_local(plan.date.and_time(walk));
        if ctx.wall < first {
            return false;
        }
        let since_first = ms_between(first, ctx.wall);
        let slot = if plan.walk_repeat {
            u32::try_from(since_first / ms(WALK_REPEAT_EVERY)).unwrap_or(u32::MAX)
        } else {
            0
        };
        if slot < used {
            return false;
        }
        // The suggestion stops at the planned finish time (or the end of work hours).
        let end = plan
            .finish_at
            .map(|t| ctx.zone.resolve_local(plan.date.and_time(t)))
            .or_else(|| ctx.schedule.window_for(plan.date, &ctx.zone).map(|w| w.end));
        let slot_at = first
            + TimeDelta::milliseconds(
                i64::try_from(u64::from(slot) * ms(WALK_REPEAT_EVERY)).unwrap_or(i64::MAX),
            );
        if end.is_some_and(|e| slot_at >= e) {
            return false;
        }
        self.today_state(ctx).walk_slots = slot.saturating_add(1);
        out.significant = true;
        if ms_between(slot_at, ctx.wall) < ms(WALK_WINDOW) {
            self.set_prompt(Prompt::RecoveryWalk, false, ctx);
            return true;
        }
        false
    }

    /// On the recovery day, once, after the planned finish time (or the end of work
    /// hours): "Did this routine feel helpful?". It may come after work hours, like the
    /// check-in. Returns true if it opened.
    pub(super) fn maybe_reflection(&mut self, ctx: &Context, out: &mut Out) -> bool {
        let Some(plan) = self.todays_plan(ctx).filter(|p| p.reflection) else {
            return false;
        };
        if self.prompt_blocked(ctx) {
            return false;
        }
        let finish = plan
            .finish_at
            .map(|t| ctx.zone.resolve_local(plan.date.and_time(t)))
            .or_else(|| ctx.schedule.window_for(plan.date, &ctx.zone).map(|w| w.end));
        if finish.is_none_or(|f| ctx.wall < f) {
            return false;
        }
        let day = self.today_state(ctx);
        if day.reflection_asked {
            return false;
        }
        day.reflection_asked = true;
        self.set_prompt(Prompt::RecoveryReflection, false, ctx);
        out.significant = true;
        true
    }

    /// The bubble for a recovery card (`None` if it no longer applies).
    pub(super) fn recovery_bubble(&self, ctx: &Context, p: Prompt) -> Option<Bubble> {
        Some(match p {
            Prompt::CheckIn | Prompt::Lighter => return None,
            Prompt::DeadlineDay => Bubble::DeadlineDay,
            Prompt::RecoveryInvite => Bubble::RecoveryInvite {
                next: *Self::next_workdays(ctx).first()?,
            },
            Prompt::RecoveryDates => {
                let next = Self::next_workdays(ctx);
                let mut dates = [None; DATE_CHOICES];
                for (slot, d) in dates.iter_mut().zip(next.iter().skip(1)) {
                    *slot = Some(*d);
                }
                Bubble::RecoveryDates { dates }
            }
            Prompt::RecoveryPlanned => Bubble::RecoveryPlanned {
                plan: self.pending_plan(ctx)?,
            },
            Prompt::RecoveryMorning => Bubble::RecoveryMorning {
                plan: self.todays_plan(ctx)?,
                water_goal_ml: ctx.water.goal_ml,
            },
            Prompt::RecoveryWalk => Bubble::RecoveryWalk,
            Prompt::RecoveryReflection => Bubble::RecoveryReflection,
        })
    }

    /// The recovery state for the pet menu.
    pub(super) fn recovery_view(&self, ctx: &Context) -> RecoveryView {
        let today = Self::local_today(ctx);
        RecoveryView {
            today,
            marked_today: self.s.days.get(&today).is_some_and(|d| d.deadline),
            plan: self.pending_plan(ctx),
            recovery_day: self.todays_plan(ctx).is_some(),
            can_open: self.can_open(ctx),
        }
    }
}
