//! The posture cue in the engine (slice V2-2; architecture 4.5 "Episode and cue", PRD V2
//! gate).
//!
//! The posture detector asks for a cue once per drift episode ([`Input::PostureCue`]).
//! The engine decides, at once, whether the pet shows it:
//!
//! - **Focus** drops it (never queued): [`CUE_FOCUS`].
//! - **Holds** (paused, away, off hours, a presentation, a meeting) or the pet hidden
//!   suppress it: [`CUE_HELD`]. The camera is released during most of these anyway.
//! - **Another card**, an exercise, a Pomodoro or the calm gap after a card suppress it:
//!   [`CUE_BUSY`]. Nothing is replayed later.
//! - **Cooldown**: at most one cue per [`POSTURE_CUE_COOLDOWN`] (PRD: 30 minutes by
//!   default): [`CUE_COOLDOWN`].
//!
//! Otherwise the card opens: one line, OK and Recalibrate. Like the recovery cards that
//! open on their own it closes quietly after 10 minutes and any hold withdraws it.
//!
//! [`Input::PostureCue`]: crate::coach::Input::PostureCue

use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::{Context, Engine, InputOutcome, Out, Prompt, ms};

/// At most one posture cue this often (PRD V2 gate: a 30-minute cooldown by default).
pub const POSTURE_CUE_COOLDOWN: Duration = Duration::from_secs(30 * 60);
/// The cue was dropped: quiet focus is on.
pub const CUE_FOCUS: &str = "posture cue dropped: focus";
/// The cue was suppressed: a hold, or the pet is hidden.
pub const CUE_HELD: &str = "posture cue suppressed: hold";
/// The cue was suppressed: another card, an exercise, a Pomodoro or the calm gap.
pub const CUE_BUSY: &str = "posture cue suppressed: busy";
/// The cue was suppressed: within the cooldown.
pub const CUE_COOLDOWN: &str = "posture cue suppressed: cooldown";

/// The posture cue's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PostureCue {
    /// Engine time the last cue was shown.
    last_shown: Option<u64>,
}

impl Engine {
    /// The detector asks for a cue.
    pub(super) fn posture_cue_input(&mut self, ctx: &Context, out: &mut Out) -> InputOutcome {
        let now = self.s.now;
        if self.s.focus_until.is_some() {
            return InputOutcome::Ignored(CUE_FOCUS);
        }
        if self.level1(ctx) || self.level2(ctx) || !self.visible(ctx) {
            return InputOutcome::Ignored(CUE_HELD);
        }
        if self
            .s
            .posture
            .last_shown
            .is_some_and(|t| now.saturating_sub(t) < ms(POSTURE_CUE_COOLDOWN))
        {
            return InputOutcome::Ignored(CUE_COOLDOWN);
        }
        let slot_free = self.s.offer.is_none()
            && self.s.prompt.is_none()
            && self.s.guide.is_none()
            && self.s.note.is_none()
            && !self.s.pomodoro.is_some_and(|p| p.card_open);
        let calm = now >= self.s.gap_until && now >= self.s.grace_until;
        if self.level3() || !slot_free || !calm {
            return InputOutcome::Ignored(CUE_BUSY);
        }
        self.s.posture.last_shown = Some(now);
        self.set_prompt(Prompt::PostureCue, false, ctx);
        out.significant = true;
        InputOutcome::Applied
    }
}
