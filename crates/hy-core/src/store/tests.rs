use std::sync::Arc;

use chrono::{Duration, NaiveDate, TimeZone, Utc};
use serde_json::json;
use uuid::Uuid;

use super::*;
use crate::clock::ManualClock;
use crate::schema::{
    Activity, PhoneLabel, ReminderOutcome, RoutineOutcome, RoutineVariant, SessionState,
};

/// Wednesday 21 Oct 2026, 10:00 IST.
fn start() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 21, 4, 30, 0).unwrap()
}

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

fn store() -> (Store, Arc<ManualClock>) {
    let clock = Arc::new(ManualClock::new(start()));
    let store = Store::open_in_memory(clock.clone()).unwrap();
    (store, clock)
}

fn water(ml: u32) -> NewEvent {
    NewEvent::manual(EventPayload::WaterLogged { amount_ml: ml })
}

fn water_total(store: &Store, d: NaiveDate) -> u32 {
    store
        .current_events_on(d)
        .unwrap()
        .iter()
        .map(|e| match e.payload {
            EventPayload::WaterLogged { amount_ml } => amount_ml,
            _ => 0,
        })
        .sum()
}

fn enable(store: &mut Store, c: Category, share_from: NaiveDate) {
    let op = store.request_enable(c, "notice-1").unwrap();
    let next_seq = store.sharing_policy(c).unwrap().seq + 1;
    assert_eq!(
        store.accept_enable(op, next_seq, share_from).unwrap(),
        EnableOutcome::Applied
    );
}

// ------------------------------------------------------------------ opening

#[test]
fn opening_migrates_and_creates_a_device_identity() {
    let (s, _) = store();
    assert_eq!(s.migration_report().to, migrations::latest_version());
    assert_eq!(s.device_id().get_version_num(), 7);
    let fk: bool = s
        .conn
        .pragma_query_value(None, "foreign_keys", |r| r.get(0))
        .unwrap();
    assert!(fk);
}

#[test]
fn file_store_keeps_its_device_id_and_data_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hy.db");
    let clock = Arc::new(ManualClock::new(start()));
    let (device, event_id) = {
        let mut s = Store::open(&path, clock.clone()).unwrap();
        let e = s.append_event(water(250)).unwrap();
        (s.device_id(), e.id)
    };
    let s = Store::open(&path, clock).unwrap();
    assert_eq!(s.device_id(), device);
    assert!(s.migration_report().applied.is_empty());
    let mode: String = s
        .conn
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
    let sync: i64 = s
        .conn
        .pragma_query_value(None, "synchronous", |r| r.get(0))
        .unwrap();
    assert_eq!(sync, 2, "synchronous=FULL");
    assert_eq!(s.event(event_id).unwrap().unwrap().id, event_id);
}

// ----------------------------------------------------------- settings, zone

#[test]
fn zone_and_schedule_default_and_round_trip() {
    let (s, _) = store();
    assert_eq!(s.user_zone().unwrap(), UserZone::KOLKATA);
    assert_eq!(s.work_schedule().unwrap(), WorkSchedule::default());
    assert_eq!(s.today().unwrap(), date(2026, 10, 21));

    let ny = UserZone::parse("America/New_York").unwrap();
    s.set_user_zone(ny).unwrap();
    assert_eq!(s.user_zone().unwrap(), ny);
    assert_eq!(s.today().unwrap(), date(2026, 10, 21)); // 00:30 in New York

    let mut sched = WorkSchedule::default();
    sched.workdays = sched.workdays.with(chrono::Weekday::Sun);
    s.set_work_schedule(&sched).unwrap();
    assert_eq!(s.work_schedule().unwrap(), sched);
    let bad = WorkSchedule {
        end: sched.start,
        ..sched
    };
    assert!(matches!(s.set_work_schedule(&bad), Err(Error::Invalid(_))));
}

#[test]
fn generic_settings_round_trip_and_reject_bad_keys() {
    let (s, _) = store();
    assert_eq!(s.setting::<u32>("reminder.intensity").unwrap(), None);
    s.put_setting("reminder.intensity", &2u32).unwrap();
    s.put_setting("reminder.intensity", &3u32).unwrap();
    assert_eq!(s.setting::<u32>("reminder.intensity").unwrap(), Some(3));
    assert!(s.setting::<String>("reminder.intensity").is_err());
    assert!(s.put_setting("", &1).is_err());
    assert!(s.put_setting(&"k".repeat(65), &1).is_err());
}

#[test]
fn corrupt_zone_setting_is_reported_not_ignored() {
    let (s, _) = store();
    s.put_setting(keys::TIME_ZONE, "Mars/Base").unwrap();
    assert!(s.user_zone().is_err());
}

#[test]
fn today_follows_the_wall_clock_across_clock_changes() {
    let (s, clock) = store();
    clock.set_wall(Utc.with_ymd_and_hms(2026, 10, 21, 18, 29, 0).unwrap());
    assert_eq!(s.today().unwrap(), date(2026, 10, 21));
    clock.advance(Duration::minutes(1)); // 00:00 IST
    assert_eq!(s.today().unwrap(), date(2026, 10, 22));
    clock.shift_wall(Duration::hours(-1)); // clock set back across midnight
    assert_eq!(s.today().unwrap(), date(2026, 10, 21));
}

// -------------------------------------------------------------------- goals

#[test]
fn goals_apply_from_their_effective_date() {
    let (s, _) = store();
    assert_eq!(
        s.goal_on(GoalKind::WaterDailyMl, date(2026, 10, 1))
            .unwrap(),
        None
    );
    s.set_goal(GoalKind::WaterDailyMl, 2000, date(2026, 10, 1))
        .unwrap();
    s.set_goal(GoalKind::WaterDailyMl, 2500, date(2026, 10, 19))
        .unwrap();
    assert_eq!(
        s.goal_on(GoalKind::WaterDailyMl, date(2026, 9, 30))
            .unwrap(),
        None
    );
    assert_eq!(
        s.goal_on(GoalKind::WaterDailyMl, date(2026, 10, 18))
            .unwrap(),
        Some(2000)
    );
    assert_eq!(
        s.goal_on(GoalKind::WaterDailyMl, date(2026, 10, 19))
            .unwrap(),
        Some(2500)
    );
    assert_eq!(
        s.goal_on(GoalKind::WaterDailyMl, date(2027, 1, 1)).unwrap(),
        Some(2500)
    );
    // Changing the goal again on the same day replaces that day's value only.
    s.set_goal(GoalKind::WaterDailyMl, 2250, date(2026, 10, 19))
        .unwrap();
    assert_eq!(
        s.goal_on(GoalKind::WaterDailyMl, date(2026, 10, 20))
            .unwrap(),
        Some(2250)
    );
    assert_eq!(
        s.goal_on(GoalKind::WaterDailyMl, date(2026, 10, 18))
            .unwrap(),
        Some(2000)
    );
    assert!(
        s.set_goal(GoalKind::WaterDailyMl, 50, date(2026, 10, 21))
            .is_err()
    );
    assert!(
        s.set_goal(GoalKind::WaterDailyMl, 20_000, date(2026, 10, 21))
            .is_err()
    );
}

