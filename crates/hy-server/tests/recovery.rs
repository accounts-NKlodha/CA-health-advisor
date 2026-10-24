//! Server restore, recovery hold, receipts and the control-log mirror (slice V1-5b;
//! contract 7.3, 7.4 and 9; architecture 5.3). T-D5, T-D6, T-D8 to T-D13, T-S13 to
//! T-S16, T-S20, T-S21 and T-S26, against the real server with real backups, restores
//! and restarts, and the companion's real sync flow.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use chrono::{Duration, NaiveDate};
use common::{Harness, Pc, new_id, opts, tls_request};
use hy_core::clock::Clock;
use hy_core::link::LinkError;
use hy_core::link::flow::{FlowError, SyncOutcome};
use hy_core::protocol::b64;
use hy_core::protocol::wire::{
    ControlOp, ControlOutcome, ControlRequest, RowOutcome, StateResponse, UploadRequest, UploadRow,
    WireError,
};
use hy_core::schema::{Category, DateRange, PolicyOpState};
use hy_core::store::NoticeKind;
use hy_server::recovery::Startup;
use hy_server::server::Faults;

fn today(h: &Harness) -> NaiveDate {
    h.clock.now().date_naive()
}

fn morning(h: &Harness) -> chrono::DateTime<chrono::Utc> {
    today(h).and_hms_opt(4, 0, 0).unwrap().and_utc()
}

async fn enable(pc: &Pc, c: Category) {
    pc.with(|s| s.request_enable(c, "v1-5-sharing-1").unwrap());
    pc.synced().await;
    assert!(pc.with(|s| s.sharing_policy(c).unwrap().enabled));
}

fn receipt_of(pc: &Pc) -> String {
    pc.with(|s| s.head_receipt().unwrap().unwrap().to_wire())
}

