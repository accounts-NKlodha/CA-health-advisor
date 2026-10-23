//! Sync between companions and the real server (slice V1-5b): consent, policy, remote
//! enable (owner decision O2), deletion and uploads. T-S6, T-S9, T-S17, T-S18 (history
//! part), T-D1 to T-D4, T-D7, T-L5 and T-L6, all over loopback TLS with the companion's
//! real sync flow.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use chrono::{DateTime, Duration, NaiveDate, Utc};
use common::{Harness, Pc};
use hy_core::clock::Clock;
use hy_core::link::flow::SyncOutcome;
use hy_core::protocol::wire::{
    ControlOp, ControlOutcome, ControlRequest, RowOutcome, UploadRequest, UploadRow, WireError,
};
use hy_core::schema::{Category, ConsentKind, DateRange, PolicyOpState};
use hy_core::store::{NoticeKind, REMOTE_ENABLE_NOTICE};
use uuid::Uuid;

/// 10:30 IST on the day `offset` days from the start.
fn day_at(h: &Harness, offset: i64) -> DateTime<Utc> {
    let start = h.clock.now();
    let d = start.date_naive() + Duration::days(offset);
    d.and_hms_opt(5, 0, 0).unwrap().and_utc()
}

fn date(h: &Harness, offset: i64) -> NaiveDate {
    h.clock.now().date_naive() + Duration::days(offset)
}