// ---------------------------------------------------------------------- pet

#[test]
fn pet_profile_round_trip_and_reset() {
    let (s, _) = store();
    assert_eq!(s.pet_profile().unwrap(), None);
    let mut p = PetProfile::defaults("turtle", "Tortuga");
    s.save_pet_profile(&p).unwrap();
    assert_eq!(s.pet_profile().unwrap(), Some(p.clone()));
    p.display_name = "Speedy Śrī turtle".into(); // 17 characters, more bytes
    p.scale_pct = 150;
    p.edge = PetEdge::Free;
    p.position = Some((-120, 640));
    p.motion = false;
    p.tone = PetTone::Cheerful;
    p.colour_variant = 2;
    s.save_pet_profile(&p).unwrap();
    assert_eq!(s.pet_profile().unwrap(), Some(p));
    let reset = PetProfile::defaults("turtle", "Tortuga");
    s.save_pet_profile(&reset).unwrap();
    assert_eq!(s.pet_profile().unwrap(), Some(reset));
}

#[test]
fn pet_profile_rules() {
    let (s, _) = store();
    let ok = PetProfile::defaults("cat", "Mittens");
    for bad in [
        PetProfile {
            scale_pct: 74,
            ..ok.clone()
        },
        PetProfile {
            scale_pct: 151,
            ..ok.clone()
        },
        PetProfile {
            colour_variant: 3,
            ..ok.clone()
        },
        PetProfile {
            display_name: String::new(),
            ..ok.clone()
        },
        PetProfile {
            display_name: "   ".into(),
            ..ok.clone()
        },
        PetProfile {
            display_name: "x".repeat(21),
            ..ok.clone()
        },
        PetProfile {
            pet_id: String::new(),
            ..ok.clone()
        },
    ] {
        assert!(
            matches!(s.save_pet_profile(&bad), Err(Error::Invalid(_))),
            "{bad:?}"
        );
    }
    assert_eq!(s.pet_profile().unwrap(), None);
}

// ------------------------------------------------------------------- events

#[test]
fn event_round_trip_keeps_every_field() {
    let (mut s, clock) = store();
    let rid = Uuid::nil();
    let offered = s
        .append_event(NewEvent::inferred(
            EventPayload::ReminderOffered {
                reminder_id: rid,
                activity: Activity::Movement,
            },
            MeasurementStatus::Measured,
        ))
        .unwrap();
    clock.advance(Duration::minutes(10));
    let response = s
        .append_event(
            NewEvent::inferred(
                EventPayload::ReminderResponse {
                    reminder_id: rid,
                    activity: Activity::Movement,
                    outcome: ReminderOutcome::NoResponse,
                },
                MeasurementStatus::Measured,
            )
            .with_confidence(1.0),
        )
        .unwrap();
    let loaded = s.event(response.id).unwrap().unwrap();
    assert_eq!(loaded, response);
    assert_eq!(loaded.zone, UserZone::KOLKATA);
    assert_eq!(loaded.local_date, date(2026, 10, 21));
    assert_eq!(loaded.recorded_at, start() + Duration::minutes(10));
    assert_eq!(loaded.origin_device, s.device_id());
    assert_eq!(loaded.schema_version, crate::schema::EVENT_SCHEMA_VERSION);
    assert!(offered.id < response.id);
    let day = s.events_on(date(2026, 10, 21)).unwrap();
    assert_eq!(
        day.iter().map(|e| e.id).collect::<Vec<_>>(),
        vec![offered.id, response.id]
    );
    assert_eq!(s.event(Uuid::nil()).unwrap(), None);
}

#[test]
fn every_payload_kind_round_trips_through_the_store() {
    let (mut s, _) = store();
    let samples = vec![
        NewEvent::system(EventPayload::SessionState {
            state: SessionState::ConfirmedBreak,
        }),
        NewEvent::manual(EventPayload::MovementSession {
            routine_id: "mon-neck".into(),
            routine_version: 1,
            variant: Some(RoutineVariant::Seated),
            outcome: RoutineOutcome::Partial,
            steps_done: 2,
            steps_skipped: 1,
            steps_total: 3,
        }),
        NewEvent::manual(EventPayload::PhoneInterruption {
            label: PhoneLabel::Unsure,
        }),
        NewEvent::inferred(
            EventPayload::PostureObservation {
                kind: crate::schema::PostureKind::BaselineSuspect,
                duration_s: 120,
            },
            MeasurementStatus::NoReading,
        ),
    ];
    for new in samples {
        let e = s.append_event(new.clone()).unwrap();
        assert_eq!(s.event(e.id).unwrap().unwrap().payload, new.payload);
    }
}

#[test]
fn invalid_events_are_refused_before_storage() {
    let (mut s, _) = store();
    assert!(matches!(s.append_event(water(0)), Err(Error::Invalid(_))));
    let mut imported = water(250);
    imported.source = EventSource::Imported;
    imported.status = MeasurementStatus::Measured;
    assert!(matches!(s.append_event(imported), Err(Error::Invalid(_))));
    assert!(s.events_on(date(2026, 10, 21)).unwrap().is_empty());
}

#[test]
fn local_date_and_zone_are_fixed_when_recorded() {
    let (mut s, clock) = store();
    // 23:50 IST on 21 Oct.
    clock.set_wall(Utc.with_ymd_and_hms(2026, 10, 21, 18, 20, 0).unwrap());
    let late = s.append_event(water(200)).unwrap();
    assert_eq!(late.local_date, date(2026, 10, 21));
    // The user moves to London: past events keep their IST date and zone.
    s.set_user_zone(UserZone::parse("Europe/London").unwrap())
        .unwrap();
    let reread = s.event(late.id).unwrap().unwrap();
    assert_eq!(reread.local_date, date(2026, 10, 21));
    assert_eq!(reread.zone, UserZone::KOLKATA);
    // A new event at the same instant gets London's date and zone.
    let next = s.append_event(water(100)).unwrap();
    assert_eq!(next.zone.name(), "Europe/London");
    assert_eq!(next.local_date, date(2026, 10, 21));
    // No event is duplicated by the change.
    assert_eq!(s.events_on(date(2026, 10, 21)).unwrap().len(), 2);
}

