//! The injectable clock (architecture 4.3: "the engine runs on an injected clock").
//!
//! Two kinds of time are kept apart on purpose:
//!
//! - **Wall time** ([`Clock::now`]) is the PC's UTC clock. It stamps events and decides
//!   the user's local date and work hours. It can jump: the user changes the clock, NTP
//!   corrects it, or the PC resumes from sleep.
//! - **Monotonic time** ([`Clock::monotonic`]) only moves forward. Timers and durations
//!   (the 45-minute movement timer, snooze, cooldown, grace periods) are measured with it,
//!   so a wall-clock jump can neither fire a burst of reminders nor silently cancel one.
//!
//! Clock-change policy for V1 (D, documented here so every slice follows it):
//!
//! 1. An event keeps the wall time, IANA zone and local date it was recorded with. A later
//!    clock or time-zone change never rewrites them (PRD section 8; architecture 6.1).
//! 2. "Today" always follows the current wall clock in the user's zone. If the clock is
//!    set back across midnight, today becomes the earlier date again; events already
//!    stored keep their dates.
//! 3. Event identifiers stay strictly increasing even when the wall clock goes backwards
//!    (see [`crate::ids`]).
//! 4. [`ClockWatch`] reports a wall-clock change whenever wall and monotonic progress
//!    disagree by more than a tolerance, so the tracker (V1-2) can re-evaluate work hours
//!    and day rollover at once. A forward discrepancy can also be a suspend on platforms
//!    whose monotonic clock stops during sleep; the tracker tells the two apart with the
//!    Windows power events it already receives.

use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration as StdDuration, Instant};

use chrono::{DateTime, Duration, Utc};

/// A source of wall and monotonic time.
pub trait Clock: Send + Sync + fmt::Debug {
    /// The current wall-clock time in UTC. May jump in either direction.
    fn now(&self) -> DateTime<Utc>;

    /// Time elapsed since an arbitrary fixed origin. Never decreases.
    fn monotonic(&self) -> StdDuration;
}

impl<C: Clock + ?Sized> Clock for Arc<C> {
    fn now(&self) -> DateTime<Utc> {
        (**self).now()
    }

    fn monotonic(&self) -> StdDuration {
        (**self).monotonic()
    }
}

impl<C: Clock + ?Sized> Clock for &C {
    fn now(&self) -> DateTime<Utc> {
        (**self).now()
    }

    fn monotonic(&self) -> StdDuration {
        (**self).monotonic()
    }
}

/// The real clock: `Utc::now()` and [`Instant`].
#[derive(Debug, Clone, Copy)]
pub struct SystemClock {
    origin: Instant,
}

impl SystemClock {
    /// A system clock whose monotonic origin is now.
    #[must_use]
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }

    fn monotonic(&self) -> StdDuration {
        self.origin.elapsed()
    }
}

#[derive(Debug, Clone, Copy)]
struct ManualState {
    wall: DateTime<Utc>,
    mono: StdDuration,
}

/// A clock moved only by the test, for deterministic trace replay.
#[derive(Debug)]
pub struct ManualClock {
    state: Mutex<ManualState>,
}

impl ManualClock {
    /// A manual clock showing `wall`, with monotonic time at zero.
    #[must_use]
    pub fn new(wall: DateTime<Utc>) -> Self {
        Self {
            state: Mutex::new(ManualState {
                wall,
                mono: StdDuration::ZERO,
            }),
        }
    }

    fn with<R>(&self, f: impl FnOnce(&mut ManualState) -> R) -> R {
        // A poisoned lock only means another test thread panicked; the state is still usable.
        let mut guard = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        f(&mut guard)
    }

    /// Let real time pass: wall and monotonic time both move forward by `by`.
    ///
    /// # Panics
    /// If `by` is negative: real time never runs backwards. Use [`ManualClock::set_wall`]
    /// to simulate a clock change.
    pub fn advance(&self, by: Duration) {
        let std = by
            .to_std()
            .unwrap_or_else(|_| panic!("ManualClock::advance needs a non-negative duration"));
        self.with(|s| {
            s.wall += by;
            s.mono += std;
        });
    }

    /// Change only the wall clock (a user or NTP clock change). Monotonic time is unchanged.
    pub fn set_wall(&self, wall: DateTime<Utc>) {
        self.with(|s| s.wall = wall);
    }

    /// Shift only the wall clock by `by` (positive or negative).
    pub fn shift_wall(&self, by: Duration) {
        self.with(|s| s.wall += by);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> DateTime<Utc> {
        self.with(|s| s.wall)
    }

    fn monotonic(&self) -> StdDuration {
        self.with(|s| s.mono)
    }
}

/// What [`ClockWatch::observe`] saw since the previous observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockObservation {
    /// Wall and monotonic time advanced together (within the tolerance).
    Steady {
        /// Monotonic time elapsed since the previous observation.
        elapsed: StdDuration,
    },
    /// The wall clock moved differently from monotonic time.
    WallChanged {
        /// Monotonic time elapsed since the previous observation.
        elapsed: StdDuration,
        /// Wall progress minus monotonic progress: positive if the clock jumped forward,
        /// negative if it was set back.
        skew: Duration,
    },
}

