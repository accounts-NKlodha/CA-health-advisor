//! Pet packs: the data-only format and its loader (architecture 4.4; PET-01, PET-02).
//!
//! A pack is a directory with two files:
//!
//! - `pet.json`, the manifest: id, version, names, art status, provenance and licence,
//!   default tone, three colour variants (palette slots `a` and `b`), the hitbox, and a
//!   pose for each of the eight PET-03 states (an animation from a fixed list, open or
//!   closed eyes, and an optional desaturate filter);
//! - `art.svg`, the art, checked by [`crate::svg::check_art`].
//!
//! Nothing in a pack can run: the manifest is plain data parsed with unknown fields
//! refused, the art is a strict SVG subset, and animations are names the pet page maps
//! to its own CSS. The ten built-in packs are compiled into the binary and checked again
//! at load time; a pack that fails is left out and reported, never half-loaded.
//!
//! Four pets are drawn (ported from the approved prototype). Six are placeholders marked
//! `pending_review`: they hold a slot in the library, are labelled "art pending review"
//! and cannot be chosen until their art passes the art review.

use std::collections::BTreeSet;
use std::path::Path;

use hy_core::schema::PetTone;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::svg::{ArtError, ArtReport, check_art};

/// The manifest format this build reads.
pub const PACK_FORMAT: u32 = 1;

/// Number of colour variants every pack offers (wireframe W3: 3 per pet; the store keeps
/// variant 0 to 2).
pub const COLOUR_VARIANTS: usize = 3;

/// Whether a pet's art is final.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtStatus {
    /// Drawn and selectable.
    Drawn,
    /// A placeholder awaiting the art review; shown in the library, not selectable.
    PendingReview,
}

/// Where the art came from and under what terms (PET-01: provenance recorded).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    /// Who made it and from what.
    pub origin: String,
    /// The licence or ownership statement.
    pub licence: String,
    /// The review state of the art.
    pub review: String,
}

/// One colour variant: a name and the two palette slots.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColourVariant {
    /// Shown in Settings (for example "Leaf").
    pub name: String,
    /// Main colour, `#rrggbb`.
    pub a: String,
    /// Second colour, `#rrggbb`.
    pub b: String,
}

/// The clickable part of the art in view-box units (0 to 128).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hitbox {
    /// Left.
    pub x: u16,
    /// Top.
    pub y: u16,
    /// Width.
    pub width: u16,
    /// Height.
    pub height: u16,
}

/// An animation the pet page knows. Packs choose from this list; they cannot add CSS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Animation {
    /// Still.
    None,
    /// Slow 3 px bob, 4 s (idle).
    Bob,
    /// Small wiggle, three times, then still (cue).
    Wiggle,
    /// Two small hops (celebration).
    Hop,
    /// Gentle stretch loop (exercise).
    Stretch,
}

/// Eye pose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Eyes {
    /// Open.
    Open,
    /// Closed (snoozed, focus).
    Closed,
}

/// An optional filter on the whole pet.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PoseFilter {
    /// None.
    #[default]
    None,
    /// Desaturated (disconnected).
    Desaturate,
}

/// How a pet looks in one state. With reduced motion the animation is dropped and the
/// pose (eyes and filter) is the still image for that state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatePose {
    /// Animation while motion is on.
    pub animation: Animation,
    /// Eyes.
    pub eyes: Eyes,
    /// Filter.
    #[serde(default)]
    pub filter: PoseFilter,
}

/// A pose for each of the eight PET-03 states. Every field is required.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatePoses {
    /// Idle.
    pub idle: StatePose,
    /// Cue (something is due).
    pub cue: StatePose,
    /// Exercise.
    pub exercise: StatePose,
    /// Waiting (no response).
    pub waiting: StatePose,
    /// Celebration.
    pub celebration: StatePose,
    /// Snoozed.
    pub snoozed: StatePose,
    /// Focus.
    pub focus: StatePose,
    /// Disconnected.
    pub disconnected: StatePose,
}

impl StatePoses {
    fn all(&self) -> [&StatePose; 8] {
        [
            &self.idle,
            &self.cue,
            &self.exercise,
            &self.waiting,
            &self.celebration,
            &self.snoozed,
            &self.focus,
            &self.disconnected,
        ]
    }
}

/// `pet.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// Must be [`PACK_FORMAT`].
    pub format: u32,
    /// Pack id, equal to the directory name: lowercase letters, digits and hyphens.
    pub id: String,
    /// Pack version, `major.minor.patch`.
    pub version: String,
    /// Library name (for example "Walking turtle").
    pub name: String,
    /// The pet's default display name, 1 to 20 characters (for example "Shelly").
    pub default_name: String,
    /// Drawn or pending review.
    pub art_status: ArtStatus,
    /// Origin, licence and review state.
    pub provenance: Provenance,
    /// The tone the pet starts with.
    pub default_tone: PetTone,
    /// Exactly three colour variants.
    pub palette: Vec<ColourVariant>,
    /// Clickable area.
    pub hitbox: Hitbox,
    /// Art file name; must be `art.svg`.
    pub art: String,
    /// Poses for the eight states.
    pub states: StatePoses,
}