#[test]
fn events_after_ist_midnight_belong_to_the_next_day() {
    let (mut s, clock) = store();
    clock.set_wall(Utc.with_ymd_and_hms(2026, 10, 21, 18, 29, 59).unwrap());
    let before = s.append_event(water(100)).unwrap();
    clock.advance(Duration::seconds(1));
    let after = s.append_event(water(150)).unwrap();
    assert_eq!(before.local_date, date(2026, 10, 21));
    assert_eq!(after.local_date, date(2026, 10, 22));
    assert_eq!(water_total(&s, date(2026, 10, 21)), 100);
    assert_eq!(water_total(&s, date(2026, 10, 22)), 150);
}

#[test]
fn backfill_is_allowed_but_future_times_are_refused() {
    let (mut s, _) = store();
    let earlier = start() - Duration::days(1);
    let e = s.append_event(water(250).at(earlier)).unwrap();
    assert_eq!(e.local_date, date(2026, 10, 20));
    assert!(e.recorded_at > e.occurred_at);
    assert!(
        s.append_event(water(250).at(start() + Duration::minutes(4)))
            .is_ok()
    );
    assert!(matches!(
        s.append_event(water(250).at(start() + Duration::minutes(6))),
        Err(Error::Invalid(_))
    ));
}

#[test]
fn ids_stay_ordered_when_the_clock_is_set_back() {
    let (mut s, clock) = store();
    let a = s.append_event(water(100)).unwrap();
    clock.shift_wall(Duration::hours(-2));
    let b = s.append_event(water(100)).unwrap();
    assert!(a.id < b.id);
    assert!(
        b.recorded_at < a.recorded_at,
        "wall time is recorded as the PC shows it"
    );
}

// ------------------------------------------- hydration: undo and correction

#[test]
fn hydration_totals_reconcile_after_correction_and_undo() {
    let (mut s, clock) = store();
    let today = date(2026, 10, 21);
    let a = s.append_event(water(250)).unwrap();
    clock.advance(Duration::minutes(30));
    let b = s.append_event(water(500)).unwrap();
    assert_eq!(water_total(&s, today), 750);

    // Edit B from 500 to 300 ml.
    clock.advance(Duration::minutes(5));
    let c = s.append_event(water(300).superseding(b.id)).unwrap();
    assert_eq!(
        c.occurred_at, b.occurred_at,
        "a correction keeps the entry's time by default"
    );
    assert_eq!(water_total(&s, today), 550);

    // Undo A.
    let d = s
        .append_event(NewEvent::manual(EventPayload::EntryRetracted {}).superseding(a.id))
        .unwrap();
    assert_eq!(d.local_date, today);
    assert_eq!(water_total(&s, today), 300);

    // The full history is kept; the current view hides superseded rows.
    assert_eq!(s.events_on(today).unwrap().len(), 4);
    let current: Vec<Uuid> = s
        .current_events_on(today)
        .unwrap()
        .iter()
        .map(|e| e.id)
        .collect();
    assert_eq!(current, vec![c.id]);

    // Correcting the correction works; correcting B again does not.
    assert!(matches!(
        s.append_event(water(400).superseding(b.id)),
        Err(Error::Invalid(_))
    ));
    s.append_event(water(400).superseding(c.id)).unwrap();
    assert_eq!(water_total(&s, today), 400);
}

#[test]
fn correcting_yesterdays_entry_changes_yesterday_only() {
    let (mut s, clock) = store();
    let yesterday_entry = s
        .append_event(water(250).at(start() - Duration::days(1)))
        .unwrap();
    s.append_event(water(200)).unwrap();
    clock.advance(Duration::hours(2));
    let fix = s
        .append_event(water(350).superseding(yesterday_entry.id))
        .unwrap();
    assert_eq!(fix.local_date, date(2026, 10, 20));
    assert_eq!(water_total(&s, date(2026, 10, 20)), 350);
    assert_eq!(water_total(&s, date(2026, 10, 21)), 200);
}

#[test]
fn correction_keeps_the_entry_zone_after_a_zone_change() {
    let (mut s, _) = store();
    let e = s.append_event(water(250)).unwrap();
    s.set_user_zone(UserZone::parse("Pacific/Kiritimati").unwrap())
        .unwrap();
    let fix = s.append_event(water(300).superseding(e.id)).unwrap();
    assert_eq!(fix.zone, UserZone::KOLKATA);
    assert_eq!(fix.local_date, e.local_date);
}

#[test]
fn supersede_rules_are_enforced() {
    let (mut s, _) = store();
    let phone = s
        .append_event(NewEvent::manual(EventPayload::PhoneInterruption {
            label: PhoneLabel::Unlabelled,
        }))
        .unwrap();
    // Relabelling is a correction of the same type.
    let relabel = s
        .append_event(
            NewEvent::manual(EventPayload::PhoneInterruption {
                label: PhoneLabel::Work,
            })
            .superseding(phone.id),
        )
        .unwrap();
    assert_eq!(
        s.current_events_on(phone.local_date).unwrap()[0].id,
        relabel.id
    );

    // A different type cannot correct it.
    assert!(matches!(
        s.append_event(water(250).superseding(relabel.id)),
        Err(Error::Invalid(_))
    ));
    // Unknown target.
    assert!(matches!(
        s.append_event(water(250).superseding(Uuid::nil())),
        Err(Error::NotFound(_))
    ));
    // Non-correctable target.
    let state = s
        .append_event(NewEvent::system(EventPayload::SessionState {
            state: SessionState::Active,
        }))
        .unwrap();
    assert!(matches!(
        s.append_event(NewEvent::manual(EventPayload::EntryRetracted {}).superseding(state.id)),
        Err(Error::Invalid(_))
    ));
    // A retraction cannot be retracted or corrected (log the entry again instead).
    let w = s.append_event(water(100)).unwrap();
    let undo = s
        .append_event(NewEvent::manual(EventPayload::EntryRetracted {}).superseding(w.id))
        .unwrap();
    assert!(
        s.append_event(NewEvent::manual(EventPayload::EntryRetracted {}).superseding(undo.id))
            .is_err()
    );
    // A retraction keeps the entry's time.
    let w2 = s.append_event(water(100)).unwrap();
    assert!(matches!(
        s.append_event(
            NewEvent::manual(EventPayload::EntryRetracted {})
                .superseding(w2.id)
                .at(start() - Duration::hours(1))
        ),
        Err(Error::Invalid(_))
    ));
}

#[test]
fn events_are_append_only_in_the_database() {
    let (mut s, _) = store();
    let e = s.append_event(water(250)).unwrap();
    let err = s
        .conn
        .execute(
            "UPDATE events SET payload = '{\"amount_ml\":2000}' WHERE id = ?1",
            [e.id.to_string()],
        )
        .unwrap_err();
    assert!(err.to_string().contains("append-only"), "{err}");
    let err = s
        .conn
        .execute(
            "UPDATE events SET local_date = '2026-10-01' WHERE id = ?1",
            [e.id.to_string()],
        )
        .unwrap_err();
    assert!(err.to_string().contains("append-only"), "{err}");
}

// -------------------------------------------------------------- summaries

