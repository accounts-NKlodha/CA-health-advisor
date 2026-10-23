//! "Try next week": up to three short suggestions on the weekly review (wireframe W13).
//!
//! Each suggestion is one line with Apply or Not for me. Apply really changes the
//! setting, the row then says what changed, and Undo restores the exact prior value. The
//! rules below are D (the wireframes give examples, not rules); each needs enough of the
//! week's own data, and none is offered without enough data:
//!
//! - **Serving button:** an amount logged at least [`SERVING_MIN_LOGS`] times this week
//!   that is not a serving button, while there is room for another button.
//! - **Default movement version:** "Ask each time" is set, and at least
//!   [`VERSION_MIN_ROUTINES`] routines this week used the same version and none the
//!   other.
//! - **Lighter reminders:** at least [`LIGHTER_MIN_OFFERED`] breaks were offered and more
//!   than half were set aside (skipped, dismissed or left unanswered); one step lighter.
//! - **Keep:** with none of the above in a normal week where most offered breaks were
//!   taken, a line saying no change is needed (no Apply).

use std::collections::BTreeMap;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use super::{Metric, MetricRow, WeekStatus};
use crate::coach::config::{Intensity, MovementVersion};
use crate::coach::water::{MAX_SERVINGS, Servings};

/// An amount must be logged this often in the week to suggest a button for it.
pub const SERVING_MIN_LOGS: u32 = 3;
/// Routines in one version before suggesting it as the default.
pub const VERSION_MIN_ROUTINES: u32 = 3;
/// Breaks offered before suggesting lighter reminders.
pub const LIGHTER_MIN_OFFERED: u32 = 10;
/// Break follow-through, in percent, for "keep your current timing".
pub const KEEP_MIN_PCT: u32 = 75;
/// At most this many suggestions.
pub const MAX_SUGGESTIONS: usize = 3;
/// Suggestion records kept, in weeks.
pub const WEEKS_KEPT: usize = 8;

/// Which suggestion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionKind {
    /// Add a serving button.
    Serving,
    /// Seated as the default movement version.
    Seated,
    /// Standing as the default movement version.
    Standing,
    /// One step lighter reminders.
    Lighter,
    /// No change needed (information only).
    Keep,
}

/// What the week's own data says, for the suggestions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SuggestFacts {
    /// How often each water amount was logged (current entries).
    pub water_amounts: BTreeMap<u32, u32>,
    /// Routines done or partly done seated.
    pub seated: u32,
    /// Routines done or partly done standing.
    pub standing: u32,
    /// Movement and eye-rest breaks offered (snoozes not counted).
    pub offered: u32,
    /// Of those, skipped, dismissed or left unanswered.
    pub set_aside: u32,
}

/// The settings a suggestion may change, as they are now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Current {
    /// The serving buttons.
    pub servings: Servings,
    /// The default movement version.
    pub movement_version: MovementVersion,
    /// The reminder intensity.
    pub intensity: Intensity,
}

/// A change to one setting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "setting", content = "value")]
pub enum Change {
    /// The serving buttons.
    Servings(Servings),
    /// The default movement version.
    MovementVersion(MovementVersion),
    /// The reminder intensity.
    Intensity(Intensity),
}

impl Change {
    /// The same setting as it is in `current`.
    #[must_use]
    pub fn current_of(&self, current: &Current) -> Self {
        match self {
            Self::Servings(_) => Self::Servings(current.servings.clone()),
            Self::MovementVersion(_) => Self::MovementVersion(current.movement_version),
            Self::Intensity(_) => Self::Intensity(current.intensity),
        }
    }

    /// The setting and its value in words.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::Servings(s) => {
                let list: Vec<String> = s.ml().iter().map(u32::to_string).collect();
                format!("serving buttons {} ml", list.join(", "))
            }
            Self::MovementVersion(v) => format!(
                "default movement version: {}",
                match v {
                    MovementVersion::Ask => "ask each time",
                    MovementVersion::Seated => "seated",
                    MovementVersion::Standing => "standing",
                }
            ),
            Self::Intensity(i) => format!(
                "reminders: {}",
                match i {
                    Intensity::Light => "Light",
                    Intensity::Balanced => "Balanced",
                    Intensity::MoreOften => "More often",
                }
            ),
        }
    }
}

/// A suggestion the rules give for a week.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposal {
    /// Which one.
    pub kind: SuggestionKind,
    /// The one-line question.
    pub text: String,
    /// What Apply sets (`None` for the information-only line).
    pub change: Option<Change>,
}

/// Where a suggestion stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum SuggestionState {
    /// Apply or Not for me.
    Open,
    /// Information only ("No change needed").
    Info,
    /// Applied: what changed; Undo restores the prior value.
    Applied {
        /// The setting now, in words.
        now: String,
        /// The setting before, in words.
        was: String,
    },
    /// "Not for me".
    Hidden,
}

/// A suggestion as the review shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Suggestion {
    /// Which one (echo it back to Apply, Undo or Not for me).
    pub kind: SuggestionKind,
    /// The one-line text.
    pub text: String,
    /// Where it stands.
    #[serde(flatten)]
    pub state: SuggestionState,
}

/// An applied suggestion, with the exact prior value for Undo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Applied {
    /// Which one.
    pub kind: SuggestionKind,
    /// Its text when applied.
    pub text: String,
    /// The value before.
    pub was: Change,
    /// The value set.
    pub now: Change,
}

/// One week's answers.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeekAnswers {
    /// The week (its Monday).
    pub week: NaiveDate,
    /// Applied suggestions.
    pub applied: Vec<Applied>,
    /// Suggestions answered "Not for me", with their text.
    pub hidden: Vec<(SuggestionKind, String)>,
}

