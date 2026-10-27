//! Pet edits (PET-02; wireframe W3): choose a pet, name, size, colour, edge, motion,
//! tone, position, and "Reset to defaults". Every edit is saved at once through the
//! `hy-core` store (`pet_profile` table, `settings` for the hotkey), so it is kept across
//! restarts.

use hy_core::error::Error as StoreError;
use hy_core::schema::{PetEdge, PetTone};
use hy_core::store::{PetProfile, Store};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::hotkey::Hotkey;
use crate::pack::Library;
use crate::placement::is_offered_scale;

/// Setting keys owned by the pet shell.
pub mod keys {
    /// The keyboard-panel hotkey: a string such as `"Ctrl+Alt+H"`, or `null` for none.
    pub const PANEL_HOTKEY: &str = "pet.panel_hotkey";
}

/// Longest pet name, characters (wireframe W3).
pub const NAME_MAX_CHARS: usize = 20;

/// One edit from Settings, the pet menu or the keyboard panel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PetEdit {
    /// Choose another pet (drawn pets only).
    ChoosePet {
        /// Pack id.
        pet_id: String,
    },
    /// Rename the pet.
    Rename {
        /// 1 to 20 characters.
        name: String,
    },
    /// Change the size.
    Size {
        /// 50 to 150 in steps of 5.
        pct: u16,
    },
    /// Choose a colour variant.
    Colour {
        /// 0 to 2.
        variant: u8,
    },
    /// Choose the screen edge.
    Edge {
        /// Left, right, or where I drop it.
        edge: PetEdge,
    },
    /// Animation on or off.
    Motion {
        /// On.
        on: bool,
    },
    /// Wording tone.
    Tone {
        /// Gentle, plain or cheerful.
        tone: PetTone,
    },
    /// The pet window's top-left corner, physical pixels on the virtual desktop.
    Position {
        /// Left.
        x: i32,
        /// Top.
        y: i32,
    },
    /// Forget the position (the pet goes home above the clock).
    ClearPosition,
    /// Restore the defaults of the current pet (keeps the chosen pet).
    ResetDefaults,
}

/// Why an edit was refused.
#[derive(Debug, Error)]
pub enum EditError {
    /// The edit breaks a rule; nothing was saved.
    #[error("{0}")]
    Invalid(String),
    /// The store failed.
    #[error(transparent)]
    Store(#[from] StoreError),
}

fn invalid(msg: impl Into<String>) -> EditError {
    EditError::Invalid(msg.into())
}

/// The defaults for the library's default pet (what the user sees before setup saves a
/// choice).
#[must_use]
pub fn default_profile(lib: &Library) -> PetProfile {
    let pack = lib.default_pack();
    let mut p = PetProfile::defaults(pack.id(), &pack.manifest.default_name);
    p.tone = pack.manifest.default_tone;
    p
}

/// The current profile: the saved one, or the defaults if none is saved yet. A saved
/// pet that is no longer selectable (unknown, or its pack failed to load) is replaced by
/// the default pet; the other edits are kept.
///
/// # Errors
/// [`StoreError`] if the store cannot be read.
pub fn current_profile(store: &Store, lib: &Library) -> Result<PetProfile, StoreError> {
    let Some(mut p) = store.pet_profile()? else {
        return Ok(default_profile(lib));
    };
    if lib.selectable(&p.pet_id).is_none() {
        let pack = lib.default_pack();
        if lib
            .get(&p.pet_id)
            .is_some_and(|old| old.manifest.default_name == p.display_name)
        {
            p.display_name.clone_from(&pack.manifest.default_name);
        }
        p.pet_id = pack.id().to_owned();
    }
    Ok(p)
}

/// Whether a profile has been saved (setup, V1-4, uses this to know a pet was chosen).
///
/// # Errors
/// [`StoreError`] if the store cannot be read.
pub fn has_saved_profile(store: &Store) -> Result<bool, StoreError> {
    Ok(store.pet_profile()?.is_some())
}

fn check_name(name: &str) -> Result<String, EditError> {
    let name = name.trim();
    let n = name.chars().count();
    if n == 0 || n > NAME_MAX_CHARS || name.chars().any(char::is_control) {
        return Err(invalid(format!(
            "The name must be 1 to {NAME_MAX_CHARS} characters."
        )));
    }
    Ok(name.to_owned())
}

/// Apply one edit and save it at once. Returns the saved profile.
///
/// # Errors
/// [`EditError::Invalid`] if the edit breaks a rule (nothing is saved), or
/// [`EditError::Store`] if saving fails.
pub fn apply_edit(store: &Store, lib: &Library, edit: &PetEdit) -> Result<PetProfile, EditError> {
    let mut p = current_profile(store, lib)?;
    match edit {
        PetEdit::ChoosePet { pet_id } => {
            let Some(next) = lib.get(pet_id) else {
                return Err(invalid("That pet is not in the library."));
            };
            if !next.selectable() {
                return Err(invalid(
                    "That pet's art is pending review, so it cannot be chosen yet.",
                ));
            }
            // A name the user never changed follows the pet; a chosen name is kept.
            let old_default = lib.get(&p.pet_id).map(|o| o.manifest.default_name.as_str());
            if old_default == Some(p.display_name.as_str()) {
                p.display_name.clone_from(&next.manifest.default_name);
            }
            p.pet_id.clone_from(pet_id);
        }
        PetEdit::Rename { name } => p.display_name = check_name(name)?,
        PetEdit::Size { pct } => {
            if !is_offered_scale(*pct) {
                return Err(invalid("The size must be 50% to 150% in steps of 5."));
            }
            p.scale_pct = *pct;
        }
        PetEdit::Colour { variant } => {
            if *variant > 2 {
                return Err(invalid("Each pet has three colours."));
            }
            p.colour_variant = *variant;
        }
        PetEdit::Edge { edge } => p.edge = *edge,
        PetEdit::Motion { on } => p.motion = *on,
        PetEdit::Tone { tone } => p.tone = *tone,
        PetEdit::Position { x, y } => p.position = Some((*x, *y)),
        PetEdit::ClearPosition => p.position = None,
        PetEdit::ResetDefaults => {
            let pack = lib
                .selectable(&p.pet_id)
                .unwrap_or_else(|| lib.default_pack());
            let mut d = PetProfile::defaults(pack.id(), &pack.manifest.default_name);
            d.tone = pack.manifest.default_tone;
            p = d;
        }
    }
    p.validate().map_err(|e| invalid(e.to_string()))?;
    store.save_pet_profile(&p)?;
    Ok(p)
}

/// The keyboard-panel hotkey: the saved one, `None` if the user turned it off, or the
/// default (Ctrl+Alt+H) if never set or unreadable.
///
/// # Errors
/// [`StoreError`] if the store cannot be read.
pub fn panel_hotkey(store: &Store) -> Result<Option<Hotkey>, StoreError> {
    let saved: Option<Option<String>> = store.setting(keys::PANEL_HOTKEY).unwrap_or(None);
    Ok(match saved {
        None => Some(Hotkey::default_hotkey()),
        Some(None) => None,
        Some(Some(s)) => Some(s.parse().unwrap_or_else(|_| Hotkey::default_hotkey())),
    })
}

/// Save the keyboard-panel hotkey (`None` turns it off).
///
/// # Errors
/// [`StoreError`] if the store cannot be written.
pub fn set_panel_hotkey(store: &Store, hotkey: Option<Hotkey>) -> Result<(), StoreError> {
    store.put_setting(keys::PANEL_HOTKEY, &hotkey.map(|h| h.to_string()))
}

#[cfg(test)]
mod tests;