fn daily(origin: Uuid, d: NaiveDate, revision: u32, ml: u32) -> DailySummary {
    Summary {
        category: Category::Hydration,
        origin_device: origin,
        period: d,
        revision,
        coverage: Some(0.8),
        payload: json!({"ml": ml, "goal": 2000, "entries": 3}),
    }
}

#[test]
fn daily_summary_upsert_only_moves_forward() {
    let (s, _) = store();
    let me = s.device_id();
    let d = date(2026, 10, 21);
    assert_eq!(s.daily_summary(Category::Hydration, me, d).unwrap(), None);
    assert!(s.upsert_daily_summary(&daily(me, d, 1, 500)).unwrap());
    assert!(s.upsert_daily_summary(&daily(me, d, 3, 900)).unwrap());
    assert!(!s.upsert_daily_summary(&daily(me, d, 2, 700)).unwrap());
    assert!(!s.upsert_daily_summary(&daily(me, d, 3, 999)).unwrap());
    let row = s
        .daily_summary(Category::Hydration, me, d)
        .unwrap()
        .unwrap();
    assert_eq!(row.revision, 3);
    assert_eq!(row.payload["ml"], 900);
    // Other devices' rows are separate (durations are never merged across devices).
    let other = Uuid::now_v7();
    assert!(s.upsert_daily_summary(&daily(other, d, 1, 100)).unwrap());
    assert_eq!(
        s.daily_summary(Category::Hydration, me, d)
            .unwrap()
            .unwrap()
            .payload["ml"],
        900
    );
}

#[test]
fn summary_rules() {
    let (s, _) = store();
    let me = s.device_id();
    let d = date(2026, 10, 21);
    assert!(s.upsert_daily_summary(&daily(me, d, 0, 1)).is_err());
    let mut bad = daily(me, d, 1, 1);
    bad.coverage = Some(1.2);
    assert!(s.upsert_daily_summary(&bad).is_err());
    let mut bad = daily(me, d, 1, 1);
    bad.payload = json!([1, 2]);
    assert!(s.upsert_daily_summary(&bad).is_err());
    let mut none = daily(me, d, 1, 1);
    none.coverage = None;
    assert!(s.upsert_daily_summary(&none).is_ok());
}

#[test]
fn monthly_summaries_round_trip() {
    let (s, _) = store();
    let me = s.device_id();
    let row = |m, rev| Summary {
        category: Category::Outcomes,
        origin_device: me,
        period: MonthKey::new(2026, m).unwrap(),
        revision: rev,
        coverage: None,
        payload: json!({"routines_done": 12}),
    };
    assert!(s.upsert_monthly_summary(&row(10, 1)).unwrap());
    assert!(s.upsert_monthly_summary(&row(9, 1)).unwrap());
    assert!(!s.upsert_monthly_summary(&row(10, 1)).unwrap());
    let rows = s.monthly_summaries(Category::Outcomes).unwrap();
    assert_eq!(
        rows.iter().map(|r| r.period).collect::<Vec<_>>(),
        vec![
            MonthKey::new(2026, 9).unwrap(),
            MonthKey::new(2026, 10).unwrap()
        ]
    );
    assert!(MonthKey::new(2026, 13).is_err());
    assert!(MonthKey::new(2026, 0).is_err());
    assert_eq!(
        MonthKey::of(date(2026, 2, 28)),
        MonthKey::new(2026, 2).unwrap()
    );
}

#[test]
fn backup_import_takes_precedence_over_server_restore() {
    let (s, _) = store();
    let origin = Uuid::now_v7();
    let d = date(2026, 10, 12);
    assert!(
        s.upsert_imported_summary(&daily(origin, d, 5, 999), ImportProvenance::ServerRestore)
            .unwrap()
    );
    // A local backup import replaces the server copy even with a lower revision.
    assert!(
        s.upsert_imported_summary(&daily(origin, d, 2, 700), ImportProvenance::BackupImport)
            .unwrap()
    );
    // A later server restore never overwrites the local import.
    assert!(
        !s.upsert_imported_summary(&daily(origin, d, 9, 111), ImportProvenance::ServerRestore)
            .unwrap()
    );
    let (row, provenance) = s
        .imported_summary(Category::Hydration, origin, d)
        .unwrap()
        .unwrap();
    assert_eq!(provenance, ImportProvenance::BackupImport);
    assert_eq!(row.payload["ml"], 700);
    // Restoring twice does not double anything (T-L6): one row per {category, origin, date}.
    let d2 = date(2026, 10, 13);
    assert!(
        s.upsert_imported_summary(&daily(origin, d2, 1, 10), ImportProvenance::ServerRestore)
            .unwrap()
    );
    assert!(
        !s.upsert_imported_summary(&daily(origin, d2, 1, 10), ImportProvenance::ServerRestore)
            .unwrap()
    );
    let n: i64 = s
        .conn
        .query_row("SELECT COUNT(*) FROM imported_summary", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 2);
    // Imported rows are never this PC's daily totals.
    assert_eq!(
        s.daily_summary(Category::Hydration, origin, d).unwrap(),
        None
    );
}

// --------------------------------------------- sharing and control changes

#[test]
fn every_category_starts_off() {
    let (s, _) = store();
    for c in Category::ALL {
        let p = s.sharing_policy(*c).unwrap();
        assert!(!p.enabled && p.share_from.is_none() && p.seq == 0, "{c}");
        assert_eq!(
            s.upload_block(*c, date(2026, 10, 21)).unwrap(),
            Some(UploadBlock::CategoryOff)
        );
    }
}

#[test]
fn enable_takes_effect_only_when_accepted() {
    let (mut s, _) = store();
    let today = date(2026, 10, 21);
    let op = s.request_enable(Category::Hydration, "notice-1").unwrap();
    assert!(!s.sharing_policy(Category::Hydration).unwrap().enabled);
    assert!(matches!(
        s.enqueue_upload(Category::Hydration, today, 1, &json!({"ml": 1})),
        Err(Error::UploadBlocked(_))
    ));
    s.accept_enable(op, 4, today).unwrap();
    let p = s.sharing_policy(Category::Hydration).unwrap();
    assert!(p.enabled);
    assert_eq!(p.share_from, Some(today));
    assert_eq!(p.seq, 4);
    let stored = s.policy_op(op).unwrap().unwrap();
    assert_eq!(stored.state, PolicyOpState::Accepted);
    assert_eq!(stored.server_seq, Some(4));
    // Accepting twice, or a non-existent op, is refused.
    assert!(matches!(
        s.accept_enable(op, 5, today),
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        s.accept_enable(Uuid::nil(), 5, today),
        Err(Error::NotFound(_))
    ));
    let kinds: Vec<ConsentKind> = s.consent_log().unwrap().iter().map(|c| c.kind).collect();
    assert_eq!(
        kinds,
        vec![
            ConsentKind::NoticeShown,
            ConsentKind::EnableRequested,
            ConsentKind::Enabled
        ]
    );
    assert_eq!(
        s.consent_log().unwrap()[0].notice_version.as_deref(),
        Some("notice-1")
    );
}