/// A loaded, checked pack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Pack {
    /// The manifest.
    #[serde(flatten)]
    pub manifest: Manifest,
    /// The art, as checked.
    pub art_svg: String,
}

impl Pack {
    /// The pack id.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.manifest.id
    }

    /// Whether the user may choose this pet.
    #[must_use]
    pub fn selectable(&self) -> bool {
        self.manifest.art_status == ArtStatus::Drawn
    }

    /// The colour variant, falling back to the first.
    #[must_use]
    pub fn colours(&self, variant: u8) -> &ColourVariant {
        let all = &self.manifest.palette;
        all.get(usize::from(variant)).unwrap_or(&all[0])
    }
}

/// Why a pack was refused.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PackError {
    /// `pet.json` is not valid JSON for the format.
    #[error("pack {dir:?}: pet.json: {message}")]
    Json {
        /// Directory.
        dir: String,
        /// Parser message.
        message: String,
    },
    /// A manifest rule is broken.
    #[error("pack {dir:?}: {message}")]
    Rule {
        /// Directory.
        dir: String,
        /// What is wrong.
        message: String,
    },
    /// The art is refused.
    #[error("pack {dir:?}: art.svg: {source}")]
    Art {
        /// Directory.
        dir: String,
        /// The art error.
        #[source]
        source: ArtError,
    },
    /// A file could not be read.
    #[error("pack {dir:?}: {message}")]
    Io {
        /// Directory.
        dir: String,
        /// What failed.
        message: String,
    },
}

fn rule(dir: &str, message: impl Into<String>) -> PackError {
    PackError::Rule {
        dir: dir.to_owned(),
        message: message.into(),
    }
}

fn is_hex_colour(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit())
}

fn is_pack_id(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 32
        && b[0].is_ascii_lowercase()
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

fn is_version(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.len() <= 5 && p.chars().all(|c| c.is_ascii_digit()))
}

fn check_text(dir: &str, field: &str, value: &str, max_chars: usize) -> Result<(), PackError> {
    let n = value.chars().count();
    if value.trim().is_empty() || n > max_chars || value.chars().any(char::is_control) {
        return Err(rule(
            dir,
            format!("{field} must be 1 to {max_chars} printable characters"),
        ));
    }
    Ok(())
}

fn check_manifest(dir: &str, m: &Manifest, art: &ArtReport) -> Result<(), PackError> {
    if m.format != PACK_FORMAT {
        return Err(rule(
            dir,
            format!(
                "format {} is not supported (expected {PACK_FORMAT})",
                m.format
            ),
        ));
    }
    if !is_pack_id(&m.id) {
        return Err(rule(
            dir,
            "id must be 1 to 32 lowercase letters, digits or hyphens",
        ));
    }
    if m.id != dir {
        return Err(rule(
            dir,
            format!("id {:?} does not match its directory", m.id),
        ));
    }
    if !is_version(&m.version) {
        return Err(rule(dir, "version must be major.minor.patch"));
    }
    check_text(dir, "name", &m.name, 40)?;
    check_text(dir, "default_name", &m.default_name, 20)?;
    check_text(dir, "provenance.origin", &m.provenance.origin, 400)?;
    check_text(dir, "provenance.licence", &m.provenance.licence, 400)?;
    check_text(dir, "provenance.review", &m.provenance.review, 400)?;
    if m.palette.len() != COLOUR_VARIANTS {
        return Err(rule(
            dir,
            format!("palette must have exactly {COLOUR_VARIANTS} variants"),
        ));
    }
    let mut names = BTreeSet::new();
    for v in &m.palette {
        check_text(dir, "palette name", &v.name, 20)?;
        if !names.insert(v.name.as_str()) {
            return Err(rule(dir, format!("palette name {:?} is repeated", v.name)));
        }
        if !is_hex_colour(&v.a) || !is_hex_colour(&v.b) {
            return Err(rule(dir, "palette colours must be #rrggbb"));
        }
    }
    let h = m.hitbox;
    if h.width == 0 || h.height == 0 || h.x + h.width > 128 || h.y + h.height > 128 {
        return Err(rule(dir, "hitbox must be a non-empty box inside 0..128"));
    }
    if m.art != "art.svg" {
        return Err(rule(dir, "art must be \"art.svg\""));
    }
    if m.art_status == ArtStatus::Drawn {
        if !art.classes.contains("hy-a") {
            return Err(rule(
                dir,
                "drawn art must use the hy-a palette slot so colour variants apply",
            ));
        }
        let closes_eyes = m.states.all().iter().any(|p| p.eyes == Eyes::Closed);
        if closes_eyes && !art.classes.contains("eye") {
            return Err(rule(
                dir,
                "a state closes the eyes, so the art must mark its eyes with class \"eye\"",
            ));
        }
    }
    Ok(())
}

