//! Pet states, badges and text tags (PET-03; wireframe section 4).
//!
//! The coaching engine (V1-2) sets one [`PetState`]. Each maps to one of the eight PET-03
//! base states, whose pose comes from the pet's pack, plus an icon badge and a text tag
//! so that no state depends on colour or animation alone (wireframe rule 5). Some states
//! dim the pet (quiet hours, meeting, paused).

use serde::{Deserialize, Serialize};

/// The eight PET-03 states every pack draws a pose for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BaseState {
    /// Idle.
    Idle,
    /// Cue: something is due or offered.
    Cue,
    /// Exercise.
    Exercise,
    /// Waiting: a reminder had no response.
    Waiting,
    /// Celebration.
    Celebration,
    /// Snoozed.
    Snoozed,
    /// Focus.
    Focus,
    /// Disconnected from the office server.
    Disconnected,
}

/// What the engine can ask the pet to show (wireframe section 4). Hidden for a
/// presentation is not a state: it is [`crate::shell::HideReason::Presentation`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PetState {
    /// Default.
    Idle,
    /// Something is due or offered (review, check-in).
    Cue,
    /// A reminder card is open.
    Reminder,
    /// Movement intro or steps.
    Exercise,
    /// A reminder waited 10 minutes with no response.
    Waiting,
    /// A completion or a goal reached.
    Celebration,
    /// A reminder is snoozed.
    Snoozed,
    /// Focus is on.
    Focus,
    /// A Pomodoro timer is running.
    Pomodoro,
    /// Outside work hours or on a non-workday.
    Quiet,
    /// The manual "in a meeting" toggle.
    Meeting,
    /// The user paused coaching.
    Paused,
    /// The office server is unreachable (paired users only).
    Disconnected,
}

impl PetState {
    /// Every state, in the order of the wireframe table.
    pub const ALL: [Self; 13] = [
        Self::Idle,
        Self::Cue,
        Self::Reminder,
        Self::Exercise,
        Self::Waiting,
        Self::Celebration,
        Self::Snoozed,
        Self::Focus,
        Self::Pomodoro,
        Self::Quiet,
        Self::Meeting,
        Self::Paused,
        Self::Disconnected,
    ];
}

/// An icon badge at the pet's top right. Icons are drawn by the pet page and always
/// come with text (the tag or the accessible name).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum Badge {
    /// "!".
    Exclaim,
    /// Bell.
    Bell,
    /// Moving figure.
    Move,
    /// A number (waiting reminders).
    Count(u8),
    /// Star.
    Star,
    /// "zz".
    Snooze,
    /// Target.
    Focus,
    /// Clock.
    Timer,
    /// Moon.
    Moon,
    /// Mute.
    Mute,
    /// Pause.
    Pause,
    /// Plug.
    Plug,
}

/// How a state looks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StateLook {
    /// The state.
    pub state: PetState,
    /// The base pose from the pack.
    pub base: BaseState,
    /// The badge, if any.
    pub badge: Option<Badge>,
    /// The text tag under the pet, if any.
    pub tag: Option<String>,
    /// Whether the pet is dimmed.
    pub dim: bool,
    /// Whether the eyes close regardless of the pack pose.
    pub eyes_closed: bool,
    /// The state's name for screen readers.
    pub label: &'static str,
}