#[test]
fn enable_accepted_after_a_local_disable_stays_off() {
    let (mut s, clock) = store();
    let op = s.request_enable(Category::Outcomes, "notice-1").unwrap();
    clock.advance(Duration::seconds(1));
    s.disable_sharing(Category::Outcomes).unwrap();
    s.accept_enable(op, 2, date(2026, 10, 21)).unwrap();
    assert!(!s.sharing_policy(Category::Outcomes).unwrap().enabled);
}

#[test]
fn uploads_respect_share_from_and_queue_newer_revisions() {
    let (mut s, _) = store();
    let today = date(2026, 10, 21);
    enable(&mut s, Category::Hydration, today);
    // No backfill before share_from.
    assert_eq!(
        s.upload_block(Category::Hydration, date(2026, 10, 20))
            .unwrap(),
        Some(UploadBlock::BeforeShareFrom)
    );
    assert!(matches!(
        s.enqueue_upload(
            Category::Hydration,
            date(2026, 10, 20),
            1,
            &json!({"ml": 1})
        ),
        Err(Error::UploadBlocked(_))
    ));
    s.enqueue_upload(Category::Hydration, today, 1, &json!({"ml": 250}))
        .unwrap();
    s.enqueue_upload(Category::Hydration, today, 2, &json!({"ml": 500}))
        .unwrap();
    assert!(matches!(
        s.enqueue_upload(Category::Hydration, today, 2, &json!({"ml": 9})),
        Err(Error::Invalid(_))
    ));
    assert!(
        s.enqueue_upload(Category::Hydration, today, 0, &json!({}))
            .is_err()
    );
    let q = s.outbox().unwrap();
    assert_eq!(q.len(), 1);
    assert_eq!(q[0].revision, 2);
    assert_eq!(q[0].origin_device, s.device_id());
    assert_eq!(q[0].payload["ml"], 500);
}

#[test]
fn disable_purges_the_queue_atomically_even_offline() {
    let (mut s, _) = store();
    let today = date(2026, 10, 21);
    enable(&mut s, Category::Hydration, date(2026, 10, 1));
    enable(&mut s, Category::PhoneReports, date(2026, 10, 1));
    s.enqueue_upload(Category::Hydration, today, 1, &json!({"ml": 1}))
        .unwrap();
    s.enqueue_upload(
        Category::Hydration,
        date(2026, 10, 20),
        1,
        &json!({"ml": 2}),
    )
    .unwrap();
    s.enqueue_upload(Category::PhoneReports, today, 1, &json!({"work": 1}))
        .unwrap();

    let op = s.disable_sharing(Category::Hydration).unwrap();

    assert!(!s.sharing_policy(Category::Hydration).unwrap().enabled);
    let q = s.outbox().unwrap();
    assert_eq!(q.len(), 1);
    assert_eq!(q[0].category, Category::PhoneReports);
    let stored = s.policy_op(op).unwrap().unwrap();
    assert_eq!(
        (stored.kind, stored.state, stored.base_seq),
        (PolicyOpKind::Disable, PolicyOpState::Pending, 1)
    );
    let last = s.consent_log().unwrap().pop().unwrap();
    assert_eq!(
        (last.kind, last.category, last.op_ref),
        (ConsentKind::Disabled, Some(Category::Hydration), Some(op))
    );
    let changed = s
        .current_events_on(today)
        .unwrap()
        .into_iter()
        .filter(|e| {
            e.payload
                == EventPayload::SharingPolicyChanged {
                    category: Category::Hydration,
                    enabled: false,
                }
        })
        .count();
    assert_eq!(changed, 1);
    assert_eq!(
        s.upload_block(Category::Hydration, today).unwrap(),
        Some(UploadBlock::CategoryOff)
    );
}