/// Parse and check one pack from its directory name and the text of its two files.
///
/// # Errors
/// [`PackError`] naming the first rule broken.
pub fn parse_pack(dir: &str, manifest_json: &str, art_svg: &str) -> Result<Pack, PackError> {
    let manifest: Manifest = serde_json::from_str(manifest_json).map_err(|e| PackError::Json {
        dir: dir.to_owned(),
        message: e.to_string(),
    })?;
    let report = check_art(art_svg).map_err(|source| PackError::Art {
        dir: dir.to_owned(),
        source,
    })?;
    check_manifest(dir, &manifest, &report)?;
    Ok(Pack {
        manifest,
        art_svg: art_svg.to_owned(),
    })
}

/// The built-in pack ids in library order (the 5 x 2 grid of wireframe W3).
pub const BUILTIN_IDS: [&str; 10] = [
    "turtle", "cat", "drop", "owl", "panda", "dog", "rabbit", "elephant", "koala", "flamingo",
];

macro_rules! builtin_sources {
    ($($id:literal),+ $(,)?) => {
        [$((
            $id,
            include_str!(concat!("../packs/", $id, "/pet.json")),
            include_str!(concat!("../packs/", $id, "/art.svg")),
        )),+]
    };
}

/// The text of the built-in packs: (directory, `pet.json`, `art.svg`).
pub const BUILTIN_SOURCES: [(&str, &str, &str); 10] = builtin_sources!(
    "turtle", "cat", "drop", "owl", "panda", "dog", "rabbit", "elephant", "koala", "flamingo",
);

/// The pet library.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Library {
    packs: Vec<Pack>,
}

/// A library that could not be used at all.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("no selectable pet pack loaded ({} refused)", .0.len())]
pub struct NoUsablePacks(pub Vec<PackError>);

impl Library {
    /// Load packs from (directory, manifest, art) texts. Refused packs are left out and
    /// returned alongside, so one bad pack never stops the pet.
    ///
    /// # Errors
    /// [`NoUsablePacks`] if no selectable pack loads, or ids repeat.
    pub fn from_sources<'a>(
        sources: impl IntoIterator<Item = (&'a str, &'a str, &'a str)>,
    ) -> Result<(Self, Vec<PackError>), NoUsablePacks> {
        let mut packs: Vec<Pack> = Vec::new();
        let mut refused = Vec::new();
        for (dir, json, svg) in sources {
            match parse_pack(dir, json, svg) {
                Ok(p) if packs.iter().any(|q| q.id() == p.id()) => {
                    refused.push(rule(dir, "id is repeated"));
                }
                Ok(p) => packs.push(p),
                Err(e) => refused.push(e),
            }
        }
        if packs.iter().any(Pack::selectable) {
            Ok((Self { packs }, refused))
        } else {
            Err(NoUsablePacks(refused))
        }
    }

    /// The ten built-in packs, checked at load time.
    ///
    /// # Errors
    /// [`NoUsablePacks`] if none of them loads.
    pub fn builtin() -> Result<(Self, Vec<PackError>), NoUsablePacks> {
        Self::from_sources(BUILTIN_SOURCES)
    }

    /// Load every pack directory under `root` (used by the build-time check and for
    /// packs delivered as files later).
    ///
    /// # Errors
    /// [`NoUsablePacks`] if no selectable pack loads or `root` cannot be read.
    pub fn load_dir(root: &Path) -> Result<(Self, Vec<PackError>), NoUsablePacks> {
        let io = |dir: &str, e: std::io::Error| PackError::Io {
            dir: dir.to_owned(),
            message: e.to_string(),
        };
        let entries = std::fs::read_dir(root)
            .map_err(|e| NoUsablePacks(vec![io(&root.display().to_string(), e)]))?;
        let mut dirs: Vec<_> = entries
            .filter_map(std::result::Result::ok)
            .filter(|e| e.path().is_dir())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        dirs.sort();
        let mut texts = Vec::new();
        let mut refused = Vec::new();
        for dir in dirs {
            let read = |f: &str| std::fs::read_to_string(root.join(&dir).join(f));
            match (read("pet.json"), read("art.svg")) {
                (Ok(j), Ok(s)) => texts.push((dir, j, s)),
                (Err(e), _) | (_, Err(e)) => refused.push(io(&dir, e)),
            }
        }
        let (lib, mut more) = Self::from_sources(
            texts
                .iter()
                .map(|(d, j, s)| (d.as_str(), j.as_str(), s.as_str())),
        )?;
        refused.append(&mut more);
        Ok((lib, refused))
    }

    /// Every pack, in library order.
    #[must_use]
    pub fn packs(&self) -> &[Pack] {
        &self.packs
    }

    /// A pack by id.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Pack> {
        self.packs.iter().find(|p| p.id() == id)
    }

    /// A selectable pack by id.
    #[must_use]
    pub fn selectable(&self, id: &str) -> Option<&Pack> {
        self.get(id).filter(|p| p.selectable())
    }

    /// The default pet: the first selectable pack. A library always has one.
    #[must_use]
    pub fn default_pack(&self) -> &Pack {
        self.packs
            .iter()
            .find(|p| p.selectable())
            .unwrap_or(&self.packs[0])
    }
}

#[cfg(test)]
mod tests;
