//! The shell's own wording, keyed by message id and tone (architecture 4.4: a message
//! catalogue keyed by tone and string id; English first, ready for translation).
//!
//! Reminder wording belongs to the coaching engine (V1-2) and its content review; these
//! are only the pet shell's few lines.

use hy_core::schema::PetTone;
use serde::Serialize;

use crate::geometry::TaskbarEdge;

/// A shell message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageId {
    /// Greeting at the top of the pet menu and panel; `{name}` is the pet's name.
    Greeting,
    /// After "Bring pet above clock" with the taskbar at the bottom.
    BackAboveClock,
    /// After "Bring pet above clock" with the taskbar on the right.
    BackBesideClock,
    /// After "Bring pet above clock" with the taskbar at the top, on the left or hidden.
    BackInCorner,
    /// Footer of the pet menu.
    MenuHint,
    /// Footer of the keyboard panel.
    PanelHint,
}

/// Every message id (the catalogue test checks each has text in every tone).
pub const ALL_MESSAGES: [MessageId; 6] = [
    MessageId::Greeting,
    MessageId::BackAboveClock,
    MessageId::BackBesideClock,
    MessageId::BackInCorner,
    MessageId::MenuHint,
    MessageId::PanelHint,
];

/// The text for `id` in `tone`, with `{name}` replaced by `pet_name`.
#[must_use]
pub fn message(id: MessageId, tone: PetTone, pet_name: &str) -> String {
    use MessageId as M;
    use PetTone as T;
    let text = match (id, tone) {
        (M::Greeting, T::Gentle) => "Hi, I'm {name}.",
        (M::Greeting, T::Plain) => "{name}",
        (M::Greeting, T::Cheerful) => "Hi there, I'm {name}!",
        (M::BackAboveClock, T::Gentle) => "Your pet is back above the clock.",
        (M::BackAboveClock, T::Plain) => "Pet moved above the clock.",
        (M::BackAboveClock, T::Cheerful) => "Here I am, just above the clock!",
        (M::BackBesideClock, T::Gentle) => "Your pet is back beside the clock.",
        (M::BackBesideClock, T::Plain) => "Pet moved beside the clock.",
        (M::BackBesideClock, T::Cheerful) => "Here I am, right beside the clock!",
        (M::BackInCorner, T::Gentle) => "Your pet is back in the bottom-right corner.",
        (M::BackInCorner, T::Plain) => "Pet moved to the bottom-right corner.",
        (M::BackInCorner, T::Cheerful) => "Here I am, in the bottom-right corner!",
        (M::MenuHint, T::Plain) => "Drag to move. Clicking never takes keyboard focus.",
        (M::MenuHint, _) => "Drag me to move. I never take keyboard focus from your work app.",
        (M::PanelHint, _) => "Tab moves between actions. Esc closes and returns you to your work.",
    };
    text.replace("{name}", pet_name)
}

/// The message after "Bring pet above clock" for a taskbar position (wireframe 10.1:
/// with the taskbar on the right the pet sits beside the clock, and says so).
#[must_use]
pub const fn bring_back_message(taskbar: TaskbarEdge) -> MessageId {
    match taskbar {
        TaskbarEdge::Bottom => MessageId::BackAboveClock,
        TaskbarEdge::Right => MessageId::BackBesideClock,
        TaskbarEdge::Top | TaskbarEdge::Left | TaskbarEdge::Hidden => MessageId::BackInCorner,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_message_has_text_in_every_tone() {
        for id in ALL_MESSAGES {
            for tone in PetTone::ALL {
                let t = message(id, *tone, "Shelly");
                assert!(!t.trim().is_empty(), "{id:?} {tone:?}");
                assert!(!t.contains("{name}"));
            }
        }
    }

    #[test]
    fn wireframe_wording_for_the_default_tone() {
        assert_eq!(
            message(MessageId::BackAboveClock, PetTone::Gentle, "x"),
            "Your pet is back above the clock."
        );
        assert_eq!(
            message(MessageId::Greeting, PetTone::Gentle, "Hoot"),
            "Hi, I'm Hoot."
        );
    }

    #[test]
    fn bring_back_wording_follows_the_taskbar() {
        assert_eq!(
            bring_back_message(TaskbarEdge::Bottom),
            MessageId::BackAboveClock
        );
        assert_eq!(
            bring_back_message(TaskbarEdge::Right),
            MessageId::BackBesideClock
        );
        assert_eq!(
            bring_back_message(TaskbarEdge::Top),
            MessageId::BackInCorner
        );
        assert_eq!(
            bring_back_message(TaskbarEdge::Left),
            MessageId::BackInCorner
        );
        assert_eq!(
            bring_back_message(TaskbarEdge::Hidden),
            MessageId::BackInCorner
        );
    }
}