#[test]
fn a_failed_disable_changes_nothing() {
    let (mut s, _) = store();
    let today = date(2026, 10, 21);
    enable(&mut s, Category::Hydration, today);
    s.enqueue_upload(Category::Hydration, today, 1, &json!({"ml": 1}))
        .unwrap();
    let consent_before = s.consent_log().unwrap().len();
    // Make the last step of the transaction (the event insert) fail.
    s.conn
        .execute_batch(
            "CREATE TEMP TRIGGER fail_events BEFORE INSERT ON events
             BEGIN SELECT RAISE(ABORT, 'injected failure'); END;",
        )
        .unwrap();
    let err = s.disable_sharing(Category::Hydration).unwrap_err();
    assert!(err.to_string().contains("injected failure"), "{err}");
    assert!(
        s.sharing_policy(Category::Hydration).unwrap().enabled,
        "policy rolled back"
    );
    assert_eq!(s.outbox().unwrap().len(), 1, "queue purge rolled back");
    assert_eq!(
        s.consent_log().unwrap().len(),
        consent_before,
        "consent entry rolled back"
    );
    let ops: i64 = s
        .conn
        .query_row(
            "SELECT COUNT(*) FROM policy_ops WHERE kind = 'disable'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(ops, 0, "policy op rolled back");
}

#[test]
fn deletion_tombstones_block_dates_permanently() {
    let (mut s, clock) = store();
    let today = date(2026, 10, 21);
    enable(&mut s, Category::Hydration, date(2026, 10, 1));
    for d in 12..=21 {
        s.enqueue_upload(Category::Hydration, date(2026, 10, d), 1, &json!({"ml": d}))
            .unwrap();
    }
    let range = DateRange::new(date(2026, 10, 12), date(2026, 10, 16)).unwrap();
    let request = s.record_deletion(Category::Hydration, range).unwrap();
    // Queued records in the range are discarded in the same transaction.
    let dates: Vec<u32> = s
        .outbox()
        .unwrap()
        .iter()
        .map(|r| chrono::Datelike::day(&r.local_date))
        .collect();
    assert_eq!(dates, vec![17, 18, 19, 20, 21]);
    assert_eq!(
        s.tombstones(Category::Hydration).unwrap(),
        vec![Tombstone {
            request_id: request,
            category: Category::Hydration,
            range,
            state: TombstoneState::Pending,
        }]
    );
    // T-D7 shape: rebuilding a deleted date never produces an upload, whatever the
    // revision, even after switching off and on again.
    assert!(matches!(
        s.enqueue_upload(
            Category::Hydration,
            date(2026, 10, 14),
            99,
            &json!({"ml": 1})
        ),
        Err(Error::UploadBlocked(_))
    ));
    s.disable_sharing(Category::Hydration).unwrap();
    clock.advance(Duration::seconds(1));
    enable(&mut s, Category::Hydration, date(2026, 10, 1));
    assert_eq!(
        s.upload_block(Category::Hydration, date(2026, 10, 14))
            .unwrap(),
        Some(UploadBlock::Tombstoned)
    );
    // Other categories are unaffected.
    enable(&mut s, Category::Outcomes, date(2026, 10, 1));
    assert_eq!(
        s.upload_block(Category::Outcomes, date(2026, 10, 14))
            .unwrap(),
        None
    );
    // A deletion including today blocks today too (sharing resumes tomorrow).
    s.record_deletion(Category::Outcomes, DateRange::day(today))
        .unwrap();
    assert_eq!(
        s.upload_block(Category::Outcomes, today).unwrap(),
        Some(UploadBlock::Tombstoned)
    );
    assert_eq!(
        s.upload_block(Category::Outcomes, date(2026, 10, 22))
            .unwrap(),
        None
    );
}

#[test]
fn all_dates_deletion_blocks_everything_for_that_category() {
    let (mut s, _) = store();
    enable(&mut s, Category::WorkSession, date(2026, 1, 1));
    s.record_deletion(Category::WorkSession, DateRange::all())
        .unwrap();
    for d in [date(2026, 1, 1), date(2026, 10, 21), date(2030, 6, 1)] {
        assert_eq!(
            s.upload_block(Category::WorkSession, d).unwrap(),
            Some(UploadBlock::Tombstoned)
        );
    }
}

#[test]
fn schema_triggers_guard_the_queue_even_without_this_api() {
    let (mut s, _) = store();
    let today = date(2026, 10, 21);
    let me = s.device_id().to_string();
    let insert = |s: &Store, category: &str, origin: &str, d: &str| {
        s.conn.execute(
            "INSERT INTO outbox (category, origin_device, local_date, revision, payload, created_at)
             VALUES (?1, ?2, ?3, 1, '{}', 0)",
            params![category, origin, d],
        )
    };
    let blocked = |r: rusqlite::Result<usize>, why: &str| {
        let e = map_upload_error(r.unwrap_err());
        assert!(matches!(&e, Error::UploadBlocked(m) if m == why), "{e}");
    };
    blocked(insert(&s, "hydration", &me, "2026-10-21"), "category off");
    enable(&mut s, Category::Hydration, today);
    blocked(
        insert(&s, "hydration", &me, "2026-10-20"),
        "before share_from",
    );
    let other = Uuid::now_v7().to_string();
    blocked(
        insert(&s, "hydration", &other, "2026-10-21"),
        "not recorded by this PC",
    );
    s.record_deletion(Category::Hydration, DateRange::day(today))
        .unwrap();
    blocked(insert(&s, "hydration", &me, "2026-10-21"), "tombstoned");
    insert(&s, "hydration", &me, "2026-10-22").unwrap();
    // Queued rows are never re-stamped.
    let err = s
        .conn
        .execute("UPDATE outbox SET local_date = '2026-10-21'", [])
        .unwrap_err();
    assert!(err.to_string().contains("re-stamped"), "{err}");
    // Tombstones cannot be removed or narrowed.
    let err = s.conn.execute("DELETE FROM tombstones", []).unwrap_err();
    assert!(err.to_string().contains("permanent"), "{err}");
    let err = s
        .conn
        .execute("UPDATE tombstones SET to_date = from_date", [])
        .unwrap_err();
    assert!(err.to_string().contains("cannot change"), "{err}");
    // Their acknowledgement state can change.
    s.conn
        .execute(
            "UPDATE tombstones SET state = 'acknowledged', server_seq = 7",
            [],
        )
        .unwrap();
    // A later share_from discards older queued rows.
    s.conn
        .execute(
            "UPDATE sharing_policy SET share_from = '2026-10-23' WHERE category = 'hydration'",
            [],
        )
        .unwrap();
    assert!(s.outbox().unwrap().is_empty());
    // Policy rows cannot be deleted; the consent log cannot be edited.
    assert!(s.conn.execute("DELETE FROM sharing_policy", []).is_err());
    assert!(
        s.conn
            .execute("UPDATE consent_log SET kind = 'enabled'", [])
            .is_err()
    );
}

// ------------------------------- operation order and response freshness

fn file_store(path: &std::path::Path, clock: &Arc<ManualClock>) -> Store {
    Store::open(path, clock.clone()).unwrap()
}

fn assert_off_and_nothing_queued(s: &mut Store, c: Category, d: NaiveDate) {
    assert!(
        !s.sharing_policy(c).unwrap().enabled,
        "sharing must stay off"
    );
    assert!(s.outbox().unwrap().is_empty(), "nothing may be queued");
    assert!(matches!(
        s.enqueue_upload(c, d, 1, &json!({"ml": 1})),
        Err(Error::UploadBlocked(_))
    ));
    assert!(s.outbox().unwrap().is_empty());
}

#[test]
fn old_enable_cannot_reenable_after_restart_with_clock_rollback() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hy.db");
    let clock = Arc::new(ManualClock::new(start()));
    let today = date(2026, 10, 21);
    let enable_op = {
        let mut s = file_store(&path, &clock);
        s.request_enable(Category::Hydration, "notice-1").unwrap()
    };
    // The PC restarts with its clock set back an hour, then the user switches off.
    clock.shift_wall(Duration::hours(-1));
    let mut s = file_store(&path, &clock);
    let disable_op = s.disable_sharing(Category::Hydration).unwrap();
    // The identifiers now sort the wrong way round; the durable order does not.
    assert!(
        disable_op < enable_op,
        "wall-time ids are misordered by the rollback"
    );
    let (e, d) = (
        s.policy_op(enable_op).unwrap().unwrap(),
        s.policy_op(disable_op).unwrap().unwrap(),
    );
    assert!(d.local_seq > e.local_seq);
    // The old acceptance arrives.
    assert_eq!(
        s.accept_enable(enable_op, 1, today).unwrap(),
        EnableOutcome::SupersededByLocalDisable
    );
    assert_off_and_nothing_queued(&mut s, Category::Hydration, today);
    // Still off after another restart.
    drop(s);
    let mut s = file_store(&path, &clock);
    assert_off_and_nothing_queued(&mut s, Category::Hydration, today);
}

#[test]
fn old_enable_cannot_reenable_after_same_millisecond_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hy.db");
    let clock = Arc::new(ManualClock::new(start())); // never advanced
    let today = date(2026, 10, 21);
    let enable_op = {
        let mut s = file_store(&path, &clock);
        s.request_enable(Category::Outcomes, "notice-1").unwrap()
    };
    let mut s = file_store(&path, &clock);
    s.disable_sharing(Category::Outcomes).unwrap();
    assert_eq!(
        s.accept_enable(enable_op, 1, today).unwrap(),
        EnableOutcome::SupersededByLocalDisable
    );
    assert_off_and_nothing_queued(&mut s, Category::Outcomes, today);
}

