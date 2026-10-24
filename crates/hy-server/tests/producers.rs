//! The `outcomes` and `work_session` records (V1 categories; V1-5 contract 8.2): built on
//! the companion from its own reminders, routines and tracker tallies, sent through the
//! real sync flow and stored by the real server. Round trip, consent, the confirmation
//! date, corrections and provenance (T-L7). `posture_summary` stays V2.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use chrono::{DateTime, Duration, NaiveDate, Utc};
use common::{Harness, Pc, new_id};
use hy_core::clock::Clock;
use hy_core::coach::DayTally;
use hy_core::link::flow::SyncOutcome;
use hy_core::protocol::share;
use hy_core::protocol::wire::{RowOutcome, UploadRequest, UploadRow, WireError};
use hy_core::schema::{
    Activity, Category, EventPayload, NewEvent, ReminderOutcome, RoutineOutcome, RoutineVariant,
};
use hy_core::store::CoachCommit;
use serde_json::json;
use uuid::Uuid;

/// `minute` minutes after 08:00 IST on the day `offset` days from now (every test
/// starts at 11:30 IST, so the first 210 minutes are already past).
fn at(h: &Harness, offset: i64, minute: i64) -> DateTime<Utc> {
    let d = h.clock.now().date_naive() + Duration::days(offset);
    d.and_hms_opt(2, 30, 0).unwrap().and_utc() + Duration::minutes(minute)
}

fn date(h: &Harness, offset: i64) -> NaiveDate {
    h.clock.now().date_naive() + Duration::days(offset)
}