/// The answers of recent weeks (settings key [`ANSWERS_KEY`]).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Answers {
    /// Newest last, at most [`WEEKS_KEPT`].
    pub weeks: Vec<WeekAnswers>,
}

/// Settings key for [`Answers`].
pub const ANSWERS_KEY: &str = "review_suggestions";

impl Answers {
    /// The answers for `week`, if any.
    #[must_use]
    pub fn week(&self, week: NaiveDate) -> Option<&WeekAnswers> {
        self.weeks.iter().find(|w| w.week == week)
    }

    /// The answers for `week`, created if needed (the oldest weeks are dropped).
    pub fn week_mut(&mut self, week: NaiveDate) -> &mut WeekAnswers {
        if let Some(i) = self.weeks.iter().position(|w| w.week == week) {
            return &mut self.weeks[i];
        }
        self.weeks.push(WeekAnswers {
            week,
            ..WeekAnswers::default()
        });
        self.weeks.sort_by_key(|w| w.week);
        while self.weeks.len() > WEEKS_KEPT {
            self.weeks.remove(0);
        }
        let i = self
            .weeks
            .iter()
            .position(|w| w.week == week)
            .unwrap_or(self.weeks.len().saturating_sub(1));
        &mut self.weeks[i]
    }
}

const fn lighter(i: Intensity) -> Option<Intensity> {
    match i {
        Intensity::MoreOften => Some(Intensity::Balanced),
        Intensity::Balanced => Some(Intensity::Light),
        Intensity::Light => None,
    }
}

/// The suggestions the rules give for a week, in a fixed order, at most
/// [`MAX_SUGGESTIONS`]. None without enough data.
#[must_use]
pub fn proposals(
    status: WeekStatus,
    rows: &[MetricRow],
    facts: &SuggestFacts,
    current: &Current,
) -> Vec<Proposal> {
    if matches!(status, WeekStatus::BeforeStart | WeekStatus::NotEnoughData) {
        return Vec::new();
    }
    let mut out = Vec::new();
    // The most-logged amount that is not a button (the smaller amount on a tie).
    let common = facts
        .water_amounts
        .iter()
        .filter(|(ml, n)| **n >= SERVING_MIN_LOGS && !current.servings.contains(**ml))
        .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))
        .map(|(ml, _)| *ml);
    if let Some(ml) = common
        && current.servings.ml().len() < MAX_SERVINGS
    {
        let mut list = current.servings.ml().to_vec();
        list.push(ml);
        if let Ok(s) = Servings::new(list) {
            out.push(Proposal {
                kind: SuggestionKind::Serving,
                text: format!("Add a {ml} ml button so logging takes one tap?"),
                change: Some(Change::Servings(s)),
            });
        }
    }
    if current.movement_version == MovementVersion::Ask {
        if facts.seated >= VERSION_MIN_ROUTINES && facts.standing == 0 {
            out.push(Proposal {
                kind: SuggestionKind::Seated,
                text: "Set the seated version as your default, so you skip one choice each time?"
                    .to_owned(),
                change: Some(Change::MovementVersion(MovementVersion::Seated)),
            });
        } else if facts.standing >= VERSION_MIN_ROUTINES && facts.seated == 0 {
            out.push(Proposal {
                kind: SuggestionKind::Standing,
                text: "Set the standing version as your default, so you skip one choice each time?"
                    .to_owned(),
                change: Some(Change::MovementVersion(MovementVersion::Standing)),
            });
        }
    }
    if facts.offered >= LIGHTER_MIN_OFFERED
        && facts.set_aside.saturating_mul(2) > facts.offered
        && let Some(next) = lighter(current.intensity)
    {
        out.push(Proposal {
            kind: SuggestionKind::Lighter,
            text: format!(
                "{} of {} breaks were set aside. Try fewer reminders next week?",
                facts.set_aside, facts.offered
            ),
            change: Some(Change::Intensity(next)),
        });
    }
    let breaks_pct = rows
        .iter()
        .find(|r| r.metric == Metric::Breaks)
        .and_then(|r| r.value.pct);
    if out.is_empty()
        && status == WeekStatus::Normal
        && breaks_pct.is_some_and(|p| p >= KEEP_MIN_PCT)
    {
        out.push(Proposal {
            kind: SuggestionKind::Keep,
            text: "Keep your current reminder timing. It suited you this week.".to_owned(),
            change: None,
        });
    }
    out.truncate(MAX_SUGGESTIONS);
    out
}

/// The suggestions as shown: the week's applied and hidden ones as answered, then the
/// open ones the rules give now, at most [`MAX_SUGGESTIONS`].
#[must_use]
pub fn shown(proposals: &[Proposal], answers: Option<&WeekAnswers>) -> Vec<Suggestion> {
    let mut out: Vec<Suggestion> = Vec::new();
    if let Some(a) = answers {
        for x in &a.applied {
            out.push(Suggestion {
                kind: x.kind,
                text: x.text.clone(),
                state: SuggestionState::Applied {
                    now: x.now.describe(),
                    was: x.was.describe(),
                },
            });
        }
        for (kind, text) in &a.hidden {
            out.push(Suggestion {
                kind: *kind,
                text: text.clone(),
                state: SuggestionState::Hidden,
            });
        }
    }
    for p in proposals {
        if out.iter().any(|s| s.kind == p.kind) {
            continue;
        }
        out.push(Suggestion {
            kind: p.kind,
            text: p.text.clone(),
            state: if p.change.is_some() {
                SuggestionState::Open
            } else {
                SuggestionState::Info
            },
        });
    }
    out.sort_by_key(|s| s.kind);
    out.truncate(MAX_SUGGESTIONS);
    out
}