impl StateLook {
    /// The look of `state`. `tag` replaces the default tag (for live text such as
    /// "Focus 45 min" or "Work 24:59"); `Some("")` removes it.
    #[must_use]
    pub fn of(state: PetState, tag: Option<&str>) -> Self {
        use Badge as B;
        use BaseState as S;
        let (base, badge, default_tag, dim, eyes_closed, label) = match state {
            PetState::Idle => (S::Idle, None, None, false, false, "Idle"),
            PetState::Cue => (S::Cue, Some(B::Exclaim), None, false, false, "Nudge"),
            PetState::Reminder => (
                S::Cue,
                Some(B::Bell),
                None,
                false,
                false,
                "Reminder card open",
            ),
            PetState::Exercise => (
                S::Exercise,
                Some(B::Move),
                Some("Moving"),
                false,
                false,
                "Exercise",
            ),
            PetState::Waiting => (
                S::Waiting,
                Some(B::Count(1)),
                Some("1 reminder waiting"),
                false,
                false,
                "Waiting, no response",
            ),
            PetState::Celebration => (
                S::Celebration,
                Some(B::Star),
                None,
                false,
                false,
                "Celebration",
            ),
            PetState::Snoozed => (
                S::Snoozed,
                Some(B::Snooze),
                Some("Snoozed 10 min"),
                false,
                true,
                "Snoozed",
            ),
            PetState::Focus => (
                S::Focus,
                Some(B::Focus),
                Some("Focus on"),
                false,
                true,
                "Focus mode",
            ),
            PetState::Pomodoro => (
                S::Focus,
                Some(B::Timer),
                Some("Pomodoro"),
                false,
                true,
                "Pomodoro",
            ),
            PetState::Quiet => (
                S::Snoozed,
                Some(B::Moon),
                Some("Off hours"),
                true,
                true,
                "Off hours",
            ),
            PetState::Meeting => (
                S::Idle,
                Some(B::Mute),
                Some("Meeting - paused"),
                true,
                false,
                "Meeting, reminders paused",
            ),
            PetState::Paused => (
                S::Snoozed,
                Some(B::Pause),
                Some("Paused"),
                true,
                true,
                "Coaching paused",
            ),
            PetState::Disconnected => (
                S::Disconnected,
                Some(B::Plug),
                Some("Offline - still works"),
                false,
                false,
                "Disconnected from the office server",
            ),
        };
        let tag = match tag {
            Some("") => None,
            Some(t) => Some(t.chars().take(40).collect()),
            None => default_tag.map(str::to_owned),
        };
        Self {
            state,
            base,
            badge,
            tag,
            dim,
            eyes_closed,
            label,
        }
    }

    /// The pet's accessible name, for example "Shelly the pet, Focus mode, Focus on".
    #[must_use]
    pub fn accessible_name(&self, pet_name: &str) -> String {
        let mut s = format!("{pet_name} the pet, {}", self.label);
        if let Some(t) = &self.tag {
            s.push_str(", ");
            s.push_str(t);
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_state_has_text_and_every_base_state_is_used() {
        let mut bases = std::collections::BTreeSet::new();
        for s in PetState::ALL {
            let look = StateLook::of(s, None);
            assert!(!look.label.is_empty());
            // Status never depends on colour alone: a non-idle state has a badge.
            if s != PetState::Idle {
                assert!(look.badge.is_some(), "{s:?} needs a badge");
            }
            bases.insert(format!("{:?}", look.base));
        }
        assert_eq!(bases.len(), 8, "all eight PET-03 states reachable");
    }

    #[test]
    fn tag_override_and_removal() {
        let look = StateLook::of(PetState::Focus, Some("Focus 60 min"));
        assert_eq!(look.tag.as_deref(), Some("Focus 60 min"));
        assert_eq!(
            look.accessible_name("Shelly"),
            "Shelly the pet, Focus mode, Focus 60 min"
        );
        assert_eq!(StateLook::of(PetState::Focus, Some("")).tag, None);
        let long = "x".repeat(100);
        assert_eq!(
            StateLook::of(PetState::Idle, Some(&long))
                .tag
                .map(|t| t.len()),
            Some(40)
        );
        assert_eq!(
            StateLook::of(PetState::Idle, None).accessible_name("Miso"),
            "Miso the pet, Idle"
        );
    }

    #[test]
    fn dimmed_states() {
        let dim: Vec<PetState> = PetState::ALL
            .into_iter()
            .filter(|s| StateLook::of(*s, None).dim)
            .collect();
        assert_eq!(dim, [PetState::Quiet, PetState::Meeting, PetState::Paused]);
    }

    #[test]
    fn badge_serialises_with_kind() {
        let v = serde_json::to_value(Badge::Count(3)).expect("json");
        assert_eq!(v, serde_json::json!({"kind": "count", "value": 3}));
        let v = serde_json::to_value(Badge::Bell).expect("json");
        assert_eq!(v, serde_json::json!({"kind": "bell"}));
    }
}
