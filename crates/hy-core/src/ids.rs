//! Event identifiers: UUIDv7 (architecture 6.1), strictly increasing within a process.
//!
//! The 48-bit millisecond timestamp comes from the injected wall clock. The 12-bit
//! `rand_a` field holds a counter and the remaining 62 bits are random. When the wall
//! clock stalls or goes backwards (clock change, see [`crate::clock`]), the generator
//! keeps the last timestamp and increments the counter, so identifiers still sort in
//! creation order and never collide.

use std::sync::{Mutex, PoisonError};

use chrono::{DateTime, Utc};
use uuid::{Builder, Uuid};

const COUNTER_MAX: u16 = 0x0FFF;

#[derive(Debug, Default, Clone, Copy)]
struct Last {
    millis: u64,
    counter: u16,
}

/// Generates strictly increasing UUIDv7 values.
#[derive(Debug, Default)]
pub struct IdGenerator {
    last: Mutex<Option<Last>>,
}

impl IdGenerator {
    /// A new generator with no history.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The next identifier for an event recorded at wall time `now`.
    ///
    /// # Panics
    /// Only if the operating system's random source fails, which `getrandom` treats as
    /// unrecoverable.
    pub fn next(&self, now: DateTime<Utc>) -> Uuid {
        let wall_millis = u64::try_from(now.timestamp_millis()).unwrap_or(0);
        let mut guard = self.last.lock().unwrap_or_else(PoisonError::into_inner);
        let next = match *guard {
            Some(last) if wall_millis <= last.millis => {
                if last.counter >= COUNTER_MAX {
                    Last {
                        millis: last.millis + 1,
                        counter: 0,
                    }
                } else {
                    Last {
                        millis: last.millis,
                        counter: last.counter + 1,
                    }
                }
            }
            _ => Last {
                millis: wall_millis,
                counter: 0,
            },
        };
        *guard = Some(next);
        drop(guard);

        let mut bytes = [0u8; 10];
        #[allow(clippy::expect_used)]
        getrandom::fill(&mut bytes).expect("operating system random source failed");
        let [hi, lo] = next.counter.to_be_bytes();
        bytes[0] = hi & 0x0F; // the version nibble is set by the builder
        bytes[1] = lo;
        Builder::from_unix_timestamp_millis(next.millis, &bytes).into_uuid()
    }
}

/// The millisecond timestamp embedded in a UUIDv7, if `id` is one.
#[must_use]
pub fn v7_millis(id: &Uuid) -> Option<u64> {
    if id.get_version_num() != 7 {
        return None;
    }
    let b = id.as_bytes();
    Some(
        b[..6]
            .iter()
            .fold(0u64, |acc, byte| (acc << 8) | u64::from(*byte)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 21, 4, 0, 0).unwrap()
    }

    #[test]
    fn ids_are_version_7_and_carry_the_wall_time() {
        let g = IdGenerator::new();
        let id = g.next(t0());
        assert_eq!(id.get_version_num(), 7);
        assert_eq!(id.get_variant(), uuid::Variant::RFC4122);
        assert_eq!(v7_millis(&id), Some(t0().timestamp_millis() as u64));
    }

    #[test]
    fn ids_increase_within_the_same_millisecond() {
        let g = IdGenerator::new();
        let ids: Vec<Uuid> = (0..5000).map(|_| g.next(t0())).collect();
        assert!(ids.windows(2).all(|w| w[0] < w[1]));
        // The counter overflowed after 4096 ids, so the timestamp advanced by one.
        assert_eq!(
            v7_millis(ids.last().unwrap()),
            Some(t0().timestamp_millis() as u64 + 1)
        );
    }

    #[test]
    fn ids_increase_when_the_clock_goes_backwards() {
        let g = IdGenerator::new();
        let a = g.next(t0());
        let b = g.next(t0() - Duration::hours(2));
        let c = g.next(t0() - Duration::hours(1));
        assert!(a < b && b < c);
        assert_eq!(v7_millis(&b), v7_millis(&a));
        // Once the clock passes the old high-water again, the real time is used.
        let d = g.next(t0() + Duration::seconds(1));
        assert!(c < d);
        assert_eq!(
            v7_millis(&d),
            Some((t0() + Duration::seconds(1)).timestamp_millis() as u64)
        );
    }

    #[test]
    fn ids_are_unique_across_generators_in_the_same_millisecond() {
        let a = IdGenerator::new().next(t0());
        let b = IdGenerator::new().next(t0());
        assert_ne!(a, b);
    }

    #[test]
    fn v7_millis_rejects_other_versions() {
        assert_eq!(v7_millis(&Uuid::nil()), None);
    }
}
