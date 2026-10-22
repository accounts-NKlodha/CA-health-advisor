//! The guided movement routine's bundled content (MOV-01, MOV-02; wireframe B4 and W6).
//!
//! **Provisional content, pending the planned content review.** The four movements, their
//! steps, doses and seated alternatives come from the approved prototype (revision 3),
//! where they are marked "provisional content, for layout only". They are gentle,
//! desk-safe suggestions with no health claims. They are not a treatment or an exercise
//! prescription, and the wireframes say no routine ships until the content review passes
//! (wireframe 10.2, items 3 and 5). Every screen that shows them carries
//! [`PROVISIONAL_NOTICE`].
//!
//! The content is versioned ([`CONTENT_VERSION`]); each routine result records the
//! routine's id and the version it used, so a later content change never rewrites what a
//! past routine meant.
//!
//! Monday to Saturday each have three movements (about 2 minutes). Sunday is optional:
//! nothing is scheduled and the user may pick any day's routine. Missed days are never
//! caught up.

use chrono::Weekday;

use crate::schema::RoutineVariant;

/// Version of the bundled movement content. Raise it with any change to a routine.
pub const CONTENT_VERSION: u32 = 1;

/// The content has not passed its review yet. Every screen showing it says so.
pub const PROVISIONAL: bool = true;

/// Shown on every movement screen while [`PROVISIONAL`] is true.
pub const PROVISIONAL_NOTICE: &str = "Provisional content, pending content review";

/// The seating check and stop wording on the routine's first screen (wireframe B4).
pub const INTRO_SAFETY: &str = "Use a steady chair; if it has wheels, lock them or hold the desk. Move gently and stop if anything feels uncomfortable. Skipping is fine.";

/// The stop wording on every movement screen.
pub const STEP_SAFETY: &str =
    "Move gently, within a comfortable range. Stop if anything hurts or feels wrong.";

/// Movements in each day's routine.
pub const MOVEMENTS_PER_ROUTINE: u32 = 3;

/// One movement as shown on its screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Movement {
    /// Stable id.
    pub id: &'static str,
    /// Its name, for example "Shoulder rolls".
    pub name: &'static str,
    /// Seated, standing, or both.
    pub place: &'static str,
    /// Repetitions or seconds, and about how long.
    pub dose: &'static str,
    /// Three to five short steps, one action each.
    pub steps: &'static [&'static str],
    /// Heading of the alternative shown beside the steps.
    pub alternative_title: &'static str,
    /// The other version (seated or standing) in words.
    pub alternative: &'static str,
    /// Illustration id, drawn by the pet page.
    pub figure: &'static str,
    /// The illustration in words (its text equivalent).
    pub figure_label: &'static str,
}

/// A movement with an optional seated replacement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Exercise {
    /// The movement as listed (standing where it has a standing form).
    pub main: Movement,
    /// A different movement used when the user chooses the seated version.
    pub seated: Option<Movement>,
}

impl Exercise {
    /// The movement shown for `variant` (`None` before the user has chosen).
    #[must_use]
    pub const fn for_variant(&self, variant: Option<RoutineVariant>) -> &Movement {
        match (variant, &self.seated) {
            (Some(RoutineVariant::Seated), Some(s)) => s,
            _ => &self.main,
        }
    }
}

/// Neck side stretch (provisional).
pub const NECK: Exercise = Exercise {
    main: Movement {
        id: "neck-side-stretch",
        name: "Neck side stretch",
        place: "Seated or standing",
        dose: "Hold about 10 s, once each side, about 30 s",
        steps: &[
            "Sit tall with your feet flat and your shoulders relaxed.",
            "Slowly tilt one ear towards the same shoulder, only as far as is comfortable.",
            "Hold for about 10 seconds and breathe normally.",
            "Come back to the centre, then repeat on the other side.",
        ],
        alternative_title: "Seated alternative",
        alternative: "This one is done seated. Standing works too: stand tall, feet hip-width apart, and follow the same steps.",
        figure: "neck",
        figure_label: "Figure seen from the front, tilting the head slowly towards one shoulder, then the other",
    },
    seated: None,
};

