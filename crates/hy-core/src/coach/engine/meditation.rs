//! The meditation pause in the engine (slice V2-4; wireframes B12 and W11, flow 10,
//! CAL-01; architecture 4.6).
//!
//! - **Invitation:** once a day at most, at the user's time
//!   ([`crate::coach::MeditationTime`]; off until a time is set), or at the first moment
//!   after it that day when a card may open. Like the weekly review offer, it opens only
//!   with no hold (not while paused, away, in focus, a presentation, a meeting, an exercise
//!   or a Pomodoro), and never outside work hours. It comes after the reminders, with
//!   their calm gap, and closes quietly after 10 minutes without an answer. It is never
//!   replayed and never carried to another day (no backlog). It offers 2, 5 (default) or 10
//!   minutes, or "Not today".
//! - **Pause:** it never starts by itself; only a length chosen on the invitation starts
//!   it. It has five short text parts of equal length, with Next, Pause or Resume, Stop,
//!   and a switch between the breath-focused and the eyes-open, non-breath versions. There
//!   is no sound. While it runs it is the one bubble, and no reminder or card appears over
//!   it (an exercise, level 3).
//! - **End:** when the last part's time is up (or Next on it), the card says "Done. Back
//!   to work whenever you are ready." and closes after a minute or on OK. Stop, the close
//!   button, any hold that starts (a lock, sleep, pause, focus, presentation, meeting, the
//!   end of work hours) and the app closing end it as stopped. Each started pause is one
//!   `meditation.session` event: the length, the version, done or stopped, and the parts
//!   reached. It stays on this PC; it is in no shared summary.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use uuid::{Builder, Uuid};

use super::guide::RESULT_SHOWN_FOR;
use super::{CALM_GAP, Context, Engine, HoldSet, InputOutcome, Out, Prompt, ms};
use crate::coach::meditation::{MEDITATION_MINUTES, MEDITATION_PARTS, MeditationVersion};
use crate::coach::types::{MeditationAction, MeditationView};
use crate::schema::{EventPayload, MeditationOutcome, NewEvent};

/// The meditation state (saved with the engine state).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Meditation {
    /// The local date the invitation was last shown; it is not shown again that day.
    invited_on: Option<NaiveDate>,
    /// Pauses started (with the engine's salt, makes each pause's id unique).
    seq: u64,
    /// The pause, running or on its last screen.
    session: Option<Session>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Session {
    id: Uuid,
    minutes: u32,
    version: MeditationVersion,
    /// The part on screen, from 0.
    part: u32,
    /// Engine time the part began (moved on by the time spent paused).
    part_started: u64,
    /// Engine time the user paused it.
    paused_at: Option<u64>,
    /// Engine time the last screen appeared.
    done_at: Option<u64>,
    /// The holds in force when it started; any other hold ends it.
    opened_under: HoldSet,
}

impl Session {
    /// One part's length, ms.
    const fn part_ms(&self) -> u64 {
        self.minutes as u64 * 60_000 / MEDITATION_PARTS as u64
    }

    const fn running(&self) -> bool {
        self.done_at.is_none()
    }

    fn elapsed_in_part(&self, now: u64) -> u64 {
        self.paused_at
            .unwrap_or(now)
            .saturating_sub(self.part_started)
    }
}

impl Engine {
    /// The id of the `seq`-th pause. It cannot equal a reminder or routine id: those use
    /// the salt itself, or the salt mixed with another constant, in their first half.
    fn meditation_id(&self, seq: u64) -> Uuid {
        let mut bytes = [0u8; 16];
        bytes[..8].copy_from_slice(&(self.s.salt ^ 0x5bd1_e995_c2b2_ae35).to_be_bytes());
        bytes[8..].copy_from_slice(&seq.to_be_bytes());
        Builder::from_random_bytes(bytes).into_uuid()
    }

    /// A pause is running (not on its last screen): an exercise, level 3.
    pub(super) fn meditation_running(&self) -> bool {
        self.s.meditation.session.is_some_and(|m| m.running())
    }

    /// A pause (running or on its last screen) holds the bubble.
    pub(super) const fn meditation_open(&self) -> bool {
        self.s.meditation.session.is_some()
    }

