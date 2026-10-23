//! The full personal backup over a real store (slice V1-4; architecture 8, wireframe
//! W9): export then import gives the same reviews on another PC, import replaces rather
//! than merges, tombstones are merged as a union, the consent log keeps both sides,
//! pairing and sharing are never carried over, and a refused file changes nothing.

// Test helpers outside `#[test]` functions may unwrap too.
#![allow(clippy::unwrap_used)]

use std::sync::Arc;

use chrono::{DateTime, NaiveDate, Utc};
use hy_core::Error;
use hy_core::backup::{Contents, KdfParams};
use hy_core::clock::ManualClock;
use hy_core::coach::{CoachConfig, DayTally, Intensity, Servings};
use hy_core::schema::{Category, DateRange, EventPayload, GoalKind, NewEvent};
use hy_core::store::{CoachCommit, PetProfile, Store};
use hy_core::time::UserZone;

const FAST: KdfParams = KdfParams {
    memory_kib: 64,
    passes: 1,
    lanes: 1,
};
const PASSWORD: &str = "a long password";

fn ist(month: u32, day: u32, h: u32) -> DateTime<Utc> {
    UserZone::KOLKATA.resolve_local(
        NaiveDate::from_ymd_opt(2026, month, day)
            .unwrap()
            .and_hms_opt(h, 0, 0)
            .unwrap(),
    )
}

fn date(month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, month, day).unwrap()
}

fn store_at(created: DateTime<Utc>, now: DateTime<Utc>) -> (Store, Arc<ManualClock>) {
    let clock = Arc::new(ManualClock::new(created));
    let store = Store::open_in_memory(clock.clone()).unwrap();
    clock.set_wall(now);
    (store, clock)
}

/// A store with two weeks of use: tallies, water with a correction, a goal change,
/// settings, a pet edit, a tombstone and an engine state.
fn used_store() -> Store {
    let (mut s, _) = store_at(ist(10, 5, 9), ist(10, 21, 12));
    let mut revision = None;
    for day in 5..=20 {
        let tally = DayTally {
            pc_active_ms: 3_600_000,
            work_active_ms: 3_000_000,
            confirmed_breaks: 2,
            ..DayTally::default()
        };
        revision = Some(
            s.commit_coach_step(CoachCommit {
                expected_revision: revision,
                format: 1,
                engine_time_ms: 0,
                state: "{}",
                events: &[],
                days: &[(date(10, day), tally)],
            })
            .unwrap(),
        );
        s.append_event(
            NewEvent::manual(EventPayload::WaterLogged { amount_ml: 750 }).at(ist(10, day, 11)),
        )
        .unwrap();
    }
    let wrong = s
        .append_event(
            NewEvent::manual(EventPayload::WaterLogged { amount_ml: 1500 }).at(ist(10, 20, 15)),
        )
        .unwrap();
    s.append_event(
        NewEvent::manual(EventPayload::WaterLogged { amount_ml: 250 })
            .at(ist(10, 20, 15))
            .superseding(wrong.id),
    )
    .unwrap();
    s.set_goal(GoalKind::WaterDailyMl, 1500, date(10, 5))
        .unwrap();
    s.set_goal(GoalKind::WaterDailyMl, 2000, date(10, 12))
        .unwrap();
    s.set_coach_config(&CoachConfig {
        intensity: Intensity::Light,
        ..CoachConfig::default()
    })
    .unwrap();
    s.set_water_servings(&Servings::new(vec![200, 750]).unwrap())
        .unwrap();
    s.save_pet_profile(&PetProfile::defaults("shelly", "Pebble"))
        .unwrap();
    s.record_deletion(
        Category::Hydration,
        DateRange::new(date(10, 6), date(10, 7)).unwrap(),
    )
    .unwrap();
    s
}

#[test]
fn export_then_import_on_another_pc_gives_the_same_reviews() {
    let a = used_store();
    let (file, summary) = a.export_backup_with(PASSWORD, FAST).unwrap();
    assert_eq!(summary.days, 16);
    assert_eq!(summary.events, 18);
    assert_eq!(summary.from, Some(date(10, 5)));
    assert_eq!(summary.to, Some(date(10, 20)));

    // A new PC, set up today.
    let (mut b, _) = store_at(ist(10, 21, 9), ist(10, 21, 12));
    let imported = b.import_backup(&file, PASSWORD).unwrap();
    assert_eq!(imported.events, 18);

    assert_eq!(
        b.week_review(Some(date(10, 14))).unwrap(),
        a.week_review(Some(date(10, 14))).unwrap()
    );
    assert_eq!(
        b.month_review(None).unwrap().bars,
        a.month_review(None).unwrap().bars
    );
    assert_eq!(b.first_use_date().unwrap(), date(10, 5));
    assert_eq!(b.coach_config().unwrap().intensity, Intensity::Light);
    assert_eq!(b.water_servings().unwrap().ml(), &[200, 750]);
    assert_eq!(b.pet_profile().unwrap().unwrap().display_name, "Pebble");
    assert_eq!(
        b.goal_on(GoalKind::WaterDailyMl, date(10, 8)).unwrap(),
        Some(1500)
    );
    // Every row keeps its origin; this PC keeps its own identity.
    assert_ne!(b.device_id(), a.device_id());
    let events = b.current_events_on(date(10, 20)).unwrap();
    assert!(events.iter().all(|e| e.origin_device == a.device_id()));
    // The engine starts fresh from the imported data.
    assert!(b.coach_state().unwrap().is_none());
}