#[test]
fn enable_requested_after_a_disable_is_applied_despite_clock_rollback() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hy.db");
    let clock = Arc::new(ManualClock::new(start()));
    {
        let mut s = file_store(&path, &clock);
        s.disable_sharing(Category::Hydration).unwrap();
    }
    clock.shift_wall(Duration::hours(-3));
    let mut s = file_store(&path, &clock);
    let op = s.request_enable(Category::Hydration, "notice-1").unwrap();
    assert_eq!(
        s.accept_enable(op, 1, date(2026, 10, 21)).unwrap(),
        EnableOutcome::Applied
    );
    assert!(s.sharing_policy(Category::Hydration).unwrap().enabled);
}

#[test]
fn policy_op_order_is_durable_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hy.db");
    let clock = Arc::new(ManualClock::new(start()));
    let mut seqs = Vec::new();
    for round in 0..3 {
        let mut s = file_store(&path, &clock);
        let a = s.request_enable(Category::Hydration, "notice-1").unwrap();
        let b = s.disable_sharing(Category::PhoneReports).unwrap();
        seqs.push(s.policy_op(a).unwrap().unwrap().local_seq);
        seqs.push(s.policy_op(b).unwrap().unwrap().local_seq);
        clock.shift_wall(Duration::minutes(-10 * (round + 1)));
    }
    assert_eq!(seqs, vec![1, 2, 3, 4, 5, 6]);
}

#[test]
fn a_failed_operation_does_not_consume_an_order_number() {
    let (mut s, _) = store();
    s.conn
        .execute_batch(
            "CREATE TEMP TRIGGER fail_consent BEFORE INSERT ON consent_log
             BEGIN SELECT RAISE(ABORT, 'injected failure'); END;",
        )
        .unwrap();
    assert!(s.request_enable(Category::Hydration, "notice-1").is_err());
    s.conn
        .execute_batch("DROP TRIGGER temp.fail_consent")
        .unwrap();
    let op = s.request_enable(Category::Hydration, "notice-1").unwrap();
    assert_eq!(s.policy_op(op).unwrap().unwrap().local_seq, 1);
}

#[test]
fn out_of_order_enable_responses_never_roll_back_policy() {
    let (mut s, clock) = store();
    let older = s.request_enable(Category::Hydration, "notice-1").unwrap();
    clock.advance(Duration::minutes(1));
    let newer = s.request_enable(Category::Hydration, "notice-1").unwrap();
    // The server sequenced them 10 and 20; the responses arrive newest first.
    assert_eq!(
        s.accept_enable(newer, 20, date(2026, 10, 22)).unwrap(),
        EnableOutcome::Applied
    );
    assert_eq!(
        s.accept_enable(older, 10, date(2026, 10, 1)).unwrap(),
        EnableOutcome::StaleResponse
    );
    let p = s.sharing_policy(Category::Hydration).unwrap();
    assert!(p.enabled);
    assert_eq!(p.seq, 20);
    assert_eq!(p.share_from, Some(date(2026, 10, 22)));
    // The older acknowledgement is recorded on its operation all the same.
    let o = s.policy_op(older).unwrap().unwrap();
    assert_eq!((o.state, o.server_seq), (PolicyOpState::Accepted, Some(10)));
    // Dates before the newer share_from stay blocked.
    for d in [date(2026, 10, 1), date(2026, 10, 15), date(2026, 10, 21)] {
        assert_eq!(
            s.upload_block(Category::Hydration, d).unwrap(),
            Some(UploadBlock::BeforeShareFrom),
            "{d}"
        );
        assert!(matches!(
            s.enqueue_upload(Category::Hydration, d, 1, &json!({"ml": 1})),
            Err(Error::UploadBlocked(_))
        ));
    }
    assert!(s.outbox().unwrap().is_empty());
    s.enqueue_upload(
        Category::Hydration,
        date(2026, 10, 22),
        1,
        &json!({"ml": 1}),
    )
    .unwrap();
    // Only one "enabled" consent entry and one sharing event were written.
    let enabled = s
        .consent_log()
        .unwrap()
        .iter()
        .filter(|c| c.kind == ConsentKind::Enabled)
        .count();
    assert_eq!(enabled, 1);
}

#[test]
fn in_order_enable_responses_move_share_from_forward() {
    let (mut s, clock) = store();
    let older = s.request_enable(Category::Hydration, "notice-1").unwrap();
    clock.advance(Duration::minutes(1));
    let newer = s.request_enable(Category::Hydration, "notice-1").unwrap();
    assert_eq!(
        s.accept_enable(older, 10, date(2026, 10, 1)).unwrap(),
        EnableOutcome::Applied
    );
    s.enqueue_upload(
        Category::Hydration,
        date(2026, 10, 15),
        1,
        &json!({"ml": 1}),
    )
    .unwrap();
    assert_eq!(
        s.accept_enable(newer, 20, date(2026, 10, 22)).unwrap(),
        EnableOutcome::Applied
    );
    let p = s.sharing_policy(Category::Hydration).unwrap();
    assert_eq!((p.seq, p.share_from), (20, Some(date(2026, 10, 22))));
    assert!(
        s.outbox().unwrap().is_empty(),
        "records before the new share_from are discarded"
    );
}

#[test]
fn a_response_with_an_already_applied_seq_is_stale() {
    let (mut s, clock) = store();
    let a = s.request_enable(Category::WorkSession, "notice-1").unwrap();
    clock.advance(Duration::seconds(1));
    let b = s.request_enable(Category::WorkSession, "notice-1").unwrap();
    s.accept_enable(a, 7, date(2026, 10, 20)).unwrap();
    assert_eq!(
        s.accept_enable(b, 7, date(2026, 10, 1)).unwrap(),
        EnableOutcome::StaleResponse
    );
    assert_eq!(
        s.sharing_policy(Category::WorkSession).unwrap().share_from,
        Some(date(2026, 10, 20))
    );
}

#[test]
fn a_stale_response_cannot_reenable_after_a_disable() {
    let (mut s, clock) = store();
    let a = s.request_enable(Category::Hydration, "notice-1").unwrap();
    let b = s.request_enable(Category::Hydration, "notice-1").unwrap();
    s.accept_enable(b, 20, date(2026, 10, 22)).unwrap();
    clock.advance(Duration::seconds(1));
    s.disable_sharing(Category::Hydration).unwrap();
    assert_eq!(
        s.accept_enable(a, 10, date(2026, 10, 1)).unwrap(),
        EnableOutcome::StaleResponse
    );
    let p = s.sharing_policy(Category::Hydration).unwrap();
    assert!(!p.enabled);
    assert_eq!((p.seq, p.share_from), (20, Some(date(2026, 10, 22))));
}

// --------------------------------------------------------------- retention