    /// Open the invitation if it is due. Returns true if it opened.
    pub(super) fn maybe_meditation_invite(&mut self, ctx: &Context, out: &mut Out) -> bool {
        let Some(at) = ctx.config.meditation.at else {
            return false;
        };
        if self.prompt_blocked(ctx) || self.off_hours(ctx) {
            return false;
        }
        let today = ctx.zone.local_date(ctx.wall);
        if self.s.meditation.invited_on == Some(today) {
            return false;
        }
        if ctx.wall < ctx.zone.resolve_local(today.and_time(at)) {
            return false;
        }
        self.s.meditation.invited_on = Some(today);
        self.set_prompt(Prompt::MeditationInvite, false, ctx);
        out.significant = true;
        true
    }

    /// The pause's card.
    pub(super) fn meditation_view(&self) -> Option<MeditationView> {
        let m = self.s.meditation.session?;
        let now = self.s.now;
        Some(MeditationView {
            minutes: m.minutes,
            version: m.version,
            part: m.part,
            paused: m.paused_at.is_some(),
            part_left: super::dur(m.part_ms().saturating_sub(m.elapsed_in_part(now))),
            done: !m.running(),
        })
    }

    /// An answer on the invitation or in the pause.
    pub(super) fn meditation_input(
        &mut self,
        ctx: &Context,
        action: MeditationAction,
        out: &mut Out,
    ) -> InputOutcome {
        use InputOutcome::{Applied, Ignored, Invalid};
        let now = self.s.now;
        match action {
            MeditationAction::Start(_) | MeditationAction::NotToday
                if self.s.prompt != Some(Prompt::MeditationInvite) =>
            {
                Ignored("no meditation invitation open")
            }
            MeditationAction::Start(minutes) => {
                if !MEDITATION_MINUTES.contains(&minutes) {
                    return Invalid("choose 2, 5 or 10 minutes");
                }
                // Revalidate against the holds now in force (work may have ended, or a
                // meeting begun, since the invitation opened): a pause starts only when
                // the invitation could open.
                if self.prompt_blocked(ctx) || self.off_hours(ctx) || !self.visible(ctx) {
                    self.clear_prompt();
                    out.significant = true;
                    return Ignored("the meditation invitation is no longer open");
                }
                self.clear_prompt();
                self.s.meditation.seq += 1;
                let id = self.meditation_id(self.s.meditation.seq);
                self.s.meditation.session = Some(Session {
                    id,
                    minutes,
                    version: MeditationVersion::Breath,
                    part: 0,
                    part_started: now,
                    paused_at: None,
                    done_at: None,
                    opened_under: self.holds_above_pomodoro(ctx),
                });
                out.significant = true;
                Applied
            }
            MeditationAction::NotToday => {
                // Skipped for today; nothing is owed and nothing is recorded.
                self.clear_prompt();
                self.s.gap_until = now + ms(CALM_GAP);
                out.significant = true;
                Applied
            }
            MeditationAction::Close => match self.s.meditation.session {
                Some(m) if !m.running() => {
                    self.s.meditation.session = None;
                    out.significant = true;
                    Applied
                }
                _ => Ignored("the pause is not over"),
            },
            _ => {
                let Some(mut m) = self.s.meditation.session.filter(Session::running) else {
                    return Ignored("no meditation pause running");
                };
                match action {
                    MeditationAction::Next => {
                        if m.part + 1 >= MEDITATION_PARTS {
                            self.s.meditation.session = Some(m);
                            self.finish_meditation(ctx, MeditationOutcome::Done, out);
                            return Applied;
                        }
                        m.part += 1;
                        m.part_started = now;
                        m.paused_at = None;
                    }
                    MeditationAction::Pause => {
                        if m.paused_at.is_some() {
                            return Ignored("already paused");
                        }
                        m.paused_at = Some(now);
                    }
                    MeditationAction::Resume => {
                        let Some(at) = m.paused_at else {
                            return Ignored("not paused");
                        };
                        m.part_started += now.saturating_sub(at);
                        m.paused_at = None;
                    }
                    MeditationAction::Switch => {
                        m.version = match m.version {
                            MeditationVersion::Breath => MeditationVersion::EyesOpen,
                            MeditationVersion::EyesOpen => MeditationVersion::Breath,
                        };
                    }
                    MeditationAction::Stop => {
                        self.finish_meditation(ctx, MeditationOutcome::Stopped, out);
                        return Applied;
                    }
                    MeditationAction::Start(_)
                    | MeditationAction::NotToday
                    | MeditationAction::Close => return Ignored("not in the pause"),
                }
                self.s.meditation.session = Some(m);
                out.significant = true;
                Applied
            }
        }
    }

