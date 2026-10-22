//! [`Coach`]: the engine bound to the store and the injected clock. This is what the shell
//! calls.
//!
//! - Every input is stamped with the store's clock: wall time for dates and work hours,
//!   monotonic time for the elapsed interval. On [`Input::Resume`] the elapsed interval is
//!   the larger of monotonic and wall progress, because Windows' monotonic clock may not
//!   advance during sleep (see [`crate::clock`]).
//! - Each step's new state, its events and its tallies are committed in one transaction
//!   with a compare-and-set on the state's revision. If the commit fails, the in-memory
//!   engine is rolled back to the last committed state and the error is returned, so
//!   memory and disk never disagree. An [`Error::Stale`] means another writer moved the
//!   state on: reopen the coach.
//! - Steps that only add counted time are saved at most once a minute; anything else
//!   (an offer, an answer, a hold change) is saved at once. [`Coach::flush`] saves on exit.
//! - On open, the time since the last save counts like a sleep (a confirmed break if
//!   2 minutes or more). If the wall clock was set back while the app was closed, that
//!   time is taken as zero: no reset and no burst. A saved state this build cannot read
//!   is replaced by a fresh one (keeping the revision chain) with [`Notice::StateReset`].

use std::time::Duration;

use chrono::{DateTime, NaiveDate, Utc};

use super::config::{CONFIG_KEY, CoachConfig};
use super::engine::{Context, DayTally, Engine, EngineState, STATE_FORMAT, StepOutput};
use super::types::{CoachView, Input, Notice};
use super::water::WaterSnapshot;
use crate::error::{Error, Result};
use crate::schema::{GoalKind, NewEvent};
use crate::store::{CoachCommit, Store};

/// How often a step that only adds counted time is saved.
pub const SAVE_EVERY: Duration = Duration::from_secs(60);

/// The coaching engine bound to the local store.
#[derive(Debug)]
pub struct Coach {
    store: Store,
    engine: Engine,
    revision: Option<u64>,
    last_mono: Duration,
    last_wall: DateTime<Utc>,
    last_save_mono: Duration,
    pending_days: Vec<(NaiveDate, DayTally)>,
    unsaved: bool,
    view: CoachView,
}

fn new_salt() -> u64 {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).expect("operating system random source failed");
    u64::from_be_bytes(bytes)
}

impl Coach {
    /// Load the saved engine state (or start fresh), apply the restart rules and save.
    /// Returns the coach and the first step's output.
    ///
    /// # Errors
    /// [`Error::Json`] or [`Error::Invalid`] if the saved state cannot be read, or any
    /// store error.
    pub fn open(store: Store) -> Result<(Self, StepOutput)> {
        let wall = store.clock().now();
        let mono = store.clock().monotonic();
        let saved = store.coach_state()?;
        let mut reset = false;
        let (mut engine, revision, gap) = match saved {
            Some(s) => {
                // Time the app was not running; zero if the clock was set back meanwhile.
                let gap = (wall - s.saved_at).to_std().unwrap_or(Duration::ZERO);
                let engine = serde_json::from_str::<EngineState>(&s.state)
                    .map_err(Error::from)
                    .and_then(Engine::from_state)
                    .unwrap_or_else(|_| {
                        // An unreadable state must not stop coaching: start fresh, keep
                        // the revision chain, and say so.
                        reset = true;
                        Engine::new_continuing(new_salt(), Duration::from_millis(s.engine_time_ms))
                    });
                (engine, Some(s.revision), gap)
            }
            None => (Engine::new(new_salt()), None, Duration::ZERO),
        };
        let ctx = context(&store, wall, gap)?;
        let mut out = if revision.is_some() && !reset {
            engine.restart(&ctx)
        } else {
            engine.step(&ctx, Input::Refresh)
        };
        if reset {
            out.notices.push(Notice::StateReset);
        }
        let mut coach = Self {
            store,
            engine,
            revision,
            last_mono: mono,
            last_wall: wall,
            last_save_mono: mono,
            pending_days: out.touched_days.clone(),
            unsaved: true,
            view: out.view.clone(),
        };
        coach.commit(&out.events)?;
        coach.after_commit(&out)?;
        Ok((coach, out))
    }

    /// The 5-second poll.
    ///
    /// # Errors
    /// See [`Coach::handle`].
    pub fn tick(&mut self, idle: Duration) -> Result<StepOutput> {
        self.handle(Input::Tick { idle })
    }

