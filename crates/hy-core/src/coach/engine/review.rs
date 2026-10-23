//! The weekly review offer in the engine (slice V1-4; wireframe B9, flow 11).
//!
//! - Once a week, at the user's review time ([`crate::coach::ReviewTime`], Saturday 17:00
//!   by default) or at the first moment after it in the same week when a card may open,
//!   the bubble offers the week's review: Open, Later or Skip this week. It is offered
//!   only for a week in which this PC recorded some activity.
//! - "Later" offers it once more on the next workday (any time that day, when a card may
//!   open); a day that passes without that chance drops it. "Skip this week" and "Open"
//!   end it for the week. Opening the review window is the shell's part.
//! - Like the recovery cards that open on their own, it opens only with no hold, after
//!   the reminders and with their calm gap, closes quietly after 10 minutes without an
//!   answer, and is never replayed.

use chrono::{Datelike, NaiveDate, TimeDelta};
use serde::{Deserialize, Serialize};

use super::{CALM_GAP, Context, Engine, InputOutcome, Out, Prompt, ms};
use crate::coach::types::{Bubble, ReviewAnswer};

/// The weekly review offer's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReviewOffer {
    /// The week (its Monday) whose offer was last shown; it is not offered again.
    handled: Option<NaiveDate>,
    /// "Later": the week, and the date it is offered once more.
    later: Option<(NaiveDate, NaiveDate)>,
    /// The week of the offer on show, and whether it is the second (Later) offer.
    showing: Option<(NaiveDate, bool)>,
}

/// The Monday of the week containing `date`.
fn monday(date: NaiveDate) -> NaiveDate {
    date - TimeDelta::days(i64::from(date.weekday().num_days_from_monday()))
}

impl Engine {
    /// Open the weekly review offer if it is due. Returns true if it opened.
    pub(super) fn maybe_review_offer(&mut self, ctx: &Context, out: &mut Out) -> bool {
        if self.prompt_blocked(ctx) {
            return false;
        }
        let today = ctx.zone.local_date(ctx.wall);
        let week = monday(today);
        let r = &mut self.s.review;
        if let Some((later_week, on)) = r.later {
            if today > on {
                r.later = None;
            } else if today == on {
                r.later = None;
                r.showing = Some((later_week, true));
                self.set_prompt(Prompt::ReviewOffer, false, ctx);
                out.significant = true;
                return true;
            }
        }
        let cfg = ctx.config.review;
        let day = week + TimeDelta::days(i64::from(cfg.day.num_days_from_monday()));
        let due = ctx.zone.resolve_local(day.and_time(cfg.at));
        if r.handled == Some(week) || ctx.wall < due || !self.recorded_since(week) {
            return false;
        }
        let r = &mut self.s.review;
        r.handled = Some(week);
        r.showing = Some((week, false));
        self.set_prompt(Prompt::ReviewOffer, false, ctx);
        out.significant = true;
        true
    }

    /// Whether the state's day counters show PC activity on or after `from`.
    fn recorded_since(&self, from: NaiveDate) -> bool {
        self.s
            .days
            .range(from..)
            .any(|(_, d)| d.tally.pc_active_ms > 0)
    }

    /// The offer's card, while it is on show.
    pub(super) fn review_bubble(&self) -> Option<Bubble> {
        let (week, second) = self.s.review.showing?;
        Some(Bubble::ReviewOffer {
            week,
            later: !second,
        })
    }

    /// The offer closed without an answer (or was withdrawn).
    pub(super) fn review_closed(&mut self) {
        self.s.review.showing = None;
    }

    /// An answer on the offer.
    pub(super) fn review_input(
        &mut self,
        ctx: &Context,
        answer: ReviewAnswer,
        out: &mut Out,
    ) -> InputOutcome {
        if self.s.prompt != Some(Prompt::ReviewOffer) {
            return InputOutcome::Ignored("no review offer open");
        }
        let Some((week, second)) = self.s.review.showing else {
            return InputOutcome::Ignored("no review offer open");
        };
        if answer == ReviewAnswer::Later {
            if second {
                return InputOutcome::Ignored("Later was already used for this week");
            }
            let today = ctx.zone.local_date(ctx.wall);
            let next = (1..=14)
                .filter_map(|d| today.checked_add_signed(TimeDelta::days(d)))
                .find(|d| ctx.schedule.window_for(*d, &ctx.zone).is_some());
            self.s.review.later = next.map(|on| (week, on));
        }
        self.clear_prompt();
        self.s.gap_until = self.s.now + ms(CALM_GAP);
        out.significant = true;
        InputOutcome::Applied
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monday_of_any_day() {
        let d = |n| NaiveDate::from_ymd_opt(2026, 10, n).unwrap();
        assert_eq!(monday(d(19)), d(19));
        assert_eq!(monday(d(24)), d(19));
        assert_eq!(monday(d(25)), d(19));
        assert_eq!(monday(d(26)), d(26));
    }
}
