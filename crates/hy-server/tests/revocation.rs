//! Revocation (V1-5 contract 6.3 and 14.2): T-S19 revocation while offline, self-unpair,
//! re-pairing the same PC, and origin conflicts.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{Harness, error_of, opts};
use hy_core::link::flow::{FlowError, unpair_remote};
use hy_core::schema::{Category, ConsentKind};
use hy_core::store::StateOutcome;
use hy_server::server::DeviceState;

#[tokio::test(flavor = "multi_thread")]
async fn t_s19_a_pc_revoked_while_offline_unpairs_locally_on_reconnect() {
    let h = Harness::start().await;
    let b = h.confirmed_device("u1", "PC-B").await;
    let mut store = b.store(h.clock.clone());
    assert!(matches!(
        b.sync(&mut store).await.unwrap(),
        Some(StateOutcome::Applied {
            newly_confirmed: true,
            ..
        })
    ));
    // Sharing on and a summary queued (the enable round trip itself is V1-5b; here the
    // local acceptance stands in for it).
    let op = store
        .request_enable(Category::Hydration, "notice-1")
        .unwrap();
    let today = h.clock_date();
    store.accept_enable(op, 1, today).unwrap();
    store
        .enqueue_upload(
            Category::Hydration,
            today,
            1,
            &serde_json::json!({"ml": 1500}),
        )
        .unwrap();
    assert_eq!(store.outbox().unwrap().len(), 1);

    // B is offline; the administrator revokes it.
    h.server.revoke_device(&b.device_ref).unwrap();

    // On reconnect: 401 device_revoked, then the local unpair transaction.
    let raw = h
        .client(1)
        .raw(reqwest::Method::GET, "/v1/state", Some(&b.token), None)
        .await
        .unwrap();
    assert_eq!(
        (raw.status, error_of(&raw).as_str()),
        (401, "device_revoked")
    );
    let err = b.sync(&mut store).await.err().unwrap();
    assert_eq!(err, FlowError::Revoked);
    assert!(store.unpair_locally().unwrap());
    assert!(store.outbox().unwrap().is_empty(), "outbox purged");
    assert!(!store.sharing_policy(Category::Hydration).unwrap().enabled);
    assert!(store.server_link().unwrap().is_none());
    assert!(store.head_receipt().unwrap().is_none());
    assert!(store.paired_devices().unwrap().is_empty());
    let log = store.consent_log().unwrap();
    assert_eq!(log.last().unwrap().kind, ConsentKind::Unpaired);
    // Nothing can be queued without a link.
    assert!(
        store
            .enqueue_upload(Category::Hydration, today, 2, &serde_json::json!({"ml": 1}))
            .is_err()
    );
    // The revocation is in the user's chain.
    let reason: String = h
        .server
        .db()
        .query_row(
            "SELECT revoke_reason FROM devices WHERE device_ref = ?1",
            [b.device_ref.as_slice()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(reason, "admin");
    assert_eq!(
        h.count("control_log"),
        3,
        "pairing, device_confirmed, revocation"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn self_unpair_revokes_the_token_at_once_and_is_recorded() {
    let h = Harness::start().await;
    let d = h.confirmed_device("u1", "PC-1").await;
    let mut store = d.store(h.clock.clone());
    d.sync(&mut store).await.unwrap();
    let link = store.server_link().unwrap().unwrap();
    unpair_remote(&link, &d.vault, opts()).await.unwrap();
    assert!(store.unpair_locally().unwrap());
    let row = h.server.devices().unwrap().into_iter().next().unwrap();
    assert_eq!(row.state, DeviceState::Revoked);
    assert_eq!(row.revoke_reason.as_deref(), Some("self"));
    let raw = h
        .client(1)
        .raw(reqwest::Method::GET, "/v1/state", Some(&d.token), None)
        .await
        .unwrap();
    assert_eq!(error_of(&raw), "device_revoked");
    // A pending device may unpair itself too.
    let u = h.user("u2");
    let (_, p) = h.pair(&h.slip(&u), "PC-2").await.unwrap();
    let raw = h
        .client(1)
        .raw(reqwest::Method::POST, "/v1/unpair", Some(&p.token), None)
        .await
        .unwrap();
    assert_eq!(raw.status, 200);
}

#[tokio::test(flavor = "multi_thread")]
async fn re_pairing_the_same_pc_supersedes_the_old_device_and_another_user_conflicts() {
    let h = Harness::start().await;
    let u = h.user("u1");
    let origin = common::new_id();
    let (_, first) = h
        .pair_as(&h.slip(&u), "PC-1", origin, opts())
        .await
        .unwrap();
    h.confirm(&first);
    let (_, second) = h
        .pair_as(&h.slip(&u), "PC-1", origin, opts())
        .await
        .unwrap();
    let devices = h.server.devices().unwrap();
    let get = |r: &[u8; 16]| devices.iter().find(|d| &d.device_ref == r).unwrap();
    assert_eq!(get(&first.device_ref).state, DeviceState::Revoked);
    assert_eq!(
        get(&first.device_ref).revoke_reason.as_deref(),
        Some("superseded")
    );
    assert_eq!(
        get(&second.device_ref).state,
        DeviceState::Pending,
        "a re-pair needs confirming again"
    );
    // The same PC (origin) cannot be paired to another user while active or pending.
    let other = h.user("u2");
    let err = h
        .pair_as(&h.slip(&other), "PC-1", origin, opts())
        .await
        .err()
        .unwrap();
    assert_eq!(
        err,
        FlowError::Server(hy_core::protocol::wire::WireError::OriginConflict)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_token_that_cannot_be_opened_here_means_re_pair_needed_without_touching_data() {
    let h = Harness::start().await;
    let d = h.confirmed_device("u1", "PC-1").await;
    let mut store = d.store(h.clock.clone());
    // The database was copied to another profile: its vault cannot open the blob.
    let elsewhere = hy_core::secret::EphemeralVault::new();
    let link = store.server_link().unwrap().unwrap();
    let err = hy_core::link::flow::fetch_state(&link, 0, &elsewhere, opts())
        .await
        .err()
        .unwrap();
    assert_eq!(err, FlowError::RepairNeeded);
    assert!(store.server_link().unwrap().is_some(), "nothing removed");
    assert!(d.sync(&mut store).await.is_ok());
}

trait ClockDate {
    fn clock_date(&self) -> chrono::NaiveDate;
}

impl ClockDate for Harness {
    fn clock_date(&self) -> chrono::NaiveDate {
        use hy_core::clock::Clock;
        self.clock.now().date_naive()
    }
}
