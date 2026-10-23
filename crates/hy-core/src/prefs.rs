//! Display preferences (wireframe W12, slice V1-4).
//!
//! Text size scales bubble and screen text to 100, 125 or 150 percent on top of Windows
//! display scaling. Reduced motion stops the windows' and bubbles' animations (the pet's
//! own Motion switch and Windows' "Show animations" setting also apply). Higher contrast
//! draws text and borders more strongly. They are kept on this PC like every setting.

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Settings key under which [`Accessibility`] is stored.
pub const ACCESSIBILITY_KEY: &str = "accessibility";

/// The text sizes offered, in percent.
pub const TEXT_SIZES: [u16; 3] = [100, 125, 150];

/// The user's display preferences.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Accessibility {
    /// Text size in bubbles and screens: 100, 125 or 150 percent.
    pub text_pct: u16,
    /// Reduced motion: no animation in windows and bubbles.
    pub reduced_motion: bool,
    /// Higher contrast text and borders.
    pub high_contrast: bool,
}

impl Default for Accessibility {
    fn default() -> Self {
        Self {
            text_pct: 100,
            reduced_motion: false,
            high_contrast: false,
        }
    }
}

impl Accessibility {
    /// Check the preferences' rules.
    ///
    /// # Errors
    /// [`Error::Invalid`] if the text size is not one of [`TEXT_SIZES`].
    pub fn validate(&self) -> Result<()> {
        if !TEXT_SIZES.contains(&self.text_pct) {
            return Err(Error::invalid("text size must be 100, 125 or 150 percent"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_rules() {
        let a = Accessibility::default();
        a.validate().unwrap();
        for (pct, ok) in [
            (100, true),
            (125, true),
            (150, true),
            (110, false),
            (200, false),
        ] {
            let a = Accessibility {
                text_pct: pct,
                ..Accessibility::default()
            };
            assert_eq!(a.validate().is_ok(), ok, "{pct}");
        }
        let json = serde_json::to_string(&a).unwrap();
        assert_eq!(
            json,
            r#"{"text_pct":100,"reduced_motion":false,"high_contrast":false}"#
        );
    }
}