/// Shoulder rolls (provisional).
pub const SHOULDERS: Exercise = Exercise {
    main: Movement {
        id: "shoulder-rolls",
        name: "Shoulder rolls",
        place: "Seated or standing",
        dose: "5 rolls backwards, then 5 forwards, about 30 s",
        steps: &[
            "Sit or stand tall with your arms loose by your sides.",
            "Lift both shoulders up towards your ears.",
            "Roll them back and down in a slow, smooth circle.",
            "Do 5 rolls backwards, then 5 forwards.",
        ],
        alternative_title: "Seated alternative",
        alternative: "Works the same seated: sit tall, let your arms hang beside the chair and follow the same steps.",
        figure: "shoulders",
        figure_label: "Figure seen from the side, rolling the shoulder up, back and down in a circle",
    },
    seated: None,
};

/// Seated spinal twist (provisional).
pub const TWIST: Exercise = Exercise {
    main: Movement {
        id: "seated-spinal-twist",
        name: "Seated spinal twist",
        place: "Seated",
        dose: "Hold about 10 s, once each side, about 40 s",
        steps: &[
            "Sit tall towards the front of your chair, feet flat on the floor.",
            "Rest one hand on the opposite knee and the other hand on the side of the seat.",
            "Gently turn your chest and head towards the seat hand; keep your hips facing forward.",
            "Hold for about 10 seconds, breathing normally, then come back to the centre.",
            "Swap hands and repeat on the other side.",
        ],
        alternative_title: "Seated alternative",
        alternative: "This movement is seated. If your chair swivels, plant your feet or hold the desk so the chair stays still. Standing option: hands on hips, turn your upper body gently each way.",
        figure: "twist",
        figure_label: "Seated figure seen from above, turning the shoulders and head to one side and then the other while the hips stay forward",
    },
    seated: None,
};

/// Standing calf raises, with seated heel raises as the seated version (provisional).
pub const CALF: Exercise = Exercise {
    main: Movement {
        id: "standing-calf-raises",
        name: "Standing calf raises",
        place: "Standing, holding the chair back",
        dose: "10 slow raises, about 40 s",
        steps: &[
            "Stand behind your chair and rest your hands lightly on the backrest.",
            "Slowly rise up onto the balls of your feet.",
            "Pause for a moment at the top.",
            "Lower your heels slowly back to the floor. Repeat 10 times.",
        ],
        alternative_title: "Seated alternative: seated heel raises",
        alternative: "Sit tall with your feet flat. Lift both heels, keeping your toes on the floor, then lower them slowly. 15 times.",
        figure: "calf",
        figure_label: "Figure standing behind a chair, holding the backrest and rising onto the balls of the feet",
    },
    seated: Some(Movement {
        id: "seated-heel-raises",
        name: "Seated heel raises",
        place: "Seated",
        dose: "15 slow raises, about 40 s",
        steps: &[
            "Sit tall with your feet flat on the floor, hip-width apart.",
            "Lift both heels, keeping your toes on the floor.",
            "Pause for a moment at the top.",
            "Lower your heels slowly. Repeat 15 times.",
        ],
        alternative_title: "Seated alternative (shown)",
        alternative: "You chose seated. Standing version: stand behind your chair, hold the backrest and rise onto the balls of your feet 10 times.",
        figure: "heel",
        figure_label: "Seated figure lifting both heels while the toes stay on the floor",
    }),
};

/// Every exercise in the library (wireframe W6).
pub const LIBRARY: [&Exercise; 4] = [&NECK, &SHOULDERS, &TWIST, &CALF];

/// One day's routine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DayRoutine {
    /// The day it is scheduled on.
    pub day: Weekday,
    /// Content id recorded with each result (with [`CONTENT_VERSION`]).
    pub id: &'static str,
    /// Its movements, in order.
    pub movements: [&'static Exercise; MOVEMENTS_PER_ROUTINE as usize],
}