#[test]
fn import_replaces_local_data_and_never_merges() {
    let a = used_store();
    let (file, _) = a.export_backup_with(PASSWORD, FAST).unwrap();
    let (mut b, _) = store_at(ist(10, 21, 9), ist(10, 21, 12));
    b.append_event(
        NewEvent::manual(EventPayload::WaterLogged { amount_ml: 400 }).at(ist(10, 21, 10)),
    )
    .unwrap();
    b.set_goal(GoalKind::WaterDailyMl, 3000, date(10, 21))
        .unwrap();
    b.import_backup(&file, PASSWORD).unwrap();
    assert!(b.current_events_on(date(10, 21)).unwrap().is_empty());
    assert_eq!(
        b.goal_on(GoalKind::WaterDailyMl, date(10, 21)).unwrap(),
        Some(2000)
    );
    // Importing the same file twice gives the same data, not twice the data.
    let once = b.export_backup_with(PASSWORD, FAST).unwrap().1;
    b.import_backup(&file, PASSWORD).unwrap();
    let twice = b.export_backup_with(PASSWORD, FAST).unwrap().1;
    assert_eq!((once.events, once.days), (twice.events, twice.days));
}

#[test]
fn tombstones_are_merged_as_a_union() {
    let a = used_store();
    let (file, _) = a.export_backup_with(PASSWORD, FAST).unwrap();
    let (mut b, _) = store_at(ist(10, 21, 9), ist(10, 21, 12));
    let local = b
        .record_deletion(
            Category::Outcomes,
            DateRange::new(date(10, 1), date(10, 3)).unwrap(),
        )
        .unwrap();
    b.import_backup(&file, PASSWORD).unwrap();
    let hydration = b.tombstones(Category::Hydration).unwrap();
    assert_eq!(hydration.len(), 1);
    assert_eq!(hydration[0].range.from, date(10, 6));
    let outcomes = b.tombstones(Category::Outcomes).unwrap();
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].request_id, local);
    // Both deletions' consent entries are kept, and a second import adds none.
    let entries = b.consent_log().unwrap().len();
    assert_eq!(entries, 2);
    b.import_backup(&file, PASSWORD).unwrap();
    assert_eq!(b.consent_log().unwrap().len(), entries);
    assert_eq!(b.tombstones(Category::Hydration).unwrap().len(), 1);
}

#[test]
fn sharing_and_pairing_are_never_carried_over() {
    let a = used_store();
    let contents = a.backup_contents().unwrap();
    for left_out in [
        "sharing_policy",
        "policy_ops",
        "outbox",
        "store_meta",
        "coach_state",
    ] {
        assert!(!contents.tables.contains_key(left_out), "{left_out}");
    }
    let (mut b, _) = store_at(ist(10, 21, 9), ist(10, 21, 12));
    b.import_contents(&contents).unwrap();
    for c in Category::ALL {
        assert!(!b.sharing_policy(*c).unwrap().enabled);
    }
}

#[test]
fn a_refused_file_changes_nothing() {
    let a = used_store();
    let (file, _) = a.export_backup_with(PASSWORD, FAST).unwrap();
    let (mut b, _) = store_at(ist(10, 21, 9), ist(10, 21, 12));
    b.append_event(
        NewEvent::manual(EventPayload::WaterLogged { amount_ml: 400 }).at(ist(10, 21, 10)),
    )
    .unwrap();
    let before = b.export_backup_with(PASSWORD, FAST).unwrap().1;

    assert!(matches!(
        b.import_backup(&file, "not the password"),
        Err(Error::Backup(_))
    ));
    let mut damaged = file.clone();
    let last = damaged.len() - 1;
    damaged[last] ^= 0x40;
    assert!(matches!(
        b.import_backup(&damaged, PASSWORD),
        Err(Error::Backup(_))
    ));

    // A newer schema, an unknown table and a row the schema refuses are all refused.
    let good = Contents::open(&file, PASSWORD).unwrap();
    let mut newer = good.clone();
    newer.schema_version += 1;
    let mut unknown = good.clone();
    unknown
        .tables
        .insert("outbox".into(), good.tables["settings"].clone());
    let mut bad_row = good.clone();
    let events = bad_row.tables.get_mut("events").unwrap();
    let ty = events.columns.iter().position(|c| c == "type").unwrap();
    events.rows[3][ty] = serde_json::json!("NOT A TYPE");
    for c in [newer, unknown, bad_row] {
        assert!(matches!(b.import_contents(&c), Err(Error::Backup(_))));
    }
    let after = b.export_backup_with(PASSWORD, FAST).unwrap().1;
    assert_eq!(
        (before.events, before.days, before.from),
        (after.events, after.days, after.from)
    );
    assert_eq!(b.current_events_on(date(10, 21)).unwrap().len(), 1);
}

#[test]
fn passwords_are_checked_before_anything_is_written() {
    let a = used_store();
    assert!(matches!(
        a.export_backup_with("short", FAST),
        Err(Error::Invalid(_))
    ));
}