    /// Record the pause's result. Done shows the last screen; stopped closes it.
    fn finish_meditation(&mut self, ctx: &Context, outcome: MeditationOutcome, out: &mut Out) {
        let Some(mut m) = self.s.meditation.session.filter(Session::running) else {
            return;
        };
        let parts_reached = if outcome == MeditationOutcome::Done {
            MEDITATION_PARTS
        } else {
            (m.part + 1).min(MEDITATION_PARTS)
        };
        out.events.push(
            NewEvent::manual(EventPayload::MeditationSession {
                session_id: m.id,
                minutes: m.minutes,
                version: m.version,
                outcome,
                parts_reached,
            })
            .at(ctx.wall),
        );
        if outcome == MeditationOutcome::Done {
            m.done_at = Some(self.s.now);
            m.paused_at = None;
            m.part = MEDITATION_PARTS - 1;
            self.s.meditation.session = Some(m);
        } else {
            self.s.meditation.session = None;
        }
        self.s.gap_until = self.s.now + ms(CALM_GAP);
        out.significant = true;
    }

    /// The close button: a running pause stops (recorded), the last screen closes.
    /// Returns false if no pause card is open.
    pub(super) fn close_meditation_card(&mut self, ctx: &Context, out: &mut Out) -> bool {
        match self.s.meditation.session {
            Some(m) if m.running() => {
                self.finish_meditation(ctx, MeditationOutcome::Stopped, out);
                true
            }
            Some(_) => {
                self.s.meditation.session = None;
                out.significant = true;
                true
            }
            None => false,
        }
    }

    /// A hold that was not in force when the pause started, or the pet being hidden, ends
    /// a running pause as stopped and closes the last screen.
    pub(super) fn settle_meditation(&mut self, ctx: &Context, out: &mut Out) {
        let Some(m) = self.s.meditation.session else {
            return;
        };
        let holds = self.holds_above_pomodoro(ctx);
        if holds.within(m.opened_under) && self.visible(ctx) {
            return;
        }
        if m.running() {
            self.finish_meditation(ctx, MeditationOutcome::Stopped, out);
        } else {
            self.s.meditation.session = None;
            out.significant = true;
        }
    }

    /// The app closed while a pause ran: it ended then, as stopped.
    pub(super) fn interrupt_meditation(&mut self, ctx: &Context, out: &mut Out) {
        if self.meditation_running() {
            self.finish_meditation(ctx, MeditationOutcome::Stopped, out);
        }
        if self.s.meditation.session.take().is_some() {
            out.significant = true;
        }
    }

    /// The parts' time: each part moves on when its time is up; the last one ends the
    /// pause as done. The last screen closes after a minute.
    pub(super) fn run_meditation_timers(&mut self, ctx: &Context, out: &mut Out) {
        let Some(mut m) = self.s.meditation.session else {
            return;
        };
        let now = self.s.now;
        if !m.running() {
            if m.done_at
                .is_some_and(|t| now.saturating_sub(t) >= ms(RESULT_SHOWN_FOR))
            {
                self.s.meditation.session = None;
                out.significant = true;
            }
            return;
        }
        if m.paused_at.is_some() {
            return;
        }
        let part = m.part_ms().max(1);
        let mut moved = false;
        while m.elapsed_in_part(now) >= part {
            if m.part + 1 >= MEDITATION_PARTS {
                self.s.meditation.session = Some(m);
                self.finish_meditation(ctx, MeditationOutcome::Done, out);
                return;
            }
            m.part += 1;
            m.part_started += part;
            moved = true;
        }
        if moved {
            self.s.meditation.session = Some(m);
            out.significant = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parts_divide_the_length_evenly() {
        let s = |minutes| Session {
            id: Uuid::nil(),
            minutes,
            version: MeditationVersion::Breath,
            part: 0,
            part_started: 1000,
            paused_at: None,
            done_at: None,
            opened_under: HoldSet::EMPTY,
        };
        assert_eq!(s(2).part_ms(), 24_000);
        assert_eq!(s(5).part_ms(), 60_000);
        assert_eq!(s(10).part_ms(), 120_000);
        let mut p = s(5);
        assert_eq!(p.elapsed_in_part(31_000), 30_000);
        p.paused_at = Some(11_000);
        assert_eq!(p.elapsed_in_part(500_000), 10_000);
    }
}