/// `POST /v1/receipt` with a raw Authorization value (or none).
async fn post_receipt(h: &Harness, auth: Option<&str>, body: &str) -> (u16, String) {
    let auth = auth.map_or(String::new(), |a| format!("Authorization: Bearer {a}\r\n"));
    let req = format!(
        "POST /v1/receipt HTTP/1.1\r\nHost: x\r\nConnection: close\r\n{auth}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    tls_request(h, 1, &req).await
}

fn token_of(pc: &Pc) -> String {
    b64(pc.dev.token.expose())
}

fn evidence(h: &Harness, user: &[u8; 16], device: &[u8; 16]) -> Option<Option<u64>> {
    h.server
        .hold_rows()
        .unwrap()
        .into_iter()
        .find(|r| &r.user_id == user)
        .map(|r| r.devices.iter().find(|d| &d.0 == device).and_then(|d| d.3))
}

fn in_hold(h: &Harness, user: &[u8; 16]) -> bool {
    h.server
        .hold_rows()
        .unwrap()
        .iter()
        .any(|r| &r.user_id == user)
}

fn summaries(h: &Harness, user: &[u8; 16]) -> i64 {
    h.server
        .db()
        .query_row(
            "SELECT COUNT(*) FROM summaries WHERE user_id = ?1",
            [user.as_slice()],
            |r| r.get(0),
        )
        .unwrap()
}

fn hold_started(h: &Harness, user: &[u8; 16]) -> Option<i64> {
    h.server
        .db()
        .query_row(
            "SELECT hold_started_at FROM users WHERE user_id = ?1",
            [user.as_slice()],
            |r| r.get(0),
        )
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
#[allow(clippy::too_many_lines)]
async fn t_d5_t_s15_t_s16_t_s26_a_restore_holds_the_user_until_every_device_checks_in() {
    let h = Harness::start().await;
    let u = h.user("u1");
    let a = h.pc(&u, "PC-A").await;
    let e = h.pc(&u, "PC-E").await;
    let r = h.pc(&u, "PC-R").await;
    let x = h.pc(&u, "PC-X").await;
    for pc in [&a, &e, &r, &x] {
        pc.synced().await;
    }
    enable(&a, Category::Hydration).await;
    a.water(morning(&h), 500);
    a.synced().await;
    assert_eq!(summaries(&h, &u), 1);
    let slip_s = h.slip(&u);
    let file = h.backup();

    // After the snapshot: D pairs with the outstanding slip; P pairs and stays pending;
    // A deletes today and switches hydration off; R is revoked.
    h.clock.advance(Duration::seconds(61)); // the per-minute pairing limit
    let d = h.pending_pc_with(&slip_s, "PC-D").await;
    h.confirm(&d.dev);
    let p = h.pending_pc(&u, "PC-P").await;
    let t = today(&h);
    a.with(|s| {
        s.record_deletion(Category::Hydration, DateRange { from: t, to: t })
            .unwrap();
        s.disable_sharing(Category::Hydration).unwrap();
    });
    a.synced().await;
    h.server.revoke_device(&r.dev.device_ref).unwrap();
    for pc in [&a, &e, &d] {
        pc.synced().await;
    }
    assert_eq!(summaries(&h, &u), 0);

    let h = h.restore(&file).await;
    let Startup::Hold { replayed } = h.startup else {
        panic!("{:?}", h.startup)
    };
    assert!(replayed >= 6, "{replayed}");
    let started = hold_started(&h, &u).expect("in hold");
    // The deleted row is absent, the category is off, R is still revoked.
    assert_eq!(summaries(&h, &u), 0);
    assert!(!h.server.policy_of(&u, Category::Hydration).unwrap().enabled);
    assert_eq!(r.sync().await.unwrap_err(), FlowError::Revoked);
    // T-S15: the restore voided the slip, even though the snapshot holds it unused.
    h.clock.advance(Duration::seconds(61));
    let again = h.pair(&slip_s, "PC-thief").await.err().unwrap();
    assert_eq!(again, FlowError::Server(WireError::SlipInvalid));
    // Nothing is served during hold: state has no head and no entries.
    let raw = a
        .client(&h)
        .raw(
            reqwest::Method::GET,
            "/v1/state?after_seq=0",
            Some(&a.dev.token),
            None,
        )
        .await
        .unwrap();
    let StateResponse::Confirmed {
        hold,
        head,
        entries,
        ..
    } = common::state_of(&raw)
    else {
        panic!()
    };
    assert!(hold && head.is_none() && entries.is_empty());
    // T-D10: restore reads, uploads and enables are refused...
    let err = a
        .client(&h)
        .history(&a.dev.token, &t.to_string(), &t.to_string())
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            LinkError::Server {
                error: WireError::RecoveryHold,
                ..
            }
        ),
        "{err:?}"
    );
    let up = a
        .client(&h)
        .upload(
            &a.dev.token,
            &UploadRequest {
                rows: vec![UploadRow {
                    category: Category::Hydration,
                    origin_device: a.origin(),
                    local_date: t.to_string(),
                    revision: 5,
                    payload: serde_json::json!({ "v": 1, "ml": 1, "goal_ml": null, "entries": 1, "coverage": null }),
                }],
            },
        )
        .await
        .unwrap();
    assert_eq!(
        up.results,
        vec![RowOutcome::Rejected {
            error: WireError::RecoveryHold
        }]
    );
    let en = ControlOp::Enable {
        op_id: new_id(),
        category: Category::PhoneReports,
        base_seq: 99,
    };
    let resp = a
        .client(&h)
        .control(&a.dev.token, &ControlRequest { ops: vec![en] })
        .await
        .unwrap();
    assert_eq!(
        resp.results[0].outcome,
        ControlOutcome::Rejected {
            error: WireError::RecoveryHold
        }
    );
    assert!(resp.head.is_none(), "no new head during hold (T-D11)");

    // T-S26: D's genuine receipt presented without D's token never counts as D's.
    let body = serde_json::json!({ "head": receipt_of(&d) }).to_string();
    assert_eq!(post_receipt(&h, None, &body).await.0, 401); // (a)
    let (s, b) = post_receipt(&h, Some("AAAA"), &body).await; // (b)
    assert_eq!((s, b.contains("unknown_token")), (401, true));
    let (s, b) = post_receipt(&h, Some(&b64(&[9u8; 32])), &body).await; // (c)
    assert_eq!((s, b.contains("unknown_token")), (401, true));
    let (s, _) = post_receipt(&h, Some(&token_of(&e)), &body).await; // (d): E's, at most
    assert_eq!(s, 200);
    let with_ref = serde_json::json!({
        "head": receipt_of(&d),
        "device_ref": hy_core::protocol::hex(&d.dev.device_ref),
    })
    .to_string();
    assert_eq!(
        post_receipt(&h, Some(&token_of(&e)), &with_ref).await.0,
        400
    );
    let (s, b) = post_receipt(&h, Some(&token_of(&r)), &body).await; // (e)
    assert_eq!((s, b.contains("device_revoked")), (401, true));
    let (s, b) = post_receipt(&h, Some(&token_of(&p)), &body).await; // (f)
    assert!(b.contains("device_unconfirmed"), "{s} {b}");
    assert_eq!(
        evidence(&h, &u, &d.dev.device_ref),
        Some(None),
        "D's evidence is missing"
    );
    assert!(evidence(&h, &u, &e.dev.device_ref).unwrap().is_some());
    assert!(h.audits("receipt_refused") >= 3);
    // The administrator witnesses everything; release still waits for D, A and X.
    h.server.witness_users(true).unwrap();
    h.server.witness_user(&u, true).unwrap();
    assert!(in_hold(&h, &u));
    assert!(!h.server.try_release(&u).unwrap());
    assert_eq!(h.audits("user_discarded"), 0);
    // (g) D with its own token (T-S16: D paired after the snapshot authenticates).
    assert!(matches!(
        d.synced().await,
        SyncOutcome::Hold { evidence: true, .. }
    ));
    assert!(evidence(&h, &u, &d.dev.device_ref).unwrap().is_some());

    // During hold E switches phone reports off and deletes a day: accepted, no seq.
    let del_from = t - Duration::days(3);
    let held = e.with(|s| {
        let id = s.disable_sharing(Category::PhoneReports).unwrap();
        s.record_deletion(
            Category::Outcomes,
            DateRange {
                from: del_from,
                to: del_from,
            },
        )
        .unwrap();
        id
    });
    let out = e.synced().await;
    assert!(
        matches!(
            out,
            SyncOutcome::Hold {
                control_sent: 2,
                ..
            }
        ),
        "{out:?}"
    );
    let op = e.with(|s| s.policy_op(held).unwrap().unwrap());
    assert_eq!(op.state, PolicyOpState::PendingRecovery);
    assert!(e.with(|s| s.pending_control_ops().unwrap()).is_empty());
    // Revoking the absent X removes it from the evidence set (a revocation entry).
    let (tail_before, _) = h.tail_and_hwm(&u);
    h.server.revoke_device(&x.dev.device_ref).unwrap();
    assert_eq!(h.tail_and_hwm(&u).0, tail_before + 1);
    assert!(in_hold(&h, &u), "A is still missing");
    assert_eq!(
        hold_started(&h, &u),
        Some(started),
        "the maximum never moves"
    );

    assert!(matches!(
        a.synced().await,
        SyncOutcome::Hold { evidence: true, .. }
    ));
    // The pending P counts as paired (contract 9, "Pending devices"): it can never
    // present a receipt and cannot be confirmed during hold, so it blocks release...
    assert!(in_hold(&h, &u), "P is still listed");
    assert_eq!(evidence(&h, &u, &p.dev.device_ref), Some(None));
    let code = hy_core::protocol::words::device_check_code(&p.dev.device_ref);
    assert_eq!(
        h.server.confirm_device(&p.dev.device_ref, &code),
        Err(hy_server::server::AdminError::Hold)
    );
    assert_eq!(h.audits("user_released"), 0);
    // ...until the administrator rejects it: the last piece of evidence is then in, and
    // U is released at that moment, without waiting for the maximum.
    h.server.revoke_device(&p.dev.device_ref).unwrap();
    assert!(!in_hold(&h, &u));
    assert_eq!(h.audits("user_released"), 1);
    // The held disable and deletion were sequenced; every PC syncs normally again.
    for pc in [&a, &e, &d] {
        assert!(matches!(pc.synced().await, SyncOutcome::Synced { .. }));
    }
    let op = e.with(|s| s.policy_op(held).unwrap().unwrap());
    assert_eq!(op.state, PolicyOpState::Accepted);
    let (tail, hwm) = h.tail_and_hwm(&u);
    assert_eq!(tail, hwm);
    assert!(
        !h.server
            .policy_of(&u, Category::PhoneReports)
            .unwrap()
            .enabled
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn t_s16_without_the_token_verifier_a_post_snapshot_device_would_be_unknown() {
    let h = Harness::start().await;
    let u = h.user("u1");
    let a = h.pc(&u, "PC-A").await;
    a.synced().await;
    let file = h.backup();
    let d = h.pc(&u, "PC-D").await;
    d.synced().await;
    let h = h
        .restore_with(
            &file,
            Faults {
                replay_without_verifier: true,
                ..Faults::default()
            },
        )
        .await;
    assert!(matches!(h.startup, Startup::Hold { .. }));
    // D's own token no longer matches: its receipt cannot count.
    let body = serde_json::json!({ "head": receipt_of(&d) }).to_string();
    let (s, b) = post_receipt(&h, Some(&token_of(&d)), &body).await;
    assert_eq!((s, b.contains("unknown_token")), (401, true));
    assert_eq!(evidence(&h, &u, &d.dev.device_ref), Some(None));
}

#[tokio::test(flavor = "multi_thread")]
async fn t_d8_t_d11_a_stale_mirror_holds_one_user_and_a_later_receipt_discards_only_them() {
    let h = Harness::start().await;
    let ux = h.user("ux");
    let uy = h.user("uy");
    let a = h.pc(&ux, "PC-A").await;
    let b = h.pc(&ux, "PC-B").await;
    let c = h.pc(&uy, "PC-C").await;
    for pc in [&a, &b, &a, &c] {
        pc.synced().await;
    }
    enable(&a, Category::Hydration).await;
    for pc in [&b, &c] {
        pc.synced().await;
    }
    // The database and the mirror are copied together (an old disk image).
    let file = h.backup();
    let mirror_copy = std::fs::read(&h.mirror).unwrap();
    // A deletion acknowledged to B after that is lost with the disk.
    let t = today(&h);
    b.with(|s| {
        s.record_deletion(Category::Hydration, DateRange { from: t, to: t })
            .unwrap()
    });
    b.synced().await;
    let b_seq = b.with(|s| s.head_receipt().unwrap().unwrap().head.seq);
    let h = h
        .restore_after(&file, |mirror| {
            std::fs::write(mirror, &mirror_copy).unwrap()
        })
        .await;
    assert_eq!(h.startup, Startup::Hold { replayed: 0 });
    assert!(in_hold(&h, &ux) && in_hold(&h, &uy));
    assert!(h.tail_and_hwm(&ux).0 < i64::try_from(b_seq).unwrap());
    h.server.witness_users(true).unwrap();
    // Y: C's receipt matches Y's tail; witnessed, Y is released at once.
    c.synced().await;
    h.server.witness_user(&uy, true).unwrap();
    assert!(!in_hold(&h, &uy));
    // X: A presents a receipt at or below the tail and sends a disable during hold.
    a.with(|s| s.disable_sharing(Category::Hydration).unwrap());
    let a_receipt = receipt_of(&a);
    assert!(matches!(
        a.synced().await,
        SyncOutcome::Hold {
            evidence: true,
            control_sent: 1
        }
    ));
    assert_eq!(
        receipt_of(&a),
        a_receipt,
        "the pre-restore receipt is kept (T-D11)"
    );
    h.server.witness_user(&ux, true).unwrap();
    assert!(
        in_hold(&h, &ux),
        "time and partial evidence never release X"
    );
    assert_eq!(summaries(&h, &ux), 0);
    // B presents its receipt for the lost deletion: X takes the discard path, Y not.
    let out = b.sync().await;
    assert_eq!(
        out,
        Ok(SyncOutcome::Hold {
            evidence: false,
            control_sent: 0
        })
    );
    assert!(!in_hold(&h, &ux));
    assert_eq!(h.audits("user_discarded"), 1);
    assert_eq!(a.sync().await.unwrap_err(), FlowError::Revoked);
    assert!(
        matches!(c.synced().await, SyncOutcome::Synced { .. }),
        "Y is unaffected"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn t_d8_variant_the_seven_day_maximum_discards_and_time_never_releases() {
    let h = Harness::start().await;
    let ux = h.user("ux");
    let a = h.pc(&ux, "PC-A").await;
    let b = h.pc(&ux, "PC-B").await;
    for pc in [&a, &b, &a] {
        pc.synced().await;
    }
    let file = h.backup();
    let h = h.restore(&file).await;
    h.server.witness_users(true).unwrap();
    h.server.witness_user(&ux, true).unwrap();
    a.synced().await;
    // B never checks in.
    h.clock.advance(Duration::days(6));
    h.server.maintenance().unwrap();
    assert!(in_hold(&h, &ux));
    h.clock.advance(Duration::days(1));
    h.server.maintenance().unwrap();
    assert!(!in_hold(&h, &ux));
    assert_eq!(h.audits("user_discarded"), 1);
    assert_eq!(b.sync().await.unwrap_err(), FlowError::Revoked);
}

#[tokio::test(flavor = "multi_thread")]
async fn t_d10_an_unwitnessed_device_list_discards_only_that_user() {
    let h = Harness::start().await;
    let ux = h.user("ux");
    let uy = h.user("uy");
    let a = h.pc(&ux, "PC-A").await;
    let c = h.pc(&uy, "PC-C").await;
    a.synced().await;
    c.synced().await;
    let file = h.backup();
    let h = h.restore(&file).await;
    h.server.witness_users(true).unwrap();
    a.synced().await;
    c.synced().await;
    h.server.witness_user(&ux, false).unwrap();
    h.server.witness_user(&uy, true).unwrap();
    assert_eq!(a.sync().await.unwrap_err(), FlowError::Revoked);
    assert!(matches!(c.synced().await, SyncOutcome::Synced { .. }));
}

#[tokio::test(flavor = "multi_thread")]
async fn t_d9_t_d12_t_s14_a_forked_server_is_detected_on_both_sides() {
    let h = Harness::start().await;
    let u = h.user("u1");
    let uy = h.user("uy");
    let a = h.pc(&u, "PC-A").await;
    let c = h.pc(&uy, "PC-C").await;
    a.synced().await;
    c.synced().await;
    enable(&a, Category::Hydration).await;
    // Two servers from one key and one database copy.
    let h = h.restart().await;
    let fork_root = tempfile::tempdir().unwrap();
    let fork_dir = fork_root.path().join("data");
    copy_dir(&h.dir, &fork_dir);
    let fork_mirror = Harness::mirror_for(&fork_dir);
    std::fs::copy(&h.mirror, &fork_mirror).unwrap();
    let h2 = Harness::open_with(
        &fork_dir,
        &fork_mirror,
        h.clock.clone(),
        h.server.config.clone(),
        common::test_limits(),
    )
    .await
    .unwrap();
    assert_eq!(h2.pin(), h.pin());
    // They diverge: a deletion on the fork (sent by A there), a disable on the original.
    let t = today(&h);
    let fork_client = hy_core::link::LinkClient::new(h2.pin(), h2.api, opts()).unwrap();
    let req = ControlRequest {
        ops: vec![ControlOp::Delete {
            request_id: new_id(),
            category: Category::Hydration,
            from: t.to_string(),
            to: t.to_string(),
        }],
    };
    fork_client.control(&a.dev.token, &req).await.unwrap();
    a.with(|s| s.disable_sharing(Category::Hydration).unwrap());
    a.synced().await;
    let a_receipt = receipt_of(&a);
    // A now reaches the fork: the extension does not verify, nothing of that branch is
    // applied, uploads stop, and A reports its receipt (the fork discards U).
    a.with(|s| {
        s.set_link_manual_address(Some(&h2.api.to_string()))
            .unwrap()
    });
    let a_state_before = a.with(|s| s.sharing_policy(Category::Hydration).unwrap());
    let _original = h.stop().await; // the original goes away; A finds the fork
    let out = a.synced().await;
    assert_eq!(out, SyncOutcome::HistoryMismatch);
    assert!(a.with(|s| s.server_status().unwrap().mismatch));
    assert_eq!(
        a.with(|s| s.sharing_policy(Category::Hydration).unwrap()),
        a_state_before
    );
    assert_eq!(receipt_of(&a), a_receipt);
    assert_eq!(
        h2.audits("user_discarded"),
        1,
        "the fork saw a receipt it never issued"
    );
    // T-D12: a receipt signed by another server key is refused and logged.
    let other = Harness::start().await;
    let stranger = other.confirmed_device("uz", "PC-Z").await;
    let mut zs = stranger.store(other.clock.clone());
    stranger.sync(&mut zs).await.unwrap();
    let z_receipt = zs.head_receipt().unwrap().unwrap().to_wire();
    let c_token = b64(c.dev.token.expose());
    let body = serde_json::json!({ "head": z_receipt }).to_string();
    let (s, b) = post_receipt(&h2, Some(&c_token), &body).await;
    assert_eq!(s, 200, "{b}");
    assert!(b.contains("\"recorded\":false"));
    assert!(h2.audits("receipt_refused") >= 1);
    assert_eq!(
        h2.audits("user_discarded"),
        1,
        "no discard for a foreign key"
    );
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let target = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &target);
        } else {
            std::fs::copy(e.path(), target).unwrap();
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn t_d12_t_s21_forged_and_late_receipts() {
    let h = Harness::start().await;
    let ux = h.user("ux");
    let uy = h.user("uy");
    let a = h.pc(&ux, "PC-A").await;
    let c = h.pc(&uy, "PC-C").await;
    a.synced().await;
    c.synced().await;
    enable(&a, Category::Hydration).await;
    let a_receipt = receipt_of(&a);
    // Bad signature, another user's receipt, a valid receipt from another user's device:
    // refused, logged, no discard.
    let mut forged = a_receipt.clone().into_bytes();
    let last = forged.len() - 3;
    forged[last] = if forged[last] == b'A' { b'B' } else { b'A' };
    let forged = String::from_utf8(forged).unwrap();
    for (pc, head) in [(&a, forged.as_str()), (&c, a_receipt.as_str())] {
        let body = serde_json::json!({ "head": head }).to_string();
        let (s, b) = post_receipt(&h, Some(&token_of(pc)), &body).await;
        assert!(s == 200 || s == 400, "{s} {b}");
        assert!(!b.contains("\"recorded\":true"));
    }
    assert!(h.audits("receipt_refused") >= 2);
    assert_eq!(h.audits("user_discarded"), 0);

    // Late evidence after a release: a receipt at or below the tail with a matching hash
    // changes nothing.
    let file = h.backup();
    let h = h.restore(&file).await;
    h.server.witness_users(true).unwrap();
    for pc in [&a, &c] {
        pc.synced().await;
    }
    h.server.witness_user(&ux, true).unwrap();
    h.server.witness_user(&uy, true).unwrap();
    assert!(!in_hold(&h, &ux) && !in_hold(&h, &uy));
    let body = serde_json::json!({ "head": a_receipt }).to_string();
    let (s, _) = post_receipt(&h, Some(&token_of(&a)), &body).await;
    assert_eq!(s, 200);
    assert_eq!(h.audits("user_discarded"), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn t_d12_an_unknown_device_with_a_receipt_beyond_the_tail_discards_that_user() {
    let h = Harness::start().await;
    let ux = h.user("ux");
    let uy = h.user("uy");
    let a = h.pc(&ux, "PC-A").await;
    let c = h.pc(&uy, "PC-C").await;
    a.synced().await;
    c.synced().await;
    let file = h.backup();
    let mirror_copy = std::fs::read(&h.mirror).unwrap();
    enable(&a, Category::Hydration).await;
    let beyond = receipt_of(&a);
    let h = h
        .restore_after(&file, |m| std::fs::write(m, &mirror_copy).unwrap())
        .await;
    assert_eq!(h.startup, Startup::Hold { replayed: 0 });
    assert!(in_hold(&h, &ux));
    // No token at all: never evidence, but the receipt proves X's history incomplete.
    let body = serde_json::json!({ "head": beyond }).to_string();
    let (s, _) = post_receipt(&h, Some(&b64(&[3u8; 32])), &body).await;
    assert_eq!(s, 401);
    assert!(!in_hold(&h, &ux));
    assert_eq!(h.audits("user_discarded"), 1);
    assert!(in_hold(&h, &uy), "Y untouched");
}

#[tokio::test(flavor = "multi_thread")]
async fn t_d13_local_use_goes_on_during_hold_with_sharing_shown_paused() {
    let h = Harness::start().await;
    let u = h.user("u1");
    let a = h.pc(&u, "PC-A").await;
    a.synced().await;
    let file = h.backup();
    let h = h.restore(&file).await;
    assert!(matches!(a.synced().await, SyncOutcome::Hold { .. }));
    assert!(a.with(|s| s.server_status().unwrap().hold));
    a.water(morning(&h), 300);
    let t = today(&h);
    let facts = a.with(|s| s.day_facts(t, t).unwrap());
    assert_eq!(facts[0].water_ml, 300);
    let fast = hy_core::backup::KdfParams {
        memory_kib: 64,
        passes: 1,
        lanes: 1,
    };
    assert!(a.with(|s| s.export_backup_with("long enough password", fast).is_ok()));
    drop(h);
}

#[tokio::test(flavor = "multi_thread")]
async fn t_s13_a_restored_or_cloned_pc_is_reset_then_revoked_on_a_repeat() {
    let h = Harness::start().await;
    let u = h.user("u1");
    let a = h.pc(&u, "PC-A").await;
    a.synced().await;
    enable(&a, Category::Hydration).await;
    a.water(morning(&h), 400);
    // An older copy of A's database: same pairing, receipt from before the enable.
    let old = Pc {
        dev: common::Device {
            device_ref: a.dev.device_ref,
            user_id: a.dev.user_id,
            token: hy_core::secret::Secret::new(*a.dev.token.expose()),
            link: a.dev.link.clone(),
            vault: a.dev.vault.clone(),
        },
        store: std::sync::Mutex::new(a.dev.store(h.clock.clone())),
    };
    // Bring the copy to the receipt A had before enabling (seq of the confirmation).
    let link = old.with(|s| s.server_link().unwrap().unwrap());
    let fetched = hy_core::link::flow::fetch_state(&link, 0, &*a.dev.vault, opts())
        .await
        .unwrap();
    let StateResponse::Confirmed { entries, .. } = &fetched.state else {
        panic!()
    };
    assert!(!entries.is_empty());
    old.with(|s| s.apply_confirmed_state(&fetched.state).unwrap());
    // A moves on (a new entry), so the copy's receipt is now older.
    a.with(|s| s.disable_sharing(Category::Hydration).unwrap());
    a.synced().await;
    a.synced().await; // A echoes its newest receipt
    // The copy reconnects: device_regressed; every category off, queue empty, notice.
    let out = old.synced().await;
    assert!(
        matches!(
            out,
            SyncOutcome::Synced {
                regressed: true,
                ..
            }
        ),
        "{out:?}"
    );
    assert!(
        Category::ALL
            .iter()
            .all(|c| !old.with(|s| s.sharing_policy(*c).unwrap().enabled))
    );
    assert!(old.with(|s| s.outbox().unwrap().is_empty()));
    assert!(
        old.with(|s| s.server_notices().unwrap())
            .iter()
            .any(|n| n.kind == NoticeKind::Regressed)
    );
    let flagged: Option<i64> = h
        .server
        .db()
        .query_row(
            "SELECT regressed_at FROM devices WHERE device_ref = ?1",
            [a.dev.device_ref.as_slice()],
            |r| r.get(0),
        )
        .unwrap();
    assert!(flagged.is_some(), "admin flag");
    // Two copies alternately within 24 h: the device is revoked.
    enable(&a, Category::PhoneReports).await;
    a.synced().await;
    let out = old.sync().await;
    assert_eq!(out.unwrap_err(), FlowError::Revoked);
    assert_eq!(a.sync().await.unwrap_err(), FlowError::Revoked);
}

#[tokio::test(flavor = "multi_thread")]
async fn t_s20_t_d6_a_tampered_missing_or_foreign_mirror_takes_the_global_discard_path() {
    type Tamper = fn(&std::path::Path, &std::path::Path);
    let cases: Vec<(&str, Tamper)> = vec![
        ("flip one byte", |_, m| {
            let mut b = std::fs::read(m).unwrap();
            let i = b.len() / 2;
            b[i] ^= 0x01;
            std::fs::write(m, b).unwrap();
        }),
        ("truncate in the middle", |_, m| {
            let b = std::fs::read(m).unwrap();
            let lines: Vec<&[u8]> = b.split(|c| *c == b'\n').collect();
            let mut out = Vec::new();
            for (i, l) in lines.iter().enumerate() {
                if i != 2 {
                    out.extend_from_slice(l);
                    out.push(b'\n');
                }
            }
            out.pop();
            std::fs::write(m, out).unwrap();
        }),
        ("reorder two lines", |_, m| {
            let t = std::fs::read_to_string(m).unwrap();
            let mut lines: Vec<&str> = t.lines().collect();
            lines.swap(2, 3);
            std::fs::write(m, lines.join("\n") + "\n").unwrap();
        }),
        ("another install's mirror", |_, m| {
            let t = std::fs::read_to_string(m).unwrap();
            let (first, rest) = t.split_once('\n').unwrap();
            let mut parts: Vec<String> = first.split(' ').map(str::to_owned).collect();
            parts[1] = "00".repeat(16);
            std::fs::write(m, parts.join(" ") + "\n" + rest).unwrap();
        }),
        ("missing", |_, m| std::fs::remove_file(m).unwrap()),
    ];
    for (name, tamper) in cases {
        let h = Harness::start().await;
        let u = h.user("u1");
        let a = h.pc(&u, "PC-A").await;
        a.synced().await;
        enable(&a, Category::Hydration).await;
        a.water(morning(&h), 400);
        a.synced().await;
        let h = h.restart_after(tamper).await;
        assert!(
            matches!(h.startup, Startup::GlobalDiscard { .. }),
            "{name}: {:?}",
            h.startup
        );
        assert_eq!(summaries(&h, &u), 0, "{name}");
        assert!(!h.server.policy_of(&u, Category::Hydration).unwrap().enabled);
        assert_eq!(a.sync().await.unwrap_err(), FlowError::Revoked, "{name}");
        // A new mirror was started and verifies.
        let h = h.restart().await;
        assert_eq!(h.startup, Startup::Opened, "{name}");
    }
    // The administrator cannot confirm the list of users: global discard too.
    let h = Harness::start().await;
    let a = h.confirmed_device("u1", "PC-A").await;
    let file = h.backup();
    let h = h.restore(&file).await;
    h.server.witness_users(false).unwrap();
    assert_eq!(h.audits("global_discard"), 1);
    let _ = a;
}

#[tokio::test(flavor = "multi_thread")]
async fn t_d6_a_mirror_older_than_the_database_high_water_is_a_global_discard() {
    let h = Harness::start().await;
    let u = h.user("u1");
    let a = h.pc(&u, "PC-A").await;
    a.synced().await;
    let old_mirror = std::fs::read(&h.mirror).unwrap();
    enable(&a, Category::Hydration).await;
    let h = h
        .restart_after(|_, m| std::fs::write(m, &old_mirror).unwrap())
        .await;
    assert_eq!(
        h.startup,
        Startup::GlobalDiscard {
            reason: "mirror_behind"
        }
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn after_release_rows_sent_after_the_snapshot_are_sent_again() {
    let h = Harness::start().await;
    let u = h.user("u1");
    let a = h.pc(&u, "PC-A").await;
    a.synced().await;
    enable(&a, Category::Hydration).await;
    let file = h.backup();
    a.water(morning(&h), 600);
    a.synced().await;
    assert_eq!(summaries(&h, &u), 1);
    let h = h.restore(&file).await;
    assert_eq!(summaries(&h, &u), 0, "the row is not in the snapshot");
    h.server.witness_users(true).unwrap();
    h.server.witness_user(&u, true).unwrap();
    assert!(matches!(
        a.synced().await,
        SyncOutcome::Hold { evidence: true, .. }
    ));
    assert!(!in_hold(&h, &u));
    let out = a.synced().await;
    assert!(
        matches!(out, SyncOutcome::Synced { uploaded: 1, .. }),
        "{out:?}"
    );
    assert_eq!(summaries(&h, &u), 1);
}

/// Each `(date, payload)` the server holds for `origin` in `category`, oldest first.
fn rows_for(h: &Harness, origin: uuid::Uuid, category: Category) -> Vec<(String, String)> {
    let conn = h.server.db();
    let mut stmt = conn
        .prepare(
            "SELECT local_date, payload FROM summaries
             WHERE origin_device = ?1 AND category = ?2 ORDER BY local_date",
        )
        .unwrap();
    stmt.query_map(
        rusqlite::params![origin.as_bytes().as_slice(), category.as_str()],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .unwrap()
    .collect::<Result<_, _>>()
    .unwrap()
}

/// The `(date, payload)` rows `pc` should have on the server for `days`: its own current
/// record of each day that has one.
fn expected_rows(pc: &Pc, days: &[NaiveDate], category: Category) -> Vec<(String, String)> {
    days.iter()
        .filter_map(|d| {
            pc.with(|s| s.share_record(category, *d).unwrap())
                .map(|r| (d.format("%Y-%m-%d").to_string(), r.to_string()))
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn an_aged_snapshot_and_a_prolonged_hold_rebuild_the_whole_retained_history() {
    // The oldest snapshot that can exist (backups are kept 30 days at most) is restored,
    // and the hold lasts six of its seven days. After release every row each PC shared
    // since the snapshot, weeks older than the ordinary lookback, is on the server again,
    // exactly as each PC holds it, and a deletion made after the snapshot stays in force.
    let h = Harness::start().await;
    let u = h.user("u1");
    let a = h.pc(&u, "PC-A").await;
    let b = h.pc(&u, "PC-B").await;
    a.synced().await;
    b.synced().await;
    enable(&a, Category::Hydration).await;
    b.synced().await;
    let d0 = today(&h);
    a.water(morning(&h), 400);
    b.water(morning(&h), 250);
    a.synced().await;
    b.synced().await;
    let file = h.backup();
    let mut days = vec![d0];
    // 29 more days of ordinary daily use after the snapshot (B logs every other day).
    for n in 1..=29 {
        h.clock.advance(Duration::days(1));
        days.push(today(&h));
        a.water(morning(&h), 100 + n);
        if n % 2 == 0 {
            b.water(morning(&h), 200 + n);
        }
        if n == 12 {
            // A deletes day 5 for everyone; the mirror keeps the deletion.
            let d5 = days[5];
            a.with(|s| {
                s.record_deletion(Category::Hydration, DateRange { from: d5, to: d5 })
                    .unwrap();
            });
        }
        a.synced().await;
        b.synced().await;
    }
    let d5 = days[5].format("%Y-%m-%d").to_string();
    let before_a = rows_for(&h, a.origin(), Category::Hydration);
    assert_eq!(before_a.len(), 29, "day 0 to day 29 shared, without day 5");
    assert!(!before_a.iter().any(|(d, _)| *d == d5));

    let h = h.restore(&file).await;
    assert_eq!(
        rows_for(&h, a.origin(), Category::Hydration).len(),
        1,
        "only day 0 is in the snapshot"
    );
    h.server.witness_users(true).unwrap();
    h.server.witness_user(&u, true).unwrap();
    // A checks in at once; B stays away for six days. Use goes on during the hold.
    assert!(matches!(
        a.synced().await,
        SyncOutcome::Hold { evidence: true, .. }
    ));
    for n in 30..=35 {
        h.clock.advance(Duration::days(1));
        days.push(today(&h));
        a.water(morning(&h), 100 + n);
        b.water(morning(&h), 200 + n);
        assert!(matches!(a.synced().await, SyncOutcome::Hold { .. }));
        assert!(in_hold(&h, &u));
    }
    assert!(matches!(
        b.synced().await,
        SyncOutcome::Hold { evidence: true, .. }
    ));
    assert!(!in_hold(&h, &u), "released on B's evidence");
    for pc in [&a, &b, &a, &b] {
        assert!(matches!(pc.synced().await, SyncOutcome::Synced { .. }));
    }

    for pc in [&a, &b] {
        let rows = rows_for(&h, pc.origin(), Category::Hydration);
        // Local entries stay after a deletion; only the shared copy is gone.
        let shareable: Vec<NaiveDate> = days.iter().copied().filter(|d| *d != days[5]).collect();
        assert_eq!(
            rows,
            expected_rows(pc, &shareable, Category::Hydration),
            "every day this PC holds, weeks older than the ordinary lookback included"
        );
        assert!(!rows.iter().any(|(d, _)| *d == d5), "the deletion holds");
    }
    // Day 0 plus days 1 to 35 without day 5 for A; day 0, the even days to 28 and the
    // six hold days for B. A's day 1 and B's day 2 are 34 and 33 days old at release.
    let a_rows = rows_for(&h, a.origin(), Category::Hydration);
    let b_rows = rows_for(&h, b.origin(), Category::Hydration);
    assert_eq!((a_rows.len(), b_rows.len()), (35, 21));
    assert_eq!(a_rows[1].0, days[1].format("%Y-%m-%d").to_string());
    assert_eq!(b_rows[1].0, days[2].format("%Y-%m-%d").to_string());
    // The next ordinary sync sends nothing more: the re-send ran once.
    assert!(matches!(
        a.synced().await,
        SyncOutcome::Synced { uploaded: 0, .. }
    ));
}