/// Detects wall-clock changes by comparing wall progress with monotonic progress.
#[derive(Debug, Clone)]
pub struct ClockWatch {
    last_wall: DateTime<Utc>,
    last_mono: StdDuration,
    tolerance: Duration,
}

impl ClockWatch {
    /// Default tolerance: 2 seconds, well above timer jitter and far below any
    /// user-visible clock change.
    pub const DEFAULT_TOLERANCE: Duration = Duration::seconds(2);

    /// Start watching from the clock's current reading.
    #[must_use]
    pub fn new(clock: &dyn Clock, tolerance: Duration) -> Self {
        Self {
            last_wall: clock.now(),
            last_mono: clock.monotonic(),
            tolerance: tolerance.abs(),
        }
    }

    /// Compare progress since the previous observation, then take the current reading as
    /// the new reference.
    pub fn observe(&mut self, clock: &dyn Clock) -> ClockObservation {
        let wall = clock.now();
        let mono = clock.monotonic();
        let elapsed = mono.saturating_sub(self.last_mono);
        let wall_progress = wall - self.last_wall;
        let mono_progress = Duration::from_std(elapsed).unwrap_or(Duration::MAX);
        let skew = wall_progress - mono_progress;
        self.last_wall = wall;
        self.last_mono = mono;
        if skew.abs() > self.tolerance {
            ClockObservation::WallChanged { elapsed, skew }
        } else {
            ClockObservation::Steady { elapsed }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 21, 4, 0, 0).unwrap()
    }

    #[test]
    fn manual_clock_advances_wall_and_monotonic_together() {
        let c = ManualClock::new(t0());
        c.advance(Duration::minutes(45));
        assert_eq!(c.now(), t0() + Duration::minutes(45));
        assert_eq!(c.monotonic(), StdDuration::from_secs(45 * 60));
    }

    #[test]
    fn manual_clock_wall_change_leaves_monotonic_alone() {
        let c = ManualClock::new(t0());
        c.advance(Duration::seconds(10));
        c.set_wall(t0() - Duration::hours(3));
        assert_eq!(c.now(), t0() - Duration::hours(3));
        assert_eq!(c.monotonic(), StdDuration::from_secs(10));
        c.shift_wall(Duration::hours(1));
        assert_eq!(c.now(), t0() - Duration::hours(2));
        assert_eq!(c.monotonic(), StdDuration::from_secs(10));
    }

    #[test]
    #[should_panic(expected = "non-negative")]
    fn manual_clock_refuses_negative_advance() {
        ManualClock::new(t0()).advance(Duration::seconds(-1));
    }

    #[test]
    fn clock_is_usable_through_arc_and_reference() {
        let c = Arc::new(ManualClock::new(t0()));
        let dyn_clock: Arc<dyn Clock> = c.clone();
        c.advance(Duration::seconds(5));
        assert_eq!(dyn_clock.now(), t0() + Duration::seconds(5));
        let by_ref: &dyn Clock = &*c;
        assert_eq!((&by_ref).monotonic(), StdDuration::from_secs(5));
    }

    #[test]
    fn system_clock_monotonic_never_decreases() {
        let c = SystemClock::new();
        let a = c.monotonic();
        let b = c.monotonic();
        assert!(b >= a);
        let drift = (c.now() - Utc::now()).abs();
        assert!(drift < Duration::seconds(5));
    }

    #[test]
    fn watch_reports_steady_progress() {
        let c = ManualClock::new(t0());
        let mut w = ClockWatch::new(&c, ClockWatch::DEFAULT_TOLERANCE);
        c.advance(Duration::seconds(5));
        assert_eq!(
            w.observe(&c),
            ClockObservation::Steady {
                elapsed: StdDuration::from_secs(5)
            }
        );
        // Jitter inside the tolerance is still steady.
        c.advance(Duration::seconds(5));
        c.shift_wall(Duration::milliseconds(1500));
        assert!(matches!(w.observe(&c), ClockObservation::Steady { .. }));
    }

    #[test]
    fn watch_reports_backward_clock_change() {
        let c = ManualClock::new(t0());
        let mut w = ClockWatch::new(&c, ClockWatch::DEFAULT_TOLERANCE);
        c.advance(Duration::seconds(5));
        c.shift_wall(Duration::hours(-1));
        assert_eq!(
            w.observe(&c),
            ClockObservation::WallChanged {
                elapsed: StdDuration::from_secs(5),
                skew: Duration::hours(-1),
            }
        );
        // The new reading becomes the reference: the next tick is steady again.
        c.advance(Duration::seconds(5));
        assert!(matches!(w.observe(&c), ClockObservation::Steady { .. }));
    }

    #[test]
    fn watch_reports_forward_clock_change() {
        let c = ManualClock::new(t0());
        let mut w = ClockWatch::new(&c, Duration::seconds(-2)); // tolerance sign is ignored
        c.advance(Duration::seconds(5));
        c.shift_wall(Duration::minutes(30));
        assert_eq!(
            w.observe(&c),
            ClockObservation::WallChanged {
                elapsed: StdDuration::from_secs(5),
                skew: Duration::minutes(30),
            }
        );
    }
}
