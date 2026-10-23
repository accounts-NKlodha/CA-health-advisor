//! Settings the V1-4 screens change (wireframes W4 to W7, W12 and the weekly review's
//! suggestions), kept in the settings table like every other setting.

use chrono::{Days, NaiveDate};
use rusqlite::params;

use super::{Store, date_text};
use crate::coach::config::{CONFIG_KEY, CoachConfig};
use crate::coach::water::{SERVINGS_KEY, Servings};
use crate::error::{Error, Result};
use crate::prefs::{ACCESSIBILITY_KEY, Accessibility};
use crate::review::suggest::{
    self, ANSWERS_KEY, Answers, Applied, Change, Current, SuggestFacts, SuggestionKind,
};
use crate::review::{WeekReview, week_start};
use crate::schema::{Activity, EventPayload, ReminderOutcome, RoutineOutcome, RoutineVariant};

impl Store {
    /// The coaching settings (defaults until changed).
    ///
    /// # Errors
    /// [`Error::Json`] if the stored settings are malformed.
    pub fn coach_config(&self) -> Result<CoachConfig> {
        Ok(self.setting(CONFIG_KEY)?.unwrap_or_default())
    }

    /// Save the coaching settings. The engine reads them at its next step; send it a
    /// refresh to apply them at once.
    ///
    /// # Errors
    /// [`Error::Invalid`] if they break a rule.
    pub fn set_coach_config(&self, config: &CoachConfig) -> Result<()> {
        config.validate()?;
        self.put_setting(CONFIG_KEY, config)
    }

    /// The serving buttons (defaults until changed).
    ///
    /// # Errors
    /// [`Error::Json`] if the stored list breaks the rules.
    pub fn water_servings(&self) -> Result<Servings> {
        Ok(self.setting(SERVINGS_KEY)?.unwrap_or_default())
    }

    /// Save the serving buttons.
    ///
    /// # Errors
    /// [`Error::Sqlite`] on a write failure.
    pub fn set_water_servings(&self, servings: &Servings) -> Result<()> {
        self.put_setting(SERVINGS_KEY, servings)
    }

    /// The display preferences (defaults until changed).
    ///
    /// # Errors
    /// [`Error::Json`] if the stored preferences are malformed.
    pub fn accessibility(&self) -> Result<Accessibility> {
        Ok(self.setting(ACCESSIBILITY_KEY)?.unwrap_or_default())
    }

    /// Save the display preferences.
    ///
    /// # Errors
    /// [`Error::Invalid`] if they break a rule.
    pub fn set_accessibility(&self, a: &Accessibility) -> Result<()> {
        a.validate()?;
        self.put_setting(ACCESSIBILITY_KEY, a)
    }

    // ------------------------------------------------------------- suggestions