/// The Monday to Saturday programme (prototype revision 3, provisional).
pub const PROGRAMME: [DayRoutine; 6] = [
    DayRoutine {
        day: Weekday::Mon,
        id: "mon",
        movements: [&NECK, &SHOULDERS, &TWIST],
    },
    DayRoutine {
        day: Weekday::Tue,
        id: "tue",
        movements: [&NECK, &TWIST, &CALF],
    },
    DayRoutine {
        day: Weekday::Wed,
        id: "wed",
        movements: [&SHOULDERS, &CALF, &NECK],
    },
    DayRoutine {
        day: Weekday::Thu,
        id: "thu",
        movements: [&TWIST, &SHOULDERS, &CALF],
    },
    DayRoutine {
        day: Weekday::Fri,
        id: "fri",
        movements: [&NECK, &SHOULDERS, &CALF],
    },
    DayRoutine {
        day: Weekday::Sat,
        id: "sat",
        movements: [&CALF, &TWIST, &NECK],
    },
];

/// The routine scheduled on `day`; `None` on Sunday (optional, nothing scheduled).
#[must_use]
pub fn routine_for(day: Weekday) -> Option<&'static DayRoutine> {
    PROGRAMME.iter().find(|r| r.day == day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{EventPayload, RoutineOutcome};

    #[test]
    fn monday_to_saturday_have_three_movements_and_sunday_is_optional() {
        for d in [
            Weekday::Mon,
            Weekday::Tue,
            Weekday::Wed,
            Weekday::Thu,
            Weekday::Fri,
            Weekday::Sat,
        ] {
            let r = routine_for(d).unwrap();
            assert_eq!(r.day, d);
            assert_eq!(r.movements.len(), MOVEMENTS_PER_ROUTINE as usize);
        }
        assert!(routine_for(Weekday::Sun).is_none());
    }

    #[test]
    fn content_is_labelled_provisional_and_fits_the_screens() {
        const { assert!(PROVISIONAL) };
        assert!(PROVISIONAL_NOTICE.contains("pending content review"));
        let mut ids = std::collections::HashSet::new();
        for e in LIBRARY {
            for m in std::iter::once(&e.main).chain(e.seated.iter()) {
                assert!(ids.insert(m.id), "duplicate movement id {}", m.id);
                assert!((3..=5).contains(&m.steps.len()), "{}", m.id);
                assert!(m.name.chars().count() <= 40);
                for s in m.steps {
                    assert!(s.chars().count() <= 120, "{s}");
                }
                assert!(m.alternative.chars().count() <= 300);
                assert!(!m.figure.is_empty() && !m.figure_label.is_empty());
            }
        }
        // The seated variant swaps only where a separate seated movement exists.
        assert_eq!(
            CALF.for_variant(Some(RoutineVariant::Seated)).name,
            "Seated heel raises"
        );
        assert_eq!(
            CALF.for_variant(Some(RoutineVariant::Standing)).name,
            "Standing calf raises"
        );
        assert_eq!(NECK.for_variant(Some(RoutineVariant::Seated)), &NECK.main);
    }

    #[test]
    fn routine_ids_are_valid_event_routine_ids() {
        for r in PROGRAMME {
            let payload = EventPayload::MovementSession {
                session_id: uuid::Uuid::nil(),
                routine_id: r.id.to_owned(),
                routine_version: CONTENT_VERSION,
                variant: None,
                outcome: RoutineOutcome::Skipped,
                steps_done: 0,
                steps_skipped: 0,
                steps_total: MOVEMENTS_PER_ROUTINE,
            };
            payload.validate().unwrap();
        }
    }

    #[test]
    fn no_health_claims_in_the_wording() {
        // Desk-safe suggestions only: no treatment, cure or medical wording.
        let banned = [
            "cure",
            "treat",
            "therapy",
            "pain relief",
            "prevent",
            "heal",
            "diagnos",
        ];
        for e in LIBRARY {
            for m in std::iter::once(&e.main).chain(e.seated.iter()) {
                let text =
                    format!("{} {} {}", m.name, m.steps.join(" "), m.alternative).to_lowercase();
                for b in banned {
                    assert!(!text.contains(b), "{} mentions {b:?}", m.id);
                }
            }
        }
    }
}
