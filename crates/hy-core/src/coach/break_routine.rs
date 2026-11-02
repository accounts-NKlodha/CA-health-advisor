//! The combined 5-minute break's steps and words (DEC15).
//!
//! The owner asked on 2 Nov 2026 for one break every 30 minutes in place of separate
//! reminders: drink water (about 10 seconds), an eye exercise (10 to 15 seconds), the
//! movement routine, then breathing with the eyes closed for the rest of the 5 minutes.
//! The movement step is the bundled routine ([`super::movement`]); the breathing step
//! reuses the meditation pause's breath-focused opening ([`super::meditation`]). Both are
//! **provisional content, pending the planned content review**, so every step's card says
//! so.

use std::time::Duration;

use super::meditation::{self, MeditationVersion};
pub use crate::schema::BreakStep;

/// The whole break aims at this length; the breathing step takes what is left (D).
pub const BREAK_LENGTH: Duration = Duration::from_secs(5 * 60);
/// The water step's time on screen (the owner: hold for 10 seconds).
pub const WATER_HOLD: Duration = Duration::from_secs(10);
/// The eye step's time on screen (the owner: 10 to 15 seconds).
pub const EYES_HOLD: Duration = Duration::from_secs(15);
/// The breathing step is at least this long, whatever is left of the 5 minutes (D).
pub const BREATHE_MIN: Duration = Duration::from_secs(60);
/// The breathing step is at most this long (D).
pub const BREATHE_MAX: Duration = Duration::from_secs(90);

/// The reminder's words.
pub const OFFER_TITLE: &str = "Time for a 5-minute break";
/// The reminder's line after the time at the PC.
pub const OFFER_LINE: &str = "Water, an eye exercise, a short movement routine and slow breathing with your eyes closed. Every step can be skipped.";

/// The last screen.
pub const DONE_TITLE: &str = "Break done. Back to work whenever you are ready.";

/// The breathing step's length: what is left of [`BREAK_LENGTH`] after `elapsed`, kept
/// between [`BREATHE_MIN`] and [`BREATHE_MAX`].
#[must_use]
pub fn breathe_for(elapsed: Duration) -> Duration {
    BREAK_LENGTH
        .saturating_sub(elapsed)
        .clamp(BREATHE_MIN, BREATHE_MAX)
}

/// A step's heading.
#[must_use]
pub const fn title(step: BreakStep) -> &'static str {
    match step {
        BreakStep::Water => "Drink some water",
        BreakStep::Eyes => "Rest your eyes",
        BreakStep::Movement => "Move a little",
        BreakStep::Breathe => "Breathe slowly with your eyes closed",
    }
}

/// A step's lines (the movement step's are the routine's own, in the guide).
#[must_use]
pub fn lines(step: BreakStep) -> Vec<&'static str> {
    match step {
        BreakStep::Water => vec!["Take a few sips."],
        BreakStep::Eyes => vec![
            "Look at something far away, out of a window or across the room.",
            "Then move your eyes slowly: up and down, side to side. Blink a few times.",
        ],
        BreakStep::Movement => vec!["Today's movement routine, seated or standing."],
        BreakStep::Breathe => {
            // The meditation pause's breath-focused opening: settle, eyes closed, notice
            // the breath.
            let p = meditation::parts(MeditationVersion::Breath);
            let mut v: Vec<&'static str> = p[0].lines.to_vec();
            v.extend_from_slice(p[1].lines);
            v.push("Let your breathing slow down if that feels comfortable.");
            v
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_breathing_step_fills_the_five_minutes_within_bounds() {
        assert_eq!(breathe_for(Duration::ZERO), BREATHE_MAX);
        assert_eq!(
            breathe_for(Duration::from_secs(225)),
            Duration::from_secs(75)
        );
        assert_eq!(breathe_for(Duration::from_secs(280)), BREATHE_MIN);
        assert_eq!(breathe_for(Duration::from_secs(900)), BREATHE_MIN);
    }

    #[test]
    fn every_step_has_short_words() {
        for s in BreakStep::ALL {
            assert!(!title(*s).is_empty() && title(*s).len() <= 60);
            let l = lines(*s);
            assert!(!l.is_empty() && l.len() <= 5);
            assert!(l.iter().all(|x| x.len() <= 200));
        }
        assert!(
            lines(BreakStep::Breathe)
                .iter()
                .any(|l| l.contains("Close your eyes"))
        );
    }
}
