//! Regressions for the Codex review of PR 17 (slice V1-5b): each test reproduces one
//! finding against the real server and the companion's real sync flow.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, NaiveDate, Utc};
use common::{Harness, Pc, new_id, opts, tls_connect};
use hy_core::clock::Clock;
use hy_core::link::flow::{FlowError, SyncOutcome};
use hy_core::protocol::b64;
use hy_core::protocol::wire::{
    ControlOp, ControlRequest, RowOutcome, UploadRequest, UploadRow, WireError,
};
use hy_core::schema::{Category, DateRange, EventPayload, NewEvent, PhoneLabel};
use hy_server::mirror::{VolumeError, VolumeId, check_volume, check_volume_with};
use hy_server::server::Faults;
use uuid::Uuid;

fn at(h: &Harness, offset: i64) -> DateTime<Utc> {
    (h.clock.now().date_naive() + Duration::days(offset))
        .and_hms_opt(5, 0, 0)
        .unwrap()
        .and_utc()
}

fn today(h: &Harness) -> NaiveDate {
    h.clock.now().date_naive()
}

fn text(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

fn rows_of(h: &Harness, origin: Uuid) -> Vec<String> {
    let conn = h.server.db();
    let mut stmt = conn
        .prepare(
            "SELECT category || ':' || local_date FROM summaries WHERE origin_device = ?1
             ORDER BY local_date, category",
        )
        .unwrap();
    stmt.query_map([origin.as_bytes().as_slice()], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn entries_of_kind(h: &Harness, user: &[u8; 16], kind: u8) -> i64 {
    h.server
        .db()
        .query_row(
            "SELECT COUNT(*) FROM control_log WHERE user_id = ?1 AND kind = ?2",
            rusqlite::params![user.as_slice(), kind],
            |r| r.get(0),
        )
        .unwrap()
}

fn hydration_row(pc: &Pc, d: NaiveDate, revision: u32) -> UploadRow {
    UploadRow {
        category: Category::Hydration,
        origin_device: pc.origin(),
        local_date: text(d),
        revision,
        payload: serde_json::json!({ "v": 1, "ml": 250, "goal_ml": null, "entries": 1, "coverage": null }),
    }
}

async fn upload(h: &Harness, pc: &Pc, rows: Vec<UploadRow>) -> Result<Vec<RowOutcome>, String> {
    pc.client(h)
        .upload(&pc.dev.token, &UploadRequest { rows })
        .await
        .map(|r| r.results)
        .map_err(|e| format!("{e:?}"))
}

async fn enable(pc: &Pc, c: Category) {
    pc.with(|s| s.request_enable(c, "v1-5-sharing-1").unwrap());
    pc.synced().await;
    pc.synced().await;
    assert!(pc.with(|s| s.sharing_policy(c).unwrap().enabled));
}

fn phone(pc: &Pc, when: DateTime<Utc>) {
    pc.with(|s| {
        s.append_event(
            NewEvent::manual(EventPayload::PhoneInterruption {
                label: PhoneLabel::Work,
            })
            .at(when),
        )
        .unwrap();
    });
}

// ---------------------------------------------------------------- finding 1

#[tokio::test(flavor = "multi_thread")]
async fn f1_a_failed_second_verification_uploads_nothing_already_queued() {
    let h = Harness::start().await;
    let u = h.user("u1");
    let a = h.pc(&u, "PC-A").await;
    a.synced().await;
    enable(&a, Category::Hydration).await;
    a.water(at(&h, 0), 400);
    // Rows already in the queue before the sync starts.
    let t = today(&h);
    assert!(a.with(|s| s.queue_share_records(t).unwrap()) >= 1);
    let queued = a.with(|s| s.outbox().unwrap().len());
    // A control operation the server accepts, then a second pull whose history does
    // not verify.
    a.with(|s| s.disable_sharing(Category::PhoneReports).unwrap());
    h.server.inject_faults(Faults {
        tamper_next_entries: true,
        ..Faults::default()
    });
    let out = a.synced().await;
    assert_eq!(out, SyncOutcome::HistoryMismatch);
    assert!(rows_of(&h, a.origin()).is_empty(), "zero rows uploaded");
    assert_eq!(
        a.with(|s| s.outbox().unwrap().len()),
        queued,
        "the queue was not sent"
    );
    assert!(a.with(|s| s.server_status().unwrap().mismatch));
    assert!(!a.with(|s| s.uploads_allowed().unwrap()));
    // Only a later pull whose history verifies lets the queue go out.
    assert!(matches!(a.synced().await, SyncOutcome::Synced { .. }));
    assert!(!rows_of(&h, a.origin()).is_empty());
}

// ---------------------------------------------------------------- finding 2

#[tokio::test(flavor = "multi_thread")]
async fn f2_a_revoked_pc_re_paired_to_a_fresh_server_sends_nothing_from_its_old_link() {
    let h1 = Harness::start().await;
    let u1 = h1.user("u1");
    let a = h1.pc(&u1, "PC-A").await;
    a.synced().await;
    // A high policy sequence on server 1.
    for _ in 0..3 {
        enable(&a, Category::Hydration).await;
        a.with(|s| s.disable_sharing(Category::Hydration).unwrap());
        a.synced().await;
    }
    enable(&a, Category::Hydration).await;
    assert!(a.with(|s| s.sharing_policy(Category::Hydration).unwrap().seq) >= 7);
    // Offline: an enable and a deletion wait to be sent; then server 1 revokes A.
    let t = today(&h1);
    a.with(|s| {
        s.request_enable(Category::PhoneReports, "v1-5-sharing-1")
            .unwrap();
        s.record_deletion(Category::Hydration, DateRange { from: t, to: t })
            .unwrap();
    });
    h1.server.revoke_device(&a.dev.device_ref).unwrap();
    assert_eq!(a.sync().await.unwrap_err(), FlowError::Revoked);
    assert!(a.with(|s| s.server_link().unwrap()).is_none());
    // Re-paired with the same store to a fresh server and user.
    let h2 = Harness::start().await;
    let u2 = h2.user("u2");
    let slip = h2.slip(&u2);
    let origin = a.origin();
    let (_, dev) = h2.pair_as(&slip, "PC-A", origin, opts()).await.unwrap();
    let Pc { store, .. } = a;
    store
        .lock()
        .unwrap()
        .save_pairing(&dev.link, "v1-5-notice-1")
        .unwrap();
    let a = Pc { dev, store };
    h2.confirm(&a.dev);
    assert!(matches!(
        a.synced().await,
        SyncOutcome::Synced {
            control_sent: 0,
            ..
        }
    ));
    assert_eq!(
        entries_of_kind(&h2, &u2, 1),
        0,
        "no policy entry on server 2"
    );
    assert_eq!(entries_of_kind(&h2, &u2, 2), 0, "no deletion on server 2");
    for &c in Category::ALL {
        let p = a.with(|s| s.sharing_policy(c).unwrap());
        assert!(!p.enabled && p.seq == 0, "{c:?}");
    }
    // A new choice on server 2 starts from its own sequence and is accepted.
    enable(&a, Category::Hydration).await;
    assert_eq!(entries_of_kind(&h2, &u2, 1), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn f2_self_unpair_also_drops_the_old_links_pending_operations() {
    let h = Harness::start().await;
    let u = h.user("u1");
    let a = h.pc(&u, "PC-A").await;
    a.synced().await;
    a.with(|s| {
        s.request_enable(Category::Hydration, "v1-5-sharing-1")
            .unwrap()
    });
    let link = a.with(|s| s.server_link().unwrap().unwrap());
    hy_core::link::flow::unpair_remote(&link, &*a.dev.vault, opts())
        .await
        .unwrap();
    a.with(|s| s.unpair_locally().unwrap());
    assert!(a.with(|s| s.pending_control_ops().unwrap()).is_empty());
    assert!(!a.with(|s| s.server_status().unwrap().mismatch));
}

// ---------------------------------------------------------------- finding 3

async fn history(h: &Harness, a: &Pc, t: &str) -> Vec<hy_core::protocol::wire::HistoryRow> {
    a.client(h).history(&a.dev.token, t, t).await.unwrap().rows
}

#[tokio::test(flavor = "multi_thread")]
async fn f3_undoing_the_last_entry_of_a_shared_day_corrects_the_server() {
    let h = Harness::start().await;
    let u = h.user("u1");
    let a = h.pc(&u, "PC-A").await;
    a.synced().await;
    enable(&a, Category::Hydration).await;
    enable(&a, Category::PhoneReports).await;
    let water = a.with(|s| {
        s.append_event(NewEvent::manual(EventPayload::WaterLogged { amount_ml: 500 }).at(at(&h, 0)))
            .unwrap()
    });
    phone(&a, at(&h, 0));
    a.synced().await;
    let t = text(today(&h));
    let rows = history(&h, &a, &t).await;
    let ml = rows
        .iter()
        .find(|r| r.category == Category::Hydration)
        .map(|r| r.payload["ml"].clone());
    assert_eq!(ml, Some(serde_json::json!(500)));
    // Undo the only water entry and the only phone report of the day.
    a.with(|s| {
        s.append_event(NewEvent::manual(EventPayload::EntryRetracted {}).superseding(water.id))
            .unwrap();
        let p = s
            .events_on(today(&h))
            .unwrap()
            .into_iter()
            .find(|e| matches!(e.payload, EventPayload::PhoneInterruption { .. }))
            .unwrap();
        s.append_event(NewEvent::manual(EventPayload::EntryRetracted {}).superseding(p.id))
            .unwrap();
    });
    let out = a.synced().await;
    assert!(
        matches!(out, SyncOutcome::Synced { uploaded: 2, .. }),
        "{out:?}"
    );
    let rows = history(&h, &a, &t).await;
    let h_row = rows
        .iter()
        .find(|r| r.category == Category::Hydration)
        .unwrap();
    assert_eq!(h_row.payload["ml"], 0);
    assert_eq!(h_row.payload["entries"], 0);
    let p_row = rows
        .iter()
        .find(|r| r.category == Category::PhoneReports)
        .unwrap();
    assert_eq!(p_row.payload["work"], 0);
    // Nothing more is sent for the emptied day; a never-recorded day sends nothing.
    let out = a.synced().await;
    assert!(
        matches!(out, SyncOutcome::Synced { uploaded: 0, .. }),
        "{out:?}"
    );
    let yesterday = text(today(&h) - Duration::days(1));
    assert!(
        !rows_of(&h, a.origin())
            .iter()
            .any(|r| r.ends_with(&yesterday))
    );
}

// ---------------------------------------------------------------- finding 4

/// Send `path` with a JSON body in two parts, revoking the device in between.
async fn split_request(h: &Harness, pc: &Pc, path: &str, body: &str) -> (u16, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut tls = tls_connect(h.api, h.pin(), 1).await;
    let head = format!(
        "POST {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        b64(pc.dev.token.expose()),
        body.len()
    );
    let (first, rest) = body.split_at(body.len() / 2);
    tls.write_all(head.as_bytes()).await.unwrap();
    tls.write_all(first.as_bytes()).await.unwrap();
    tls.flush().await.unwrap();
    tokio::time::sleep(StdDuration::from_millis(300)).await;
    h.server.revoke_device(&pc.dev.device_ref).unwrap();
    tls.write_all(rest.as_bytes()).await.unwrap();
    tls.flush().await.unwrap();
    let mut out = Vec::new();
    let _ = tokio::time::timeout(StdDuration::from_secs(10), tls.read_to_end(&mut out)).await;
    let text = String::from_utf8_lossy(&out).into_owned();
    let status = text
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    (
        status,
        text.split_once("\r\n\r\n").map_or("", |x| x.1).to_owned(),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn f4_a_revocation_while_the_body_is_in_flight_stops_upload_and_control() {
    let h = Harness::start().await;
    let u = h.user("u1");
    let a = h.pc(&u, "PC-A").await;
    let b = h.pc(&u, "PC-B").await;
    a.synced().await;
    b.synced().await;
    enable(&a, Category::Hydration).await;
    b.synced().await;
    // Upload.
    let body = serde_json::to_string(&UploadRequest {
        rows: vec![hydration_row(&a, today(&h), 1)],
    })
    .unwrap();
    let (status, resp) = split_request(&h, &a, "/v1/upload", &body).await;
    assert_eq!(
        (status, resp.contains("device_revoked")),
        (401, true),
        "{resp}"
    );
    assert!(rows_of(&h, a.origin()).is_empty(), "no effect");
    // Control.
    let tail = h.tail_and_hwm(&u).0;
    let body = serde_json::to_string(&ControlRequest {
        ops: vec![ControlOp::Delete {
            request_id: new_id(),
            category: Category::Hydration,
            from: text(today(&h)),
            to: text(today(&h)),
        }],
    })
    .unwrap();
    let (status, resp) = split_request(&h, &b, "/v1/control", &body).await;
    assert_eq!(
        (status, resp.contains("device_revoked")),
        (401, true),
        "{resp}"
    );
    // Only the two revocations were recorded.
    assert_eq!(h.tail_and_hwm(&u).0, tail + 1);
    assert_eq!(entries_of_kind(&h, &u, 2), 0, "no deletion");
}

#[tokio::test(flavor = "multi_thread")]
async fn f4_a_revocation_during_hold_stops_a_receipt_in_flight_from_counting() {
    let h = Harness::start().await;
    let u = h.user("u1");
    let a = h.pc(&u, "PC-A").await;
    let b = h.pc(&u, "PC-B").await;
    a.synced().await;
    b.synced().await;
    let file = h.backup();
    let h = h.restore(&file).await;
    let receipt = b.with(|s| s.head_receipt().unwrap().unwrap().to_wire());
    let body = serde_json::json!({ "head": receipt }).to_string();
    let (status, resp) = split_request(&h, &b, "/v1/receipt", &body).await;
    assert_eq!(
        (status, resp.contains("device_revoked")),
        (401, true),
        "{resp}"
    );
    let evidence: i64 = h
        .server
        .db()
        .query_row(
            "SELECT COUNT(*) FROM evidence WHERE device_ref = ?1",
            [b.dev.device_ref.as_slice()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(evidence, 0, "no evidence for a revoked device");
}

// ---------------------------------------------------------------- finding 5

#[tokio::test(flavor = "multi_thread")]
async fn f5_an_unmirrored_enable_or_confirmation_authorises_no_upload() {
    let h = Harness::start().await;
    let u = h.user("u1");
    let a = h.pc(&u, "PC-A").await;
    a.synced().await;
    // The enable commits, then the mirror step fails (crash after commit).
    a.with(|s| {
        s.request_enable(Category::Hydration, "v1-5-sharing-1")
            .unwrap()
    });
    h.server.inject_faults(Faults {
        crash_after_commit: true,
        ..Faults::default()
    });
    assert!(a.sync().await.is_err());
    assert!(
        h.server.policy_of(&u, Category::Hydration).unwrap().enabled,
        "live, unmirrored"
    );
    let (tail, hwm) = h.tail_and_hwm(&u);
    assert_eq!(tail, hwm + 1);
    let out = upload(&h, &a, vec![hydration_row(&a, today(&h), 1)])
        .await
        .unwrap();
    assert_eq!(
        out,
        vec![RowOutcome::Rejected {
            error: WireError::CategoryOff
        }]
    );
    assert!(rows_of(&h, a.origin()).is_empty());
    // Once mirrored (any later request mirrors what is outstanding), it authorises.
    a.synced().await;
    let (tail, hwm) = h.tail_and_hwm(&u);
    assert_eq!(tail, hwm);
    let out = upload(&h, &a, vec![hydration_row(&a, today(&h), 2)])
        .await
        .unwrap();
    assert_eq!(out, vec![RowOutcome::Accepted]);

    // A confirmation whose entry the mirror does not hold authorises nothing either.
    let d = h.pending_pc(&u, "PC-D").await;
    h.server
        .set_mirror_fault(Some(hy_server::mirror::MirrorFault::Unavailable));
    h.confirm(&d.dev);
    let out = upload(&h, &d, vec![hydration_row(&d, today(&h), 1)])
        .await
        .unwrap();
    assert_eq!(
        out,
        vec![RowOutcome::Rejected {
            error: WireError::BeforeShareFrom
        }]
    );
    // Policy already on and mirrored still lets A upload during the outage.
    let out = upload(&h, &a, vec![hydration_row(&a, today(&h), 3)])
        .await
        .unwrap();
    assert_eq!(out, vec![RowOutcome::Accepted]);
    h.server.set_mirror_fault(None);
    d.synced().await;
    let out = upload(&h, &d, vec![hydration_row(&d, today(&h), 2)])
        .await
        .unwrap();
    assert_eq!(out, vec![RowOutcome::Accepted]);
}

// ---------------------------------------------------------------- finding 6

#[test]
fn f6_the_mirror_volume_check_fails_closed() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    // A new folder on the data folder's own drive: it does not exist yet, so it is
    // refused instead of created (and would be on the same volume anyway).
    let new_parent = tmp.path().join("new-mirror-folder").join("control.mirror");
    assert_eq!(
        check_volume(&data, &new_parent),
        Err(VolumeError::Unreachable)
    );
    assert!(
        !tmp.path().join("new-mirror-folder").exists(),
        "nothing created"
    );
    // An existing folder on the same volume.
    let same = tmp.path().join("same");
    std::fs::create_dir_all(&same).unwrap();
    assert_eq!(
        check_volume(&data, &same.join("control.mirror")),
        Err(VolumeError::SameVolume)
    );
    // Inside the data folder.
    assert_eq!(
        check_volume(&data, &data.join("control.mirror")),
        Err(VolumeError::InsideData)
    );
    // An unavailable share (its folder cannot be reached).
    let share = tmp
        .path()
        .join("offline-share")
        .join("hy")
        .join("control.mirror");
    assert_eq!(check_volume(&data, &share), Err(VolumeError::Unreachable));
    // A valid separate volume, and a volume that cannot be identified.
    let data_c = data.canonicalize().unwrap();
    let separate = |p: &std::path::Path| {
        Ok(VolumeId::for_tests(if p == data_c {
            "data"
        } else {
            "mirror"
        }))
    };
    assert_eq!(
        check_volume_with(&data, &same.join("control.mirror"), separate),
        Ok(())
    );
    let unknown = |p: &std::path::Path| {
        if p == data_c {
            Ok(VolumeId::for_tests("data"))
        } else {
            Err(std::io::Error::other("share went away"))
        }
    };
    assert_eq!(
        check_volume_with(&data, &same.join("control.mirror"), unknown),
        Err(VolumeError::Unverifiable)
    );
}

// ---------------------------------------------------------------- finding 7

#[tokio::test(flavor = "multi_thread")]
async fn f7_an_unreadable_mirror_pauses_the_server_with_its_data_intact() {
    let h = Harness::start().await;
    let u = h.user("u1");
    let a = h.pc(&u, "PC-A").await;
    a.synced().await;
    enable(&a, Category::Hydration).await;
    a.water(at(&h, 0), 400);
    a.synced().await;
    let before = (
        h.count("summaries"),
        h.count("control_log"),
        h.count("devices"),
    );
    let mirror = h.mirror.clone();
    let config = h.server.config.clone();
    let port = h.api.port();
    let (tmp, dir, clock) = h.stop().await;
    let snapshot = |dir: &std::path::Path| {
        let conn = hy_server::db::open(&dir.join("server.db")).unwrap();
        let n = |t: &str| -> i64 {
            conn.query_row(&format!("SELECT COUNT(*) FROM {t}"), [], |r| r.get(0))
                .unwrap()
        };
        let revoked: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM devices WHERE state = 'revoked'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        ((n("summaries"), n("control_log"), n("devices")), revoked)
    };
    // 1. The file cannot be read (a permission failure): here, a folder in its place.
    let aside = mirror.with_extension("aside");
    std::fs::rename(&mirror, &aside).unwrap();
    std::fs::create_dir(&mirror).unwrap();
    let err = Harness::open_full(
        &dir,
        &mirror,
        clock.clone(),
        config.clone(),
        common::test_limits(),
        Faults::default(),
        port,
    )
    .await
    .err()
    .expect("the server must not start");
    assert!(err.contains("cannot be read"), "{err}");
    assert_eq!(snapshot(&dir), (before, 0), "nothing deleted or revoked");
    std::fs::remove_dir(&mirror).unwrap();
    std::fs::rename(&aside, &mirror).unwrap();
    // 2. The share's folder is unreachable.
    let folder = mirror.parent().unwrap().to_path_buf();
    let away = folder.with_extension("offline");
    std::fs::rename(&folder, &away).unwrap();
    let err = Harness::open_full(
        &dir,
        &mirror,
        clock.clone(),
        config.clone(),
        common::test_limits(),
        Faults::default(),
        port,
    )
    .await
    .err()
    .expect("the server must not start");
    assert!(err.contains("cannot be read"), "{err}");
    assert_eq!(snapshot(&dir), (before, 0));
    std::fs::rename(&away, &folder).unwrap();
    // 3. Access returns: a normal start, everything as it was.
    let mut h = Harness::open_full(
        &dir,
        &mirror,
        clock,
        config,
        common::test_limits(),
        Faults::default(),
        port,
    )
    .await
    .unwrap();
    h.keep(tmp);
    assert_eq!(h.startup, hy_server::recovery::Startup::Opened);
    assert!(matches!(a.synced().await, SyncOutcome::Synced { .. }));
    assert_eq!(h.audits("global_discard"), 0);
}

// ------------------------------------------------- deviation 12 (re-send rules)

#[tokio::test(flavor = "multi_thread")]
#[allow(clippy::too_many_lines)]
async fn d12_the_re_send_after_release_keeps_consent_confirmation_deletion_and_provenance() {
    let h = Harness::start().await;
    let u = h.user("u1");
    let a = h.pc(&u, "PC-A").await;
    let b = h.pc(&u, "PC-B").await;
    a.synced().await;
    b.synced().await;
    enable(&a, Category::Hydration).await;
    enable(&a, Category::PhoneReports).await;
    b.synced().await;
    let d0 = today(&h);
    a.water(at(&h, 0), 500);
    phone(&a, at(&h, 0));
    b.water(at(&h, 0), 300);
    a.synced().await;
    b.synced().await;
    // A also holds B's row as imported history (provenance: never re-sent as A's).
    hy_core::link::flow::restore_from_server(&a.store, &*a.dev.vault, opts(), d0, d0)
        .await
        .unwrap();
    let phone_row = |h: &Harness, d: &str| -> Option<(i64, i64)> {
        h.server
            .db()
            .query_row(
                "SELECT revision, received_at FROM summaries
                 WHERE category = 'phone_reports' AND origin_device = ?1 AND local_date = ?2",
                rusqlite::params![a.origin().as_bytes().as_slice(), d],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok()
    };
    let sent_before = phone_row(&h, &text(d0)).expect("day 0 phone row sent with consent");
    let file = h.backup();

    // Day 1: C is confirmed (it holds water from day 0 locally); A deletes day 0 and
    // switches phone reports off before its day-1 report is ever sent.
    h.clock.advance(Duration::days(1));
    let d1 = today(&h);
    let c = h.pc(&u, "PC-C").await;
    c.water(at(&h, -1), 700);
    c.water(at(&h, 0), 200);
    a.water(at(&h, 0), 600);
    phone(&a, at(&h, 0));
    b.water(at(&h, 0), 100);
    a.with(|s| {
        s.record_deletion(Category::Hydration, DateRange { from: d0, to: d0 })
            .unwrap();
        s.disable_sharing(Category::PhoneReports).unwrap();
    });
    for pc in [&a, &b, &c, &a, &b, &c] {
        pc.synced().await;
    }

    let h = h.restore(&file).await;
    h.server.witness_users(true).unwrap();
    h.server.witness_user(&u, true).unwrap();
    for pc in [&a, &b, &c] {
        assert!(matches!(
            pc.synced().await,
            SyncOutcome::Hold { evidence: true, .. }
        ));
    }
    for pc in [&a, &b, &c, &a, &b, &c] {
        assert!(matches!(pc.synced().await, SyncOutcome::Synced { .. }));
    }
    let (d0, d1) = (text(d0), text(d1));
    // Re-sent: each PC's own current days. A's day-0 phone row is the one restored
    // from the backup (sent while consent was on); stopping sharing does not delete
    // history, but it is never sent again.
    assert_eq!(
        rows_of(&h, a.origin()),
        vec![format!("phone_reports:{d0}"), format!("hydration:{d1}")]
    );
    assert_eq!(
        phone_row(&h, &d0),
        Some(sent_before),
        "not re-sent after the disable"
    );
    assert_eq!(rows_of(&h, b.origin()), vec![format!("hydration:{d1}")]);
    assert_eq!(rows_of(&h, c.origin()), vec![format!("hydration:{d1}")]);
    // Deletion: hydration day 0 is gone for every origin and never re-sent.
    // Consent: phone reports were switched off before day 1's was sent: never sent.
    assert!(!rows_of(&h, a.origin()).contains(&format!("phone_reports:{d1}")));
    // Confirmation date: C's day-0 water is never sent.
    assert!(!rows_of(&h, c.origin()).contains(&format!("hydration:{d0}")));
    // Provenance: nothing A sent carries another origin.
    let foreign: i64 = h
        .server
        .db()
        .query_row(
            "SELECT COUNT(*) FROM summaries WHERE device_ref = ?1 AND origin_device <> ?2",
            rusqlite::params![
                a.dev.device_ref.as_slice(),
                a.origin().as_bytes().as_slice()
            ],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(foreign, 0);
}