    /// What the week's own records say, for the suggestions (current entries only).
    ///
    /// # Errors
    /// A read failure.
    pub fn suggest_facts(&self, from: NaiveDate, to: NaiveDate) -> Result<SuggestFacts> {
        let mut stmt = self.conn.prepare(
            "SELECT e.type, e.payload FROM events e
             WHERE e.local_date BETWEEN ?1 AND ?2
               AND e.type IN ('water.logged', 'movement.session', 'reminder.response')
               AND NOT EXISTS (SELECT 1 FROM events s WHERE s.supersedes = e.id)",
        )?;
        let rows = stmt
            .query_map(params![date_text(from), date_text(to)], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut f = SuggestFacts::default();
        for (type_name, payload) in rows {
            let Ok(payload) = EventPayload::from_stored(&type_name, &payload) else {
                continue;
            };
            match payload {
                EventPayload::WaterLogged { amount_ml } => {
                    *f.water_amounts.entry(amount_ml).or_default() += 1;
                }
                EventPayload::MovementSession {
                    variant: Some(v),
                    outcome: RoutineOutcome::Done | RoutineOutcome::Partial,
                    ..
                } => match v {
                    RoutineVariant::Seated => f.seated += 1,
                    RoutineVariant::Standing => f.standing += 1,
                },
                EventPayload::ReminderResponse {
                    activity: Activity::Movement | Activity::EyeRest,
                    outcome,
                    ..
                } if outcome != ReminderOutcome::Snoozed => {
                    f.offered += 1;
                    if matches!(
                        outcome,
                        ReminderOutcome::Skipped
                            | ReminderOutcome::Dismissed
                            | ReminderOutcome::NoResponse
                    ) {
                        f.set_aside += 1;
                    }
                }
                _ => {}
            }
        }
        Ok(f)
    }

    /// The settings a suggestion may change, as they are now.
    ///
    /// # Errors
    /// A read failure.
    pub fn suggestion_settings(&self) -> Result<Current> {
        let config = self.coach_config()?;
        Ok(Current {
            servings: self.water_servings()?,
            movement_version: config.movement_version,
            intensity: config.intensity,
        })
    }

    fn suggestion_answers(&self) -> Result<Answers> {
        Ok(self.setting(ANSWERS_KEY)?.unwrap_or_default())
    }

    /// Whether `week` gets suggestions: this week and the previous one ("for next week").
    fn suggests_for(&self, week: NaiveDate) -> Result<bool> {
        let this_week = week_start(self.today()?);
        Ok(week == this_week || this_week.checked_sub_days(Days::new(7)) == Some(week))
    }

    /// The rules' suggestions for the reviewed week.
    fn proposals_for(&self, review: &WeekReview) -> Result<Vec<suggest::Proposal>> {
        if !self.suggests_for(review.start)? {
            return Ok(Vec::new());
        }
        let facts = self.suggest_facts(review.start, review.end)?;
        Ok(suggest::proposals(
            review.status,
            &review.rows,
            &facts,
            &self.suggestion_settings()?,
        ))
    }

    /// Add the "Try next week" suggestions to a weekly review.
    ///
    /// # Errors
    /// A read failure.
    pub fn add_suggestions(&self, review: &mut WeekReview) -> Result<()> {
        let proposals = self.proposals_for(review)?;
        let answers = self.suggestion_answers()?;
        review.suggestions = if self.suggests_for(review.start)? {
            suggest::shown(&proposals, answers.week(review.start))
        } else {
            Vec::new()
        };
        Ok(())
    }

    fn apply_change(&self, change: &Change) -> Result<()> {
        match change {
            Change::Servings(s) => self.set_water_servings(s),
            Change::MovementVersion(v) => {
                let mut c = self.coach_config()?;
                c.movement_version = *v;
                self.set_coach_config(&c)
            }
            Change::Intensity(i) => {
                let mut c = self.coach_config()?;
                c.intensity = *i;
                self.set_coach_config(&c)
            }
        }
    }

    /// "Apply" on a suggestion of the week starting `week`: change the setting and keep
    /// its exact prior value for Undo. The engine reads the settings at its next step.
    ///
    /// # Errors
    /// [`Error::Invalid`] if the week has no such open suggestion now.
    pub fn apply_suggestion(&mut self, week: NaiveDate, kind: SuggestionKind) -> Result<()> {
        let review = self.week_review(Some(week))?;
        if review.start != week {
            return Err(Error::invalid("that week has no suggestions"));
        }
        let mut answers = self.suggestion_answers()?;
        if answers
            .week(week)
            .is_some_and(|a| a.applied.iter().any(|x| x.kind == kind))
        {
            return Err(Error::invalid("that suggestion is already applied"));
        }
        let proposal = self
            .proposals_for(&review)?
            .into_iter()
            .find(|p| p.kind == kind)
            .ok_or_else(|| Error::invalid("that suggestion no longer applies"))?;
        let now = proposal
            .change
            .ok_or_else(|| Error::invalid("that line changes nothing"))?;
        let was = now.current_of(&self.suggestion_settings()?);
        let a = answers.week_mut(week);
        a.hidden.retain(|(k, _)| *k != kind);
        a.applied.push(Applied {
            kind,
            text: proposal.text,
            was,
            now: now.clone(),
        });
        let tx = self.conn.unchecked_transaction()?;
        self.apply_change(&now)?;
        self.put_setting(ANSWERS_KEY, &answers)?;
        tx.commit()?;
        Ok(())
    }

    /// "Undo" on an applied suggestion: restore the exact prior value.
    ///
    /// # Errors
    /// [`Error::Invalid`] if the suggestion was not applied.
    pub fn undo_suggestion(&mut self, week: NaiveDate, kind: SuggestionKind) -> Result<()> {
        let mut answers = self.suggestion_answers()?;
        let a = answers.week_mut(week);
        let i = a
            .applied
            .iter()
            .position(|x| x.kind == kind)
            .ok_or_else(|| Error::invalid("that suggestion was not applied"))?;
        let applied = a.applied.remove(i);
        let tx = self.conn.unchecked_transaction()?;
        self.apply_change(&applied.was)?;
        self.put_setting(ANSWERS_KEY, &answers)?;
        tx.commit()?;
        Ok(())
    }

    /// "Not for me" on an open suggestion: it is not offered again for that week.
    ///
    /// # Errors
    /// [`Error::Invalid`] if the week has no such open suggestion.
    pub fn hide_suggestion(&mut self, week: NaiveDate, kind: SuggestionKind) -> Result<()> {
        let review = self.week_review(Some(week))?;
        let proposal = self
            .proposals_for(&review)?
            .into_iter()
            .find(|p| p.kind == kind && p.change.is_some())
            .ok_or_else(|| Error::invalid("that suggestion no longer applies"))?;
        let mut answers = self.suggestion_answers()?;
        let a = answers.week_mut(review.start);
        if a.applied.iter().any(|x| x.kind == kind) {
            return Err(Error::invalid("undo it first"));
        }
        if !a.hidden.iter().any(|(k, _)| *k == kind) {
            a.hidden.push((kind, proposal.text));
        }
        self.put_setting(ANSWERS_KEY, &answers)
    }
}