#[test]
fn prune_applies_retention_and_keeps_corrections_consistent() {
    let (mut s, _) = store();
    let today = date(2026, 10, 21);
    let me = s.device_id();
    let old = start() - Duration::days(91);
    let kept = start() - Duration::days(90);
    let old_entry = s.append_event(water(100).at(old)).unwrap();
    // A correction recorded today but moved to a kept date.
    let moved = s
        .append_event(water(150).superseding(old_entry.id).at(kept))
        .unwrap();
    s.append_event(water(200).at(kept)).unwrap();
    s.upsert_daily_summary(&daily(me, old_entry.local_date, 1, 1))
        .unwrap();
    s.upsert_daily_summary(&daily(me, today, 1, 1)).unwrap();
    s.upsert_imported_summary(
        &daily(me, old_entry.local_date, 1, 1),
        ImportProvenance::ServerRestore,
    )
    .unwrap();
    let month = |y, m| Summary {
        category: Category::Hydration,
        origin_device: me,
        period: MonthKey::new(y, m).unwrap(),
        revision: 1,
        coverage: None,
        payload: json!({}),
    };
    s.upsert_monthly_summary(&month(2024, 9)).unwrap();
    s.upsert_monthly_summary(&month(2024, 10)).unwrap();

    let report = s.prune(today).unwrap();
    assert_eq!(
        report,
        PruneReport {
            events: 1,
            daily: 1,
            imported: 1,
            monthly: 1
        }
    );
    // The moved correction survives and now stands alone.
    let survivor = s.event(moved.id).unwrap().unwrap();
    assert_eq!(survivor.supersedes, None);
    assert_eq!(water_total(&s, survivor.local_date), 350);
    assert_eq!(s.monthly_summaries(Category::Hydration).unwrap().len(), 1);
    // Pruning again removes nothing.
    assert_eq!(s.prune(today).unwrap(), PruneReport::default());
}

/// Water totals for every date from `from` to `to`.
fn totals(s: &Store, from: NaiveDate, to: NaiveDate) -> Vec<(NaiveDate, u32)> {
    from.iter_days()
        .take_while(|d| *d <= to)
        .map(|d| (d, water_total(s, d)))
        .collect()
}

/// Prune and check that every kept date's current view is unchanged.
fn prune_preserves_current_view(s: &mut Store) {
    let today = date(2026, 10, 21);
    let cutoff = date(2026, 7, 23); // today minus 90 days: the oldest kept date
    let before = totals(s, cutoff, today);
    let current_before: Vec<Vec<Uuid>> = cutoff
        .iter_days()
        .take_while(|d| *d <= today)
        .map(|d| {
            s.current_events_on(d)
                .unwrap()
                .iter()
                .map(|e| e.id)
                .collect()
        })
        .collect();
    s.prune(today).unwrap();
    assert_eq!(
        totals(s, cutoff, today),
        before,
        "totals changed on a kept date"
    );
    let current_after: Vec<Vec<Uuid>> = cutoff
        .iter_days()
        .take_while(|d| *d <= today)
        .map(|d| {
            s.current_events_on(d)
                .unwrap()
                .iter()
                .map(|e| e.id)
                .collect()
        })
        .collect();
    assert_eq!(
        current_after, current_before,
        "current view changed on a kept date"
    );
    assert!(
        s.events_on(date(2026, 7, 22)).unwrap().is_empty(),
        "expired date kept"
    );
}

fn expired_time() -> DateTime<Utc> {
    start() - Duration::days(91) // 22 Jul 2026, expired
}

fn kept_time() -> DateTime<Utc> {
    start() - Duration::days(90) // 23 Jul 2026, the oldest kept date
}

#[test]
fn prune_does_not_resurrect_an_entry_whose_correction_moved_it_earlier() {
    let (mut s, _) = store();
    let a = s.append_event(water(250).at(kept_time())).unwrap();
    let b = s
        .append_event(water(300).superseding(a.id).at(expired_time()))
        .unwrap();
    assert_eq!(b.local_date, date(2026, 7, 22));
    assert_eq!(water_total(&s, a.local_date), 0);
    prune_preserves_current_view(&mut s);
    assert_eq!(
        water_total(&s, a.local_date),
        0,
        "the corrected entry came back"
    );
    assert!(s.event(a.id).unwrap().is_none());
    assert!(s.event(b.id).unwrap().is_none());
}

#[test]
fn prune_keeps_a_correction_that_moved_an_entry_later() {
    let (mut s, _) = store();
    let a = s.append_event(water(250).at(expired_time())).unwrap();
    let b = s
        .append_event(water(400).superseding(a.id).at(kept_time()))
        .unwrap();
    prune_preserves_current_view(&mut s);
    assert_eq!(water_total(&s, b.local_date), 400);
    assert_eq!(s.event(b.id).unwrap().unwrap().supersedes, None);
}

#[test]
fn prune_is_chain_safe_for_repeated_corrections_and_retractions() {
    let (mut s, _) = store();
    let later = kept_time() + Duration::days(1);
    // Kept, then corrected onto an expired date, then corrected back onto a kept date.
    let a1 = s.append_event(water(100).at(kept_time())).unwrap();
    let a2 = s
        .append_event(water(200).superseding(a1.id).at(expired_time()))
        .unwrap();
    let a3 = s
        .append_event(water(300).superseding(a2.id).at(later))
        .unwrap();
    // Kept, corrected onto an expired date, then retracted there.
    let b1 = s.append_event(water(150).at(later)).unwrap();
    let b2 = s
        .append_event(water(175).superseding(b1.id).at(expired_time()))
        .unwrap();
    s.append_event(NewEvent::manual(EventPayload::EntryRetracted {}).superseding(b2.id))
        .unwrap();
    // A chain entirely inside the kept window, with an undo.
    let c1 = s.append_event(water(500).at(later)).unwrap();
    let c2 = s.append_event(water(450).superseding(c1.id)).unwrap();
    let c3 = s.append_event(water(425).superseding(c2.id)).unwrap();
    let d1 = s.append_event(water(50).at(later)).unwrap();
    s.append_event(NewEvent::manual(EventPayload::EntryRetracted {}).superseding(d1.id))
        .unwrap();
    // A plain kept entry and a plain expired one.
    s.append_event(water(75).at(kept_time())).unwrap();
    s.append_event(water(999).at(expired_time())).unwrap();
    assert_eq!(water_total(&s, later.date_naive()), 300 + 425);

    prune_preserves_current_view(&mut s);

    assert_eq!(water_total(&s, a3.local_date), 300 + 425);
    assert_eq!(water_total(&s, kept_time().date_naive()), 75);
    assert!(s.event(a1.id).unwrap().is_none() && s.event(b1.id).unwrap().is_none());
    // The fully kept chain keeps its whole history.
    for id in [c1.id, c2.id, c3.id] {
        assert!(s.event(id).unwrap().is_some());
    }
    assert_eq!(s.event(a3.id).unwrap().unwrap().supersedes, None);
}
