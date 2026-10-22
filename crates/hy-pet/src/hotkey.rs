//! The keyboard-panel hotkey (architecture 4.1: default Ctrl+Alt+H, configurable).
//!
//! A hotkey needs Ctrl or Alt, so it can never swallow ordinary typing. The Windows key
//! is refused because Windows reserves most of its combinations.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

/// The default hotkey.
pub const DEFAULT_HOTKEY: &str = "Ctrl+Alt+H";

/// Hotkeys offered in Settings. Ctrl+Alt combinations can clash with AltGr keyboard
/// layouts (wireframe open question 2), so alternatives without Alt are offered too.
pub const HOTKEY_CHOICES: [&str; 4] = [
    "Ctrl+Alt+H",
    "Ctrl+Alt+J",
    "Ctrl+Shift+F9",
    "Ctrl+Shift+F10",
];

/// The key part of a hotkey.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    /// A to Z.
    Letter(char),
    /// 0 to 9.
    Digit(u8),
    /// F1 to F12.
    Function(u8),
}

/// A parsed hotkey.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Hotkey {
    /// Ctrl held.
    pub ctrl: bool,
    /// Alt held.
    pub alt: bool,
    /// Shift held.
    pub shift: bool,
    /// The key.
    pub key: Key,
}

/// Why a hotkey was refused.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HotkeyError {
    /// Not of the form `Ctrl+Alt+H`.
    #[error("hotkey {0:?} is not of the form Ctrl+Alt+H")]
    Syntax(String),
    /// Neither Ctrl nor Alt is held.
    #[error("a hotkey needs Ctrl or Alt so it never interrupts typing")]
    NeedsCtrlOrAlt,
    /// The Windows key is reserved.
    #[error("the Windows key is reserved by Windows")]
    WindowsKey,
}

/// `MOD_ALT`.
pub const MOD_ALT: u32 = 0x0001;
/// `MOD_CONTROL`.
pub const MOD_CONTROL: u32 = 0x0002;
/// `MOD_SHIFT`.
pub const MOD_SHIFT: u32 = 0x0004;
/// `MOD_NOREPEAT`: holding the keys down does not repeat the hotkey.
pub const MOD_NOREPEAT: u32 = 0x4000;

impl Hotkey {
    /// The default, Ctrl+Alt+H.
    #[must_use]
    pub const fn default_hotkey() -> Self {
        Self {
            ctrl: true,
            alt: true,
            shift: false,
            key: Key::Letter('H'),
        }
    }

    /// Modifier flags for the Windows `RegisterHotKey` call.
    #[must_use]
    pub const fn win32_modifiers(&self) -> u32 {
        let mut m = MOD_NOREPEAT;
        if self.ctrl {
            m |= MOD_CONTROL;
        }
        if self.alt {
            m |= MOD_ALT;
        }
        if self.shift {
            m |= MOD_SHIFT;
        }
        m
    }

    /// The Windows virtual-key code.
    #[must_use]
    pub const fn virtual_key(&self) -> u32 {
        match self.key {
            Key::Letter(c) => c as u32,
            Key::Digit(d) => 0x30 + d as u32,
            Key::Function(n) => 0x70 + (n as u32 - 1),
        }
    }
}

