//! The meditation pause's bundled text (V2-4; wireframes B12 and W11, CAL-01).
//!
//! **Provisional content, pending the planned content review**, like the movement
//! routines ([`super::movement`]). It is a short, text-only pause with no health claims:
//! no audio, nothing timed to the breath, and an eyes-open version that does not mention
//! the breath at all, for anyone who finds breath focus uncomfortable. Every screen shows
//! [`PROVISIONAL_NOTICE`].
//!
//! A pause has [`MEDITATION_PARTS`] parts of equal length (the chosen 2, 5 or 10 minutes
//! divided by five). Each part is a heading and one or two short lines.

pub use crate::schema::{MEDITATION_MINUTES, MEDITATION_PARTS, MeditationVersion};

/// The length offered first (CAL-01).
pub const DEFAULT_MINUTES: u32 = 5;

/// The content has not passed its review yet. Every screen showing it says so.
pub const PROVISIONAL: bool = true;

/// Shown on every meditation screen while [`PROVISIONAL`] is true.
pub const PROVISIONAL_NOTICE: &str = "Provisional content, pending content review";

/// The invitation's words (wireframe B12).
pub const INVITATION_TITLE: &str = "A short pause?";
/// The invitation's line.
pub const INVITATION_LINE: &str = "A few quiet minutes with short text prompts. No sound. Optional, and nothing is owed if you skip it.";

/// The last screen (flow 10).
pub const DONE_TITLE: &str = "Done. Back to work whenever you are ready.";

/// One part of a pause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Part {
    /// Heading.
    pub title: &'static str,
    /// One or two short lines.
    pub lines: &'static [&'static str],
}

const BREATH: [Part; 5] = [
    Part {
        title: "Settle",
        lines: &[
            "Sit comfortably with your feet on the floor and your hands resting.",
            "Close your eyes, or rest your gaze low and soft.",
        ],
    },
    Part {
        title: "Notice the breath",
        lines: &["Notice the air moving in and out. There is no need to change it."],
    },
    Part {
        title: "Count",
        lines: &[
            "Count each out-breath, from one to five, then begin again.",
            "If you lose count, simply start again at one.",
        ],
    },
    Part {
        title: "Let thoughts pass",
        lines: &["When a thought comes, notice it, let it go, and come back to the breath."],
    },
    Part {
        title: "Return",
        lines: &[
            "Widen your attention to the room: sounds, the chair, your feet on the floor.",
            "Open your eyes when you are ready.",
        ],
    },
];

const EYES_OPEN: [Part; 5] = [
    Part {
        title: "Settle",
        lines: &[
            "Sit comfortably with your feet on the floor and your eyes open.",
            "Rest your gaze on one spot away from the screen.",
        ],
    },
    Part {
        title: "Sounds",
        lines: &["Notice three sounds around you, near and far. Just listen."],
    },
    Part {
        title: "Touch",
        lines: &["Notice where you touch the chair and the floor. Feel the weight."],
    },
    Part {
        title: "Look",
        lines: &["Let your gaze slowly take in the room: colours, shapes and light."],
    },
    Part {
        title: "Return",
        lines: &["Take a moment, then turn back to your work when you are ready."],
    },
];

/// The parts of `version`.
#[must_use]
pub const fn parts(version: MeditationVersion) -> &'static [Part; 5] {
    match version {
        MeditationVersion::Breath => &BREATH,
        MeditationVersion::EyesOpen => &EYES_OPEN,
    }
}

/// The link that switches to the other version.
#[must_use]
pub const fn switch_label(version: MeditationVersion) -> &'static str {
    match version {
        MeditationVersion::Breath => "Eyes-open version (no breath focus)",
        MeditationVersion::EyesOpen => "Breath version",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_version_has_five_short_parts() {
        assert_eq!(MEDITATION_PARTS, 5);
        for v in MeditationVersion::ALL {
            for p in parts(*v) {
                assert!(!p.title.is_empty() && p.title.len() <= 60);
                assert!(!p.lines.is_empty() && p.lines.len() <= 2);
                assert!(p.lines.iter().all(|l| l.len() <= 200));
            }
        }
    }

    #[test]
    fn the_eyes_open_version_never_mentions_the_breath() {
        for p in parts(MeditationVersion::EyesOpen) {
            let text = format!("{} {}", p.title, p.lines.join(" ")).to_lowercase();
            assert!(
                !text.contains("breath") && !text.contains("breathe"),
                "{text}"
            );
            assert!(!text.contains("close your eyes"), "{text}");
        }
    }

    #[test]
    fn the_default_length_is_offered() {
        assert!(MEDITATION_MINUTES.contains(&DEFAULT_MINUTES));
    }
}