    /// Apply one input now.
    ///
    /// # Errors
    /// Any store error; the engine is then rolled back to the last saved state.
    pub fn handle(&mut self, input: Input) -> Result<StepOutput> {
        let wall = self.store.clock().now();
        let mono = self.store.clock().monotonic();
        let mono_elapsed = mono.saturating_sub(self.last_mono);
        let elapsed = if input == Input::Resume {
            let wall_elapsed = (wall - self.last_wall).to_std().unwrap_or(Duration::ZERO);
            mono_elapsed.max(wall_elapsed)
        } else {
            mono_elapsed
        };
        let ctx = context(&self.store, wall, elapsed)?;
        let before = (self.engine.clone(), self.pending_days.clone(), self.unsaved);
        let out = self.engine.step(&ctx, input);
        merge_days(&mut self.pending_days, &out.touched_days);
        self.unsaved = true;
        let due = mono.saturating_sub(self.last_save_mono) >= SAVE_EVERY;
        if out.significant || !out.events.is_empty() || due {
            if let Err(e) = self.commit(&out.events) {
                (self.engine, self.pending_days, self.unsaved) = before;
                return Err(e);
            }
            self.last_save_mono = mono;
        }
        self.last_mono = mono;
        self.last_wall = wall;
        self.view = out.view.clone();
        self.after_commit(&out)?;
        Ok(out)
    }

    /// Save anything not yet saved (call on exit).
    ///
    /// # Errors
    /// Any store error.
    pub fn flush(&mut self) -> Result<()> {
        if !self.unsaved {
            return Ok(());
        }
        self.commit(&[])
    }

    fn commit(&mut self, events: &[NewEvent]) -> Result<()> {
        let state = serde_json::to_string(self.engine.state())?;
        let rev = self.store.commit_coach_step(CoachCommit {
            expected_revision: self.revision,
            format: STATE_FORMAT,
            engine_time_ms: engine_ms(&self.engine),
            state: &state,
            events,
            days: &self.pending_days,
        })?;
        self.revision = Some(rev);
        self.pending_days.clear();
        self.unsaved = false;
        Ok(())
    }

    fn after_commit(&mut self, out: &StepOutput) -> Result<()> {
        for n in &out.notices {
            if let Notice::UseIntensity(i) = n {
                let mut cfg = self.config()?;
                cfg.intensity = *i;
                self.store.put_setting(CONFIG_KEY, &cfg)?;
            }
        }
        Ok(())
    }

    /// The current coaching settings.
    ///
    /// # Errors
    /// [`Error::Json`] if the stored settings are malformed.
    pub fn config(&self) -> Result<CoachConfig> {
        Ok(self.store.setting(CONFIG_KEY)?.unwrap_or_default())
    }

    /// Change the coaching settings and re-evaluate at once.
    ///
    /// # Errors
    /// [`Error::Invalid`] if the settings break a rule, or any store error.
    pub fn set_config(&mut self, config: &CoachConfig) -> Result<StepOutput> {
        config.validate()?;
        self.store.put_setting(CONFIG_KEY, config)?;
        self.handle(Input::Refresh)
    }

    /// The last view.
    #[must_use]
    pub fn view(&self) -> &CoachView {
        &self.view
    }

    /// The engine (read-only).
    #[must_use]
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// The revision of the last saved state.
    #[must_use]
    pub fn revision(&self) -> Option<u64> {
        self.revision
    }

    /// The store.
    #[must_use]
    pub fn store(&self) -> &Store {
        &self.store
    }

    /// The store, for screens that log water or change settings. Call
    /// [`Coach::handle`] with [`Input::Refresh`] afterwards to update the view.
    pub fn store_mut(&mut self) -> &mut Store {
        &mut self.store
    }

    /// Save, then give the store back.
    ///
    /// # Errors
    /// Any store error from the final save.
    pub fn close(mut self) -> Result<Store> {
        self.flush()?;
        Ok(self.store)
    }
}

fn engine_ms(e: &Engine) -> u64 {
    u64::try_from(e.engine_time().as_millis()).unwrap_or(u64::MAX)
}

fn merge_days(pending: &mut Vec<(NaiveDate, DayTally)>, new: &[(NaiveDate, DayTally)]) {
    for (d, t) in new {
        match pending.iter_mut().find(|(pd, _)| pd == d) {
            Some(slot) => slot.1 = *t,
            None => pending.push((*d, *t)),
        }
    }
}

fn context(store: &Store, wall: DateTime<Utc>, elapsed: Duration) -> Result<Context> {
    let zone = store.user_zone()?;
    let today = zone.local_date(wall);
    let config: CoachConfig = store.setting(CONFIG_KEY)?.unwrap_or_default();
    if config.validate().is_err() {
        return Err(Error::invalid("stored coaching settings are invalid"));
    }
    Ok(Context {
        wall,
        elapsed,
        zone,
        schedule: store.work_schedule()?,
        water: WaterSnapshot {
            logged_ml: store.water_total_on(today)?,
            goal_ml: store.goal_on(GoalKind::WaterDailyMl, today)?,
        },
        config,
    })
}