fn text(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

fn summaries(h: &Harness, origin: Uuid) -> Vec<String> {
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

fn hydration(ml: u32) -> serde_json::Value {
    serde_json::json!({ "v": 1, "ml": ml, "goal_ml": null, "entries": 2, "coverage": null })
}

fn row(pc: &Pc, d: NaiveDate, revision: u32, payload: serde_json::Value) -> UploadRow {
    UploadRow {
        category: Category::Hydration,
        origin_device: pc.origin(),
        local_date: text(d),
        revision,
        payload,
    }
}

async fn upload(h: &Harness, pc: &Pc, rows: Vec<UploadRow>) -> Vec<RowOutcome> {
    pc.client(h)
        .upload(&pc.dev.token, &UploadRequest { rows })
        .await
        .unwrap()
        .results
}

async fn enable(pc: &Pc, c: Category) {
    pc.with(|s| s.request_enable(c, "v1-5-sharing-1").unwrap());
    let out = pc.synced().await;
    assert!(
        matches!(
            out,
            SyncOutcome::Synced {
                control_sent: 1,
                ..
            }
        ),
        "{out:?}"
    );
    assert!(pc.with(|s| s.sharing_policy(c).unwrap().enabled));
}

/// A user with two confirmed and synced PCs.
async fn two_pcs(h: &Harness) -> (Pc, Pc) {
    let u = h.user("u1");
    let a = h.pc(&u, "PC-A").await;
    a.synced().await;
    let b = h.pc(&u, "PC-B").await;
    b.synced().await;
    a.synced().await;
    (a, b)
}

#[tokio::test(flavor = "multi_thread")]
async fn t_s17_a_remote_enable_applies_after_a_verified_extension_with_a_notice() {
    let h = Harness::start().await;
    let (a, b) = two_pcs(&h).await;
    b.water(day_at(&h, -1), 300);
    b.water(day_at(&h, 0), 250);
    b.water(day_at(&h, 0), 250);

    enable(&a, Category::Hydration).await;
    let pa = a.with(|s| s.sharing_policy(Category::Hydration).unwrap());
    assert_eq!(pa.share_from, Some(date(&h, 0)));
    assert!(!b.with(|s| s.sharing_policy(Category::Hydration).unwrap().enabled));

    // B applies A's enable at its next verified pull (O2): no blocking question, a
    // notice and a consent entry, and only rows from share_from on are sent.
    let out = b.synced().await;
    assert!(
        matches!(
            out,
            SyncOutcome::Synced {
                uploaded: 1,
                rejected: 0,
                ..
            }
        ),
        "{out:?}"
    );
    let pb = b.with(|s| s.sharing_policy(Category::Hydration).unwrap());
    assert!(pb.enabled);
    assert_eq!(pb.share_from, Some(date(&h, 0)));
    let notices = b.with(|s| s.server_notices().unwrap());
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].kind, NoticeKind::RemoteEnable);
    assert_eq!(notices[0].category, Some(Category::Hydration));
    assert_eq!(notices[0].from_label.as_deref(), Some("PC-A"));
    let log = b.with(|s| s.consent_log().unwrap());
    let applied = log
        .iter()
        .find(|e| e.kind == ConsentKind::RemoteEnableApplied)
        .expect("remote_enable_applied recorded");
    assert_eq!(
        applied.notice_version.as_deref(),
        Some(REMOTE_ENABLE_NOTICE)
    );
    assert_eq!(
        summaries(&h, b.origin()),
        vec![format!("hydration:{}", text(date(&h, 0)))]
    );
    let stored: String = h
        .server
        .db()
        .query_row(
            "SELECT payload FROM summaries WHERE origin_device = ?1",
            [b.origin().as_bytes().as_slice()],
            |r| r.get(0),
        )
        .unwrap();
    let stored: serde_json::Value = serde_json::from_str(&stored).unwrap();
    assert_eq!(stored["ml"], 500);
    assert_eq!(stored["entries"], 2);
    // Only the schema's fields reach the server.
    let keys: Vec<&String> = stored.as_object().unwrap().keys().collect();
    assert!(
        keys.iter()
            .all(|k| ["v", "ml", "goal_ml", "entries", "coverage"].contains(&k.as_str())),
        "{keys:?}"
    );

    // C is paired and confirmed ten days later, holding rows from before that.
    h.clock.advance(Duration::days(10));
    let c = h.pc(&a.dev.user_id, "PC-C").await;
    for d in [-3, -1, 0] {
        c.water(day_at(&h, d), 400);
    }
    let out = c.synced().await;
    assert!(
        matches!(out, SyncOutcome::Synced { uploaded: 1, .. }),
        "{out:?}"
    );
    assert!(c.with(|s| s.sharing_policy(Category::Hydration).unwrap().enabled));
    assert_eq!(
        summaries(&h, c.origin()),
        vec![format!("hydration:{}", text(date(&h, 0)))]
    );
    // The server refuses an earlier row from C even though it is after share_from.
    let forced = upload(&h, &c, vec![row(&c, date(&h, -3), 1, hydration(400))]).await;
    assert_eq!(
        forced,
        vec![RowOutcome::Rejected {
            error: WireError::BeforeShareFrom
        }]
    );
    assert_eq!(summaries(&h, c.origin()).len(), 1);

    // Switch off on B: A applies the disable at its next pull, with a notice.
    b.with(|s| s.disable_sharing(Category::Hydration).unwrap());
    b.synced().await;
    a.synced().await;
    assert!(!a.with(|s| s.sharing_policy(Category::Hydration).unwrap().enabled));
    let notices = a.with(|s| s.server_notices().unwrap());
    assert!(notices.iter().any(|n| n.kind == NoticeKind::RemoteDisable
        && n.from_label.as_deref() == Some("PC-B")));
    // After the disable nothing more of that category is accepted.
    let late = upload(&h, &c, vec![row(&c, date(&h, 0), 2, hydration(900))]).await;
    assert_eq!(
        late,
        vec![RowOutcome::Rejected {
            error: WireError::CategoryOff
        }]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn t_s6_replayed_control_operations_and_uploads_keep_their_original_results() {
    let h = Harness::start().await;
    let (a, _b) = two_pcs(&h).await;
    let enable = ControlOp::Enable {
        op_id: common::new_id(),
        category: Category::Hydration,
        base_seq: a.with(|s| s.sharing_policy(Category::Hydration).unwrap().seq),
    };
    let client = a.client(&h);
    let first = client
        .control(
            &a.dev.token,
            &ControlRequest {
                ops: vec![enable.clone()],
            },
        )
        .await
        .unwrap();
    let ControlOutcome::Accepted { seq } = first.results[0].outcome else {
        panic!("{first:?}")
    };
    let disable = ControlOp::Disable {
        op_id: common::new_id(),
        category: Category::Hydration,
    };
    client
        .control(&a.dev.token, &ControlRequest { ops: vec![disable] })
        .await
        .unwrap();
    let entries_before = h.count("control_log");
    // The captured enable, sent again after the disable: its original result, applied
    // nowhere.
    let again = client
        .control(&a.dev.token, &ControlRequest { ops: vec![enable] })
        .await
        .unwrap();
    assert_eq!(again.results[0].outcome, ControlOutcome::Accepted { seq });
    assert_eq!(h.count("control_log"), entries_before);
    assert!(
        !h.server
            .policy_of(&a.dev.user_id, Category::Hydration)
            .unwrap()
            .enabled
    );

    // The same upload twice: one row, the second refused.
    a.synced().await;
    enable_on(&h, &a).await;
    let r = row(&a, date(&h, 0), 1, hydration(500));
    assert_eq!(
        upload(&h, &a, vec![r.clone()]).await,
        vec![RowOutcome::Accepted]
    );
    assert_eq!(
        upload(&h, &a, vec![r]).await,
        vec![RowOutcome::Rejected {
            error: WireError::RevisionNotHigher
        }]
    );
    assert_eq!(summaries(&h, a.origin()).len(), 1);
}

async fn enable_on(_h: &Harness, pc: &Pc) {
    enable(pc, Category::Hydration).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn t_s9_every_invalid_upload_is_rejected_with_its_reason() {
    let h = Harness::start().await;
    let (a, b) = two_pcs(&h).await;
    enable(&a, Category::Hydration).await;
    let today = date(&h, 0);
    // A deletion of yesterday..today for hydration on another day range to tombstone.
    a.with(|s| {
        s.record_deletion(
            Category::Hydration,
            DateRange {
                from: today + Duration::days(1),
                to: today + Duration::days(1),
            },
        )
        .unwrap()
    });
    a.synced().await;
    let before = h.count("summaries");
    let mut unknown = hydration(500);
    unknown["note"] = serde_json::json!("x");
    let mut free_text = hydration(500);
    free_text["ml"] = serde_json::json!("five hundred");
    let mut huge = hydration(500);
    huge["coverage"] = serde_json::json!("x".repeat(2000));
    let mut other_origin = row(&a, today, 1, hydration(500));
    other_origin.origin_device = b.origin();
    let cases = vec![
        (row(&a, today, 1, unknown), WireError::SchemaInvalid),
        (row(&a, today, 1, free_text), WireError::SchemaInvalid),
        (row(&a, today, 1, huge), WireError::SchemaInvalid),
        (
            row(
                &a,
                today,
                1,
                serde_json::json!({ "v": 1, "ml": -5, "goal_ml": null, "entries": 1, "coverage": null }),
            ),
            WireError::SchemaInvalid,
        ),
        (
            row(&a, today + Duration::days(2), 1, hydration(500)),
            WireError::DateOutOfRange,
        ),
        (
            row(&a, today - Duration::days(1), 1, hydration(500)),
            WireError::BeforeShareFrom,
        ),
        (other_origin, WireError::OriginMismatch),
        (
            row(&a, today + Duration::days(1), 1, hydration(500)),
            WireError::Tombstoned,
        ),
        (
            UploadRow {
                category: Category::PhoneReports,
                ..row(
                    &a,
                    today,
                    1,
                    serde_json::json!({ "v": 1, "work": 1, "personal": 0, "unsure": 0 }),
                )
            },
            WireError::CategoryOff,
        ),
    ];
    for (r, why) in cases {
        let got = upload(&h, &a, vec![r.clone()]).await;
        assert_eq!(got, vec![RowOutcome::Rejected { error: why }], "{r:?}");
    }
    assert_eq!(h.count("summaries"), before);

    // A date older than 90 days, once share_from is that old.
    h.clock.advance(Duration::days(95));
    let old = upload(
        &h,
        &a,
        vec![row(&a, today + Duration::days(2), 1, hydration(1))],
    )
    .await;
    assert_eq!(
        old,
        vec![RowOutcome::Rejected {
            error: WireError::DateOutOfRange
        }]
    );
    assert_eq!(h.count("summaries"), before);

    // An unconfirmed device: refused before the body is read.
    let p = h.pending_pc(&a.dev.user_id, "PC-P").await;
    let err = p
        .client(&h)
        .upload(
            &p.dev.token,
            &UploadRequest {
                rows: vec![row(&p, today, 1, hydration(1))],
            },
        )
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            hy_core::link::LinkError::Server {
                error: WireError::DeviceUnconfirmed,
                ..
            }
        ),
        "{err:?}"
    );
    assert_eq!(h.count("summaries"), before);
}

#[tokio::test(flavor = "multi_thread")]
async fn t_d1_a_delayed_upload_after_a_deletion_is_discarded() {
    let h = Harness::start().await;
    let (a, b) = two_pcs(&h).await;
    enable(&a, Category::Hydration).await;
    b.synced().await;
    // B goes offline with a queued record for today.
    b.water(day_at(&h, 0), 600);
    let today = h.clock.now().date_naive();
    assert_eq!(b.with(|s| s.queue_share_records(today).unwrap()), 1);
    assert_eq!(b.with(|s| s.outbox().unwrap().len()), 1);
    let queued = b.with(|s| s.outbox().unwrap()[0].clone());
    // The user deletes today on A.
    a.with(|s| {
        s.record_deletion(
            Category::Hydration,
            DateRange {
                from: today,
                to: today,
            },
        )
        .unwrap()
    });
    a.synced().await;
    // B reconnects: the deletion reaches it through the chain and its queue is purged;
    // a forced send of the old record is rejected; the server holds none.
    let out = b.synced().await;
    assert!(
        matches!(out, SyncOutcome::Synced { uploaded: 0, .. }),
        "{out:?}"
    );
    assert!(b.with(|s| s.outbox().unwrap().is_empty()));
    let forced = upload(
        &h,
        &b,
        vec![UploadRow {
            category: queued.category,
            origin_device: queued.origin_device,
            local_date: text(queued.local_date),
            revision: queued.revision,
            payload: queued.payload,
        }],
    )
    .await;
    assert_eq!(
        forced,
        vec![RowOutcome::Rejected {
            error: WireError::Tombstoned
        }]
    );
    assert!(summaries(&h, b.origin()).is_empty());
    // B holds the tombstone as acknowledged.
    let t = b.with(|s| s.tombstones(Category::Hydration).unwrap());
    assert_eq!(t.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn t_d2_an_offline_withdrawal_purges_the_queue_at_once() {
    let h = Harness::start().await;
    let (a, b) = two_pcs(&h).await;
    enable(&a, Category::Hydration).await;
    b.synced().await;
    b.water(day_at(&h, 0), 600);
    let today = h.clock.now().date_naive();
    b.with(|s| s.queue_share_records(today).unwrap());
    // Offline: switching off purges the queue in the same transaction.
    b.with(|s| s.disable_sharing(Category::Hydration).unwrap());
    assert!(b.with(|s| s.outbox().unwrap().is_empty()));
    let out = b.synced().await;
    assert!(
        matches!(
            out,
            SyncOutcome::Synced {
                uploaded: 0,
                control_sent: 1,
                ..
            }
        ),
        "{out:?}"
    );
    assert!(summaries(&h, b.origin()).is_empty());
    assert!(
        !h.server
            .policy_of(&b.dev.user_id, Category::Hydration)
            .unwrap()
            .enabled
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn t_d3_an_enable_older_than_a_disable_elsewhere_is_rejected_and_asked_again() {
    let h = Harness::start().await;
    let (a, b) = two_pcs(&h).await;
    enable(&a, Category::Hydration).await;
    b.synced().await;
    // B's user switches it off and on again while offline (seq from before A's disable).
    a.with(|s| s.disable_sharing(Category::Hydration).unwrap());
    let stale = b.with(|s| {
        s.disable_sharing(Category::Hydration).unwrap();
        s.request_enable(Category::Hydration, "v1-5-sharing-1")
            .unwrap()
    });
    a.synced().await;
    b.synced().await;
    // B's disable was accepted, its enable was older than A's disable: rejected.
    assert_eq!(
        b.with(|s| s.policy_op(stale).unwrap().unwrap().state),
        PolicyOpState::RejectedStale
    );
    assert!(!b.with(|s| s.sharing_policy(Category::Hydration).unwrap().enabled));
    assert!(
        b.with(|s| s.server_notices().unwrap())
            .iter()
            .any(|n| n.kind == NoticeKind::StaleEnable)
    );
    assert!(
        !h.server
            .policy_of(&b.dev.user_id, Category::Hydration)
            .unwrap()
            .enabled
    );
    // Asked again, it now succeeds.
    enable(&b, Category::Hydration).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn t_d4_t_d7_after_a_deletion_only_new_dates_are_sent_and_a_deleted_date_stays_deleted() {
    let h = Harness::start().await;
    let (a, _b) = two_pcs(&h).await;
    enable(&a, Category::Hydration).await;
    a.water(day_at(&h, 0), 500);
    a.synced().await;
    assert_eq!(summaries(&h, a.origin()).len(), 1);
    let today = date(&h, 0);
    // Delete today, switch off.
    a.with(|s| {
        s.record_deletion(
            Category::Hydration,
            DateRange {
                from: today,
                to: today,
            },
        )
        .unwrap();
        s.disable_sharing(Category::Hydration).unwrap();
    });
    a.synced().await;
    assert!(summaries(&h, a.origin()).is_empty());
    // Two days later the user switches it on again and logs more.
    h.clock.advance(Duration::days(2));
    a.water(day_at(&h, 0), 700);
    a.water(day_at(&h, -1), 700);
    enable(&a, Category::Hydration).await;
    assert_eq!(
        summaries(&h, a.origin()),
        vec![format!("hydration:{}", text(date(&h, 0)))],
        "only dates on or after the new share_from"
    );
    // T-D7: rebuild the deleted date from local events and retry: no record is built,
    // a forced upload is refused.
    a.water(today.and_hms_opt(4, 0, 0).unwrap().and_utc(), 100);
    assert!(matches!(
        a.with(|s| s.enqueue_upload(Category::Hydration, today, 9, &hydration(800))),
        Err(hy_core::Error::UploadBlocked(_))
    ));
    let forced = upload(&h, &a, vec![row(&a, today, 9, hydration(800))]).await;
    assert!(
        matches!(forced[0], RowOutcome::Rejected { .. }),
        "{forced:?}"
    );
    assert!(!summaries(&h, a.origin()).contains(&format!("hydration:{}", text(today))));
}

#[tokio::test(flavor = "multi_thread")]
async fn t_s18_t_l5_t_l6_restoring_from_the_server_is_keyed_by_origin_and_logged() {
    let h = Harness::start().await;
    let (a, b) = two_pcs(&h).await;
    enable(&a, Category::Hydration).await;
    b.synced().await;
    a.water(day_at(&h, 0), 500);
    b.water(day_at(&h, 0), 300);
    a.synced().await;
    b.synced().await;
    let entries_before = h.count("control_log");
    let today = date(&h, 0);
    // A restores twice: B's row once (its own is skipped), never doubled.
    for _ in 0..2 {
        let n = hy_core::link::flow::restore_from_server(
            &a.store,
            &*a.dev.vault,
            common::opts(),
            today - Duration::days(89),
            today,
        )
        .await
        .unwrap();
        assert!(n <= 1);
    }
    let other = b.origin();
    let imported = a.with(|s| {
        s.imported_summary(Category::Hydration, other, today)
            .unwrap()
    });
    let (s, _) = imported.expect("B's row imported");
    assert_eq!(s.payload["ml"], 300);
    let own = a.origin();
    assert!(
        a.with(|s| s.imported_summary(Category::Hydration, own, today).unwrap())
            .is_none()
    );
    // Each read is a history_access entry in the user's chain, seen by B.
    assert_eq!(h.count("control_log"), entries_before + 2);
    b.synced().await;
    // More than 90 days is refused.
    let err = a
        .client(&h)
        .history(
            &a.dev.token,
            &text(today - Duration::days(90)),
            &text(today),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        hy_core::link::LinkError::Server {
            error: WireError::SchemaInvalid,
            ..
        }
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn t_l7_rows_imported_from_another_pcs_backup_are_never_sent_under_this_pcs_id() {
    let h = Harness::start().await;
    let (a, _b) = two_pcs(&h).await;
    enable(&a, Category::Hydration).await;
    a.water(day_at(&h, 0), 500);
    a.synced().await;
    let server_rows = h.count("summaries");
    // A's data is imported on a new PC N that was paired: the import unpairs it, and
    // it pairs again and syncs.
    let contents = a.with(|s| s.backup_contents().unwrap());
    let n = h.pc(&a.dev.user_id, "PC-N").await;
    n.synced().await;
    n.with(|s| s.import_contents(&contents).unwrap());
    assert!(n.with(|s| s.server_link().unwrap().is_none()));
    let (_, again) = h
        .pair_as(&h.slip(&a.dev.user_id), "PC-N", n.origin(), common::opts())
        .await
        .unwrap();
    n.with(|s| s.save_pairing(&again.link, "v1-5-notice-1").unwrap());
    h.confirm(&again);
    let n2 = Pc {
        dev: again,
        store: n.store,
    };
    // It switches hydration on itself (the chain's enable was before its new pairing).
    n2.synced().await;
    if !n2.with(|s| s.sharing_policy(Category::Hydration).unwrap().enabled) {
        enable(&n2, Category::Hydration).await;
    }
    let out = n2.synced().await;
    assert!(
        matches!(out, SyncOutcome::Synced { uploaded: 0, .. }),
        "{out:?}"
    );
    assert_eq!(h.count("summaries"), server_rows, "server totals unchanged");
    assert!(summaries(&h, n2.origin()).is_empty());
}