fn text(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

async fn enable(pc: &Pc, c: Category) {
    pc.with(|s| s.request_enable(c, "v1-5-sharing-1").unwrap());
    pc.synced().await;
    assert!(pc.with(|s| s.sharing_policy(c).unwrap().enabled), "{c:?}");
}

/// One engine step on `pc`: these events and tallies, committed as the coach does.
fn step(pc: &Pc, events: &[NewEvent], days: &[(NaiveDate, DayTally)]) {
    pc.with(|s| {
        let expected = s.coach_state().unwrap().map(|c| c.revision);
        s.commit_coach_step(CoachCommit {
            expected_revision: expected,
            format: 1,
            engine_time_ms: 0,
            state: "{}",
            events,
            days,
        })
        .unwrap();
    });
}

/// A reminder offered at `t`, answered with `outcome` (if any) a minute later.
fn reminder(pc: &Pc, t: DateTime<Utc>, activity: Activity, outcome: Option<ReminderOutcome>) {
    let id = new_id();
    let mut events = vec![
        NewEvent::system(EventPayload::ReminderOffered {
            reminder_id: id,
            activity,
        })
        .at(t),
    ];
    if let Some(outcome) = outcome {
        events.push(
            NewEvent::manual(EventPayload::ReminderResponse {
                reminder_id: id,
                activity,
                outcome,
            })
            .at(t + Duration::minutes(1)),
        );
    }
    step(pc, &events, &[]);
}

/// A movement routine ended at `t` with `outcome` (four steps).
fn routine(pc: &Pc, t: DateTime<Utc>, outcome: RoutineOutcome) {
    let steps_done = match outcome {
        RoutineOutcome::Done => 4,
        RoutineOutcome::Partial => 2,
        RoutineOutcome::None | RoutineOutcome::Skipped => 0,
    };
    let variant = (outcome != RoutineOutcome::Skipped).then_some(RoutineVariant::Seated);
    pc.with(|s| {
        s.append_event(
            NewEvent::manual(EventPayload::MovementSession {
                session_id: new_id(),
                routine_id: "desk-reset".into(),
                routine_version: 1,
                variant,
                outcome,
                steps_done,
                steps_skipped: 0,
                steps_total: 4,
            })
            .at(t),
        )
        .unwrap();
    });
}

/// The tracker's tallies for `d`, in minutes.
fn tally(pc: &Pc, d: NaiveDate, work: u64, quiet: u64, longest: u64, focus: u32, pomodoro: u32) {
    let t = DayTally {
        pc_active_ms: (work + quiet) * 60_000,
        work_active_ms: work * 60_000,
        work_quiet_ms: quiet * 60_000,
        longest_block_ms: longest * 60_000,
        focus_sessions: focus,
        pomodoro_rounds: pomodoro,
        ..DayTally::default()
    };
    step(pc, &[], &[(d, t)]);
}

/// The server's row for `origin`, `category` and `d`: (revision, payload).
fn server_row(
    h: &Harness,
    origin: Uuid,
    category: Category,
    d: NaiveDate,
) -> Option<(i64, serde_json::Value)> {
    h.server
        .db()
        .query_row(
            "SELECT revision, payload FROM summaries
             WHERE origin_device = ?1 AND category = ?2 AND local_date = ?3",
            rusqlite::params![origin.as_bytes().as_slice(), category.as_str(), text(d)],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
        )
        .ok()
        .map(|(rev, p)| (rev, serde_json::from_str(&p).unwrap()))
}

fn rows_of(h: &Harness, origin: Uuid, category: Category) -> i64 {
    h.server
        .db()
        .query_row(
            "SELECT COUNT(*) FROM summaries WHERE origin_device = ?1 AND category = ?2",
            rusqlite::params![origin.as_bytes().as_slice(), category.as_str()],
            |r| r.get(0),
        )
        .unwrap()
}

/// A day of reminders on `pc`: six offers covering every answer and one unanswered,
/// a partial routine then a done one, and the tracker's tallies.
fn busy_day(h: &Harness, pc: &Pc, offset: i64) {
    reminder(
        pc,
        at(h, offset, 0),
        Activity::Movement,
        Some(ReminderOutcome::Started),
    );
    reminder(
        pc,
        at(h, offset, 30),
        Activity::EyeRest,
        Some(ReminderOutcome::Snoozed),
    );
    reminder(
        pc,
        at(h, offset, 60),
        Activity::Water,
        Some(ReminderOutcome::Skipped),
    );
    reminder(
        pc,
        at(h, offset, 90),
        Activity::Movement,
        Some(ReminderOutcome::Dismissed),
    );
    reminder(
        pc,
        at(h, offset, 120),
        Activity::EyeRest,
        Some(ReminderOutcome::NoResponse),
    );
    reminder(
        pc,
        at(h, offset, 150),
        Activity::Water,
        Some(ReminderOutcome::Done),
    );
    reminder(pc, at(h, offset, 180), Activity::Movement, None);
    routine(pc, at(h, offset, 5), RoutineOutcome::Partial);
    routine(pc, at(h, offset, 200), RoutineOutcome::Done);
    tally(pc, date(h, offset), 200, 40, 70, 1, 3);
}

#[test]
fn every_v1_category_is_produced_and_posture_stays_v2() {
    for c in [
        Category::Hydration,
        Category::Outcomes,
        Category::WorkSession,
        Category::PhoneReports,
    ] {
        assert!(share::produced(c), "{c:?}");
    }
    assert!(!share::produced(Category::PostureSummary));
}

#[tokio::test(flavor = "multi_thread")]
async fn outcomes_and_work_session_round_trip_through_the_server() {
    let h = Harness::start().await;
    let u = h.user("u1");
    let a = h.pc(&u, "PC-A").await;
    let b = h.pc(&u, "PC-B").await;
    a.synced().await;
    b.synced().await;
    enable(&a, Category::Outcomes).await;
    enable(&a, Category::WorkSession).await;
    b.synced().await;
    busy_day(&h, &a, 0);
    let d0 = date(&h, 0);

    let outcomes = json!({
        "v": 1, "offered": 7, "done": 2, "snoozed": 1, "skipped": 2, "no_response": 1,
        "routine": "done"
    });
    let work = json!({
        "v": 1, "active_min": 240, "longest_block_min": 70, "focus_count": 1, "pomodoro_count": 3
    });
    // The preview is exactly the record that is sent.
    assert_eq!(
        a.with(|s| s.share_record(Category::Outcomes, d0).unwrap()),
        Some(outcomes.clone())
    );
    assert_eq!(
        a.with(|s| s.share_record(Category::WorkSession, d0).unwrap()),
        Some(work.clone())
    );
    let out = a.synced().await;
    assert!(
        matches!(
            out,
            SyncOutcome::Synced {
                uploaded: 2,
                rejected: 0,
                ..
            }
        ),
        "{out:?}"
    );
    assert_eq!(
        server_row(&h, a.origin(), Category::Outcomes, d0).map(|r| r.1),
        Some(outcomes.clone())
    );
    assert_eq!(
        server_row(&h, a.origin(), Category::WorkSession, d0).map(|r| r.1),
        Some(work.clone())
    );
    // Nothing else of the day reaches the server: no times, no reminder text, no
    // hydration or phone row for categories that are off.
    assert_eq!(h.count("summaries"), 2);

    // B restores from the server: A's rows arrive keyed by A's origin, read-only.
    let n =
        hy_core::link::flow::restore_from_server(&b.store, &*b.dev.vault, common::opts(), d0, d0)
            .await
            .unwrap();
    assert_eq!(n, 2);
    for (c, payload) in [
        (Category::Outcomes, &outcomes),
        (Category::WorkSession, &work),
    ] {
        let (row, _) = b
            .with(|s| s.imported_summary(c, a.origin(), d0).unwrap())
            .expect("imported");
        assert_eq!(&row.payload, payload);
    }
    // B has nothing of its own for the day and sends nothing.
    assert!(matches!(
        b.synced().await,
        SyncOutcome::Synced { uploaded: 0, .. }
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn with_consent_off_nothing_of_outcomes_or_work_session_is_sent() {
    let h = Harness::start().await;
    let u = h.user("u1");
    let a = h.pc(&u, "PC-A").await;
    a.synced().await;
    busy_day(&h, &a, 0);
    // Both off (the default): nothing is queued or sent.
    let out = a.synced().await;
    assert!(
        matches!(out, SyncOutcome::Synced { uploaded: 0, .. }),
        "{out:?}"
    );
    assert!(a.with(|s| s.outbox().unwrap()).is_empty());
    assert_eq!(h.count("summaries"), 0);

    // Only outcomes on: the work session still sends nothing.
    enable(&a, Category::Outcomes).await;
    assert_eq!(rows_of(&h, a.origin(), Category::Outcomes), 1);
    assert_eq!(rows_of(&h, a.origin(), Category::WorkSession), 0);

    // Off again, offline: the queue is purged at once and nothing more is sent.
    reminder(
        &a,
        at(&h, 0, 210),
        Activity::Water,
        Some(ReminderOutcome::Done),
    );
    a.with(|s| {
        s.queue_share_records(date(&h, 0)).unwrap();
        assert_eq!(s.outbox().unwrap().len(), 1, "queued while offline");
        s.disable_sharing(Category::Outcomes).unwrap();
        assert!(s.outbox().unwrap().is_empty(), "purged by the disable");
    });
    h.clock.advance(Duration::days(1));
    busy_day(&h, &a, 0);
    let out = a.synced().await;
    assert!(
        matches!(out, SyncOutcome::Synced { uploaded: 0, .. }),
        "{out:?}"
    );
    assert_eq!(
        rows_of(&h, a.origin(), Category::Outcomes),
        1,
        "only the first day"
    );
    let (_, first) = server_row(&h, a.origin(), Category::Outcomes, date(&h, -1)).unwrap();
    assert_eq!(first["offered"], 7, "the later answer was never sent");
    assert_eq!(rows_of(&h, a.origin(), Category::WorkSession), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pc_confirmed_later_sends_outcomes_and_work_session_only_from_its_confirmation_date() {
    let h = Harness::start().await;
    let u = h.user("u1");
    let a = h.pc(&u, "PC-A").await;
    a.synced().await;
    enable(&a, Category::Outcomes).await;
    enable(&a, Category::WorkSession).await;
    // B pairs today but is confirmed only tomorrow; it is used on both days.
    let b = h.pending_pc(&u, "PC-B").await;
    busy_day(&h, &b, 0);
    let d0 = date(&h, 0);
    h.clock.advance(Duration::days(1));
    let d1 = date(&h, 0);
    h.confirm(&b.dev);
    busy_day(&h, &b, 0);
    b.synced().await;
    let out = b.synced().await;
    assert!(matches!(out, SyncOutcome::Synced { .. }), "{out:?}");
    for c in [Category::Outcomes, Category::WorkSession] {
        assert!(
            server_row(&h, b.origin(), c, d1).is_some(),
            "{c:?} from its confirmation date"
        );
        assert!(
            server_row(&h, b.origin(), c, d0).is_none(),
            "{c:?} never before it"
        );
        // The PC itself refuses to queue the earlier day.
        assert!(b.with(|s| s.enqueue_upload(c, d0, 9, &json!({"v": 1})).is_err()));
    }
    // A forced upload of the earlier day is refused by the server.
    let record = b.with(|s| s.share_record(Category::Outcomes, d0).unwrap().unwrap());
    let rows = vec![UploadRow {
        category: Category::Outcomes,
        origin_device: b.origin(),
        local_date: text(d0),
        revision: 9,
        payload: record,
    }];
    let results = b
        .client(&h)
        .upload(&b.dev.token, &UploadRequest { rows })
        .await
        .unwrap()
        .results;
    assert_eq!(
        results,
        vec![RowOutcome::Rejected {
            error: WireError::BeforeShareFrom
        }]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn corrections_to_outcomes_and_work_session_replace_the_shared_day() {
    let h = Harness::start().await;
    let u = h.user("u1");
    let a = h.pc(&u, "PC-A").await;
    a.synced().await;
    enable(&a, Category::Outcomes).await;
    enable(&a, Category::WorkSession).await;
    let d0 = date(&h, 0);
    // Morning: one reminder not yet answered, half an hour of work.
    let id = new_id();
    step(
        &a,
        &[NewEvent::system(EventPayload::ReminderOffered {
            reminder_id: id,
            activity: Activity::Movement,
        })
        .at(at(&h, 0, 0))],
        &[],
    );
    tally(&a, d0, 30, 0, 30, 0, 0);
    a.synced().await;
    let (r1, o1) = server_row(&h, a.origin(), Category::Outcomes, d0).unwrap();
    assert_eq!(
        o1,
        json!({"v": 1, "offered": 1, "done": 0, "snoozed": 0, "skipped": 0, "no_response": 0, "routine": null})
    );
    let (w1, _) = server_row(&h, a.origin(), Category::WorkSession, d0).unwrap();

    // Later: the reminder is answered, the routine done, and the tallies grow (the
    // tracker rewrites the day). The day's rows are replaced with higher revisions.
    step(
        &a,
        &[NewEvent::manual(EventPayload::ReminderResponse {
            reminder_id: id,
            activity: Activity::Movement,
            outcome: ReminderOutcome::Started,
        })
        .at(at(&h, 0, 1))],
        &[],
    );
    routine(&a, at(&h, 0, 10), RoutineOutcome::Done);
    tally(&a, d0, 95, 25, 80, 2, 1);
    let out = a.synced().await;
    assert!(
        matches!(out, SyncOutcome::Synced { uploaded: 2, .. }),
        "{out:?}"
    );
    let (r2, o2) = server_row(&h, a.origin(), Category::Outcomes, d0).unwrap();
    let (w2, wp) = server_row(&h, a.origin(), Category::WorkSession, d0).unwrap();
    assert!(r2 > r1 && w2 > w1, "higher revisions replace the day");
    assert_eq!(
        o2,
        json!({"v": 1, "offered": 1, "done": 1, "snoozed": 0, "skipped": 0, "no_response": 0, "routine": "done"})
    );
    assert_eq!(
        wp,
        json!({"v": 1, "active_min": 120, "longest_block_min": 80, "focus_count": 2, "pomodoro_count": 1})
    );
    assert_eq!(h.count("summaries"), 2, "one row per day and category");
    // Unchanged since: nothing is sent again.
    assert!(matches!(
        a.synced().await,
        SyncOutcome::Synced { uploaded: 0, .. }
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn outcomes_and_tallies_from_another_pcs_backup_are_never_sent_under_this_pcs_id() {
    // T-L7 for the two categories: reminders carry their origin; tracker tallies do
    // not, so every day they cover is withheld after another PC's backup.
    let h = Harness::start().await;
    let u = h.user("u1");
    let a = h.pc(&u, "PC-A").await;
    a.synced().await;
    busy_day(&h, &a, 0);
    let d0 = date(&h, 0);
    let contents = a.with(|s| s.backup_contents().unwrap());
    let n = h.pc(&u, "PC-N").await;
    n.synced().await;
    n.with(|s| s.import_contents(&contents).unwrap());
    let (_, again) = h
        .pair_as(&h.slip(&u), "PC-N", n.origin(), common::opts())
        .await
        .unwrap();
    n.with(|s| s.save_pairing(&again.link, "v1-5-notice-1").unwrap());
    h.confirm(&again);
    let n = Pc {
        dev: again,
        store: n.store,
    };
    n.synced().await;
    enable(&n, Category::Outcomes).await;
    enable(&n, Category::WorkSession).await;
    for c in [Category::Outcomes, Category::WorkSession] {
        assert_eq!(n.with(|s| s.share_record(c, d0).unwrap()), None, "{c:?}");
    }
    assert!(matches!(
        n.synced().await,
        SyncOutcome::Synced { uploaded: 0, .. }
    ));
    assert_eq!(h.count("summaries"), 0);
    // The next day's own use is shared as usual.
    h.clock.advance(Duration::days(1));
    busy_day(&h, &n, 0);
    let out = n.synced().await;
    assert!(
        matches!(out, SyncOutcome::Synced { uploaded: 2, .. }),
        "{out:?}"
    );
    assert_eq!(rows_of(&h, n.origin(), Category::Outcomes), 1);
    assert_eq!(rows_of(&h, n.origin(), Category::WorkSession), 1);
}