impl FromStr for Hotkey {
    type Err = HotkeyError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let syntax = || HotkeyError::Syntax(s.to_owned());
        let parts: Vec<&str> = s.split('+').map(str::trim).collect();
        let (key_part, mods) = parts.split_last().ok_or_else(syntax)?;
        let (mut ctrl, mut alt, mut shift) = (false, false, false);
        for m in mods {
            let flag = match m.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => &mut ctrl,
                "alt" => &mut alt,
                "shift" => &mut shift,
                "win" | "windows" | "super" | "meta" => return Err(HotkeyError::WindowsKey),
                _ => return Err(syntax()),
            };
            if *flag {
                return Err(syntax());
            }
            *flag = true;
        }
        let upper = key_part.to_ascii_uppercase();
        let key = match upper.as_bytes() {
            [c] if c.is_ascii_uppercase() => Key::Letter(char::from(*c)),
            [c] if c.is_ascii_digit() => Key::Digit(c - b'0'),
            [b'F', rest @ ..] if !rest.is_empty() => {
                let n: u8 = std::str::from_utf8(rest)
                    .ok()
                    .and_then(|r| r.parse().ok())
                    .ok_or_else(syntax)?;
                if !(1..=12).contains(&n) {
                    return Err(syntax());
                }
                Key::Function(n)
            }
            _ => return Err(syntax()),
        };
        if !ctrl && !alt {
            return Err(HotkeyError::NeedsCtrlOrAlt);
        }
        Ok(Self {
            ctrl,
            alt,
            shift,
            key,
        })
    }
}

impl fmt::Display for Hotkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.ctrl {
            f.write_str("Ctrl+")?;
        }
        if self.alt {
            f.write_str("Alt+")?;
        }
        if self.shift {
            f.write_str("Shift+")?;
        }
        match self.key {
            Key::Letter(c) => write!(f, "{c}"),
            Key::Digit(d) => write!(f, "{d}"),
            Key::Function(n) => write!(f, "F{n}"),
        }
    }
}

impl Serialize for Hotkey {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Hotkey {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_formats_canonically() {
        let h: Hotkey = "ctrl + alt + h".parse().expect("ok");
        assert_eq!(h, Hotkey::default_hotkey());
        assert_eq!(h.to_string(), DEFAULT_HOTKEY);
        assert_eq!(
            "Shift+Ctrl+f9".parse::<Hotkey>().expect("ok").to_string(),
            "Ctrl+Shift+F9"
        );
        assert_eq!("Alt+7".parse::<Hotkey>().expect("ok").key, Key::Digit(7));
        for c in HOTKEY_CHOICES {
            assert_eq!(c.parse::<Hotkey>().expect("choice").to_string(), c);
        }
    }

    #[test]
    fn refuses_unsafe_or_malformed_hotkeys() {
        assert_eq!("H".parse::<Hotkey>(), Err(HotkeyError::NeedsCtrlOrAlt));
        assert_eq!(
            "Shift+H".parse::<Hotkey>(),
            Err(HotkeyError::NeedsCtrlOrAlt)
        );
        assert_eq!("Win+H".parse::<Hotkey>(), Err(HotkeyError::WindowsKey));
        for bad in [
            "",
            "Ctrl+",
            "Ctrl+Ctrl+H",
            "Ctrl+Hyper+H",
            "Ctrl+F13",
            "Ctrl+F0",
            "Ctrl+Space",
            "Ctrl+HH",
        ] {
            assert!(
                matches!(bad.parse::<Hotkey>(), Err(HotkeyError::Syntax(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn win32_codes() {
        let h = Hotkey::default_hotkey();
        assert_eq!(h.virtual_key(), 0x48);
        assert_eq!(h.win32_modifiers(), MOD_CONTROL | MOD_ALT | MOD_NOREPEAT);
        let f: Hotkey = "Ctrl+Shift+F10".parse().expect("ok");
        assert_eq!(f.virtual_key(), 0x79);
        assert_eq!(f.win32_modifiers(), MOD_CONTROL | MOD_SHIFT | MOD_NOREPEAT);
        let d: Hotkey = "Alt+0".parse().expect("ok");
        assert_eq!(d.virtual_key(), 0x30);
    }

    #[test]
    fn serde_round_trip() {
        let h: Hotkey = "Ctrl+Alt+J".parse().expect("ok");
        let j = serde_json::to_string(&h).expect("json");
        assert_eq!(j, "\"Ctrl+Alt+J\"");
        assert_eq!(serde_json::from_str::<Hotkey>(&j).expect("back"), h);
        assert!(serde_json::from_str::<Hotkey>("\"H\"").is_err());
    }
}
