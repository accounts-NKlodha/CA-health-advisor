//! The companion's side of a server link (slice V1-5a; contract 4.2, 4.5, 6.3 and 7.4):
//! storing a pairing, applying a confirmed state only after the signed head and the
//! chain extension verify, and the local unpair transaction.

// Test helpers outside `#[test]` functions may unwrap too.
#![allow(clippy::unwrap_used)]

use std::sync::Arc;

use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::signature::Ed25519KeyPair;
use chrono::{TimeZone, Utc};
use hy_core::clock::{Clock, ManualClock};
use hy_core::protocol::chain::{ControlEntry, EntryBody, RevocationReason, genesis};
use hy_core::protocol::head::{Head, public_key};
use hy_core::protocol::wire::{DeviceInfo, StateResponse, WireEntry};
use hy_core::protocol::{b64, key_id};
use hy_core::schema::{Category, ConsentKind};
use hy_core::store::{NewLink, StateOutcome, Store};
use uuid::Uuid;

/// A stand-in for the server's chain and head key.
struct FakeServer {
    pkcs8: Vec<u8>,
    key: Ed25519KeyPair,
    user: [u8; 16],
    entries: Vec<ControlEntry>,
    hash: [u8; 32],
}

impl FakeServer {
    fn new(user: [u8; 16]) -> Self {
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new())
            .unwrap()
            .as_ref()
            .to_vec();
        let key = Ed25519KeyPair::from_pkcs8(&pkcs8).unwrap();
        let hash = genesis(&key_id(&public_key(&key)), &user);
        Self {
            pkcs8,
            key,
            user,
            entries: Vec::new(),
            hash,
        }
    }

    /// A copy of this server (same key, same chain so far) that then diverges.
    fn fork(&self) -> Self {
        Self {
            pkcs8: self.pkcs8.clone(),
            key: Ed25519KeyPair::from_pkcs8(&self.pkcs8).unwrap(),
            user: self.user,
            entries: self.entries.clone(),
            hash: self.hash,
        }
    }

    fn head_key(&self) -> [u8; 32] {
        public_key(&self.key)
    }

    fn push(&mut self, body: EntryBody) {
        let e = ControlEntry {
            user_id: self.user,
            seq: self.entries.len() as u64 + 1,
            at_ms: 1_790_000_000_000 + self.entries.len() as u64,
            body,
        };
        self.hash = e.hash(&self.hash);
        self.entries.push(e);
    }

    fn head(&self) -> String {
        Head {
            key_id: key_id(&self.head_key()),
            user_id: self.user,
            seq: self.entries.len() as u64,
            chain_hash: self.hash,
            issued_at_ms: 1_790_000_100_000,
        }
        .sign(&self.key)
        .to_wire()
    }

    fn wire(entries: &[ControlEntry]) -> Vec<WireEntry> {
        entries
            .iter()
            .map(|e| WireEntry {
                seq: e.seq,
                at_ms: e.at_ms,
                kind: e.body.kind(),
                body: b64(&e.body.encode()),
            })
            .collect()
    }

    fn state(&self, after: u64, devices: Vec<DeviceInfo>) -> StateResponse {
        StateResponse::Confirmed {
            alias: "u1".into(),
            confirmed_on: "2026-10-23".into(),
            devices,
            entries: Self::wire(&self.entries[usize::try_from(after).unwrap()..]),
            more: false,
            head: Some(self.head()),
            hold: false,
        }
    }
}

fn device(id: Uuid, label: &str) -> DeviceInfo {
    DeviceInfo {
        device_ref: id,
        label: label.into(),
        paired_at_ms: 1,
        last_seen_ms: None,
        confirmed: true,
    }
}

struct Setup {
    store: Store,
    server: FakeServer,
    me: Uuid,
}

fn setup() -> Setup {
    let clock: Arc<dyn Clock> = Arc::new(ManualClock::new(
        Utc.with_ymd_and_hms(2026, 10, 23, 6, 0, 0).unwrap(),
    ));
    let mut store = Store::open_in_memory(clock).unwrap();
    let user = Uuid::from_bytes([7; 16]);
    let me = Uuid::from_bytes([9; 16]);
    let mut server = FakeServer::new(*user.as_bytes());
    server.push(EntryBody::Pairing {
        device_ref: *me.as_bytes(),
        origin_device: *store.device_id().as_bytes(),
        token_verifier: [1; 32],
    });
    let link = NewLink {
        pin: [3; 32],
        head_key: server.head_key(),
        host: "office-pc".into(),
        port: 8443,
        last_ip: Some("192.168.1.20:8443".into()),
        user_id: user,
        device_ref: me,
        alias: "u1".into(),
        token_blob: vec![0; 64],
    };
    store.save_pairing(&link, "notice-1").unwrap();
    Setup { store, server, me }
}

#[test]
fn pairing_is_stored_pending_with_every_category_off_and_logged() {
    let mut s = setup();
    let link = s.store.server_link().unwrap().unwrap();
    assert!(!link.confirmed && link.confirmed_on.is_none());
    assert_eq!(link.server_key_id, key_id(&s.server.head_key()));
    for &c in Category::ALL {
        assert!(!s.store.sharing_policy(c).unwrap().enabled);
    }
    let kinds: Vec<_> = s
        .store
        .consent_log()
        .unwrap()
        .into_iter()
        .map(|e| e.kind)
        .collect();
    assert_eq!(
        &kinds[kinds.len() - 2..],
        [ConsentKind::NoticeShown, ConsentKind::Paired]
    );
    // A second pairing needs an unpair first.
    let again = NewLink {
        pin: [4; 32],
        head_key: [5; 32],
        host: "x".into(),
        port: 1,
        last_ip: None,
        user_id: Uuid::from_bytes([1; 16]),
        device_ref: Uuid::from_bytes([2; 16]),
        alias: "u2".into(),
        token_blob: vec![],
    };
    assert!(s.store.save_pairing(&again, "notice-1").is_err());
    // A pending answer changes nothing.
    let pending = StateResponse::Pending { alias: "u1".into() };
    assert_eq!(s.store.apply_confirmed_state(&pending).unwrap(), None);
    assert!(s.store.head_receipt().unwrap().is_none());
}

#[test]
fn a_verified_confirmation_confirms_the_pc_and_stores_the_receipt() {
    let mut s = setup();
    s.server.push(EntryBody::DeviceConfirmed {
        device_ref: *s.me.as_bytes(),
    });
    let out = s
        .store
        .apply_confirmed_state(&s.server.state(0, vec![device(s.me, "PC-1")]))
        .unwrap();
    assert_eq!(
        out,
        Some(StateOutcome::Applied {
            seq: 2,
            newly_confirmed: true,
            new_devices: vec![]
        })
    );
    let link = s.store.server_link().unwrap().unwrap();
    assert!(link.confirmed);
    assert_eq!(link.confirmed_on.unwrap().to_string(), "2026-10-23");
    assert_eq!(s.store.head_receipt().unwrap().unwrap().head.seq, 2);

    // A later device of the same user is announced once.
    let other = Uuid::from_bytes([11; 16]);
    s.server.push(EntryBody::Pairing {
        device_ref: [11; 16],
        origin_device: [12; 16],
        token_verifier: [2; 32],
    });
    s.server.push(EntryBody::DeviceConfirmed {
        device_ref: [11; 16],
    });
    let devices = vec![device(s.me, "PC-1"), device(other, "PC-2")];
    let out = s
        .store
        .apply_confirmed_state(&s.server.state(2, devices.clone()))
        .unwrap();
    assert_eq!(
        out,
        Some(StateOutcome::Applied {
            seq: 4,
            newly_confirmed: false,
            new_devices: vec![other]
        })
    );
    // A revoked device leaves the list.
    s.server.push(EntryBody::Revocation {
        device_ref: [11; 16],
        reason: RevocationReason::Admin,
    });
    s.store
        .apply_confirmed_state(&s.server.state(4, vec![device(s.me, "PC-1")]))
        .unwrap();
    assert_eq!(s.store.paired_devices().unwrap().len(), 1);
}

fn assert_mismatch_changes_nothing(s: &mut Setup, state: &StateResponse) {
    let link = s.store.server_link().unwrap().unwrap();
    let receipt = s.store.head_receipt().unwrap();
    assert_eq!(
        s.store.apply_confirmed_state(state).unwrap(),
        Some(StateOutcome::HistoryMismatch)
    );
    assert_eq!(s.store.server_link().unwrap().unwrap(), link);
    assert_eq!(s.store.head_receipt().unwrap(), receipt);
}

#[test]
fn a_state_without_this_pcs_confirmation_in_the_chain_is_refused() {
    let mut s = setup();
    // Correctly signed, but the chain holds no device_confirmed for this PC.
    let state = s.server.state(0, vec![device(s.me, "PC-1")]);
    assert_mismatch_changes_nothing(&mut s, &state);
    assert!(!s.store.server_link().unwrap().unwrap().confirmed);
}

#[test]
fn a_head_under_another_key_or_for_another_user_is_refused() {
    let mut s = setup();
    s.server.push(EntryBody::DeviceConfirmed {
        device_ref: *s.me.as_bytes(),
    });
    // Another key signing the same chain (a different server or a forger).
    let mut forger = FakeServer::new(s.server.user);
    forger.entries = s.server.entries.clone();
    forger.hash = s.server.hash;
    let state = forger.state(0, vec![]);
    assert_mismatch_changes_nothing(&mut s, &state);
    // The genuine key, for another user.
    let mut wrong_user = s.server.state(0, vec![]);
    if let StateResponse::Confirmed { head, .. } = &mut wrong_user {
        let h = Head {
            key_id: key_id(&s.server.head_key()),
            user_id: [8; 16],
            seq: 2,
            chain_hash: s.server.hash,
            issued_at_ms: 1,
        }
        .sign(&s.server.key);
        *head = Some(h.to_wire());
    }
    assert_mismatch_changes_nothing(&mut s, &wrong_user);
    // No head at all.
    let mut headless = s.server.state(0, vec![]);
    if let StateResponse::Confirmed { head, .. } = &mut headless {
        *head = None;
    }
    assert_mismatch_changes_nothing(&mut s, &headless);
}

#[test]
fn tampered_missing_or_reordered_entries_are_refused() {
    let mut s = setup();
    s.server.push(EntryBody::DeviceConfirmed {
        device_ref: *s.me.as_bytes(),
    });
    s.server.push(EntryBody::Policy {
        category: Category::Hydration,
        enabled: true,
        share_from: 20_000,
        op_id: [5; 16],
        device_ref: *s.me.as_bytes(),
    });
    let good = s.server.state(0, vec![]);
    let StateResponse::Confirmed { entries, .. } = &good else {
        panic!()
    };

    let mut variants = Vec::new();
    // One flipped byte in one body.
    let mut flipped = entries.clone();
    let mut body = hy_core::protocol::unb64(&flipped[2].body, 256).unwrap();
    body[0] ^= 1;
    flipped[2].body = b64(&body);
    variants.push(flipped);
    // A missing entry.
    let mut missing = entries.clone();
    missing.remove(2);
    variants.push(missing);
    // Two entries swapped.
    let mut swapped = entries.clone();
    swapped.swap(1, 2);
    variants.push(swapped);
    // A changed time.
    let mut retimed = entries.clone();
    retimed[1].at_ms += 1;
    variants.push(retimed);
    for v in variants {
        let mut st = good.clone();
        if let StateResponse::Confirmed { entries, .. } = &mut st {
            *entries = v;
        }
        assert_mismatch_changes_nothing(&mut s, &st);
    }
    assert!(matches!(
        s.store.apply_confirmed_state(&good).unwrap(),
        Some(StateOutcome::Applied { seq: 3, .. })
    ));
}

#[test]
fn a_rolled_back_or_forked_server_is_refused_after_a_receipt() {
    let mut s = setup();
    s.server.push(EntryBody::DeviceConfirmed {
        device_ref: *s.me.as_bytes(),
    });
    let old = s.server.state(0, vec![]);
    s.server.push(EntryBody::Revocation {
        device_ref: [11; 16],
        reason: RevocationReason::Admin,
    });
    s.store
        .apply_confirmed_state(&s.server.state(0, vec![]))
        .unwrap();
    assert_eq!(s.store.head_receipt().unwrap().unwrap().head.seq, 3);
    // The older (validly signed) head is a rollback.
    assert_mismatch_changes_nothing(&mut s, &old);
    // A fork (a second server started from a copy of the key and database) with a
    // different seq 3: the device asks for entries after its receipt and gets the
    // fork's seq 4, which does not extend its own seq 3 to the fork's head.
    let mut fork = s.server.fork();
    fork.entries.truncate(2);
    fork.hash = genesis(&key_id(&fork.head_key()), &fork.user);
    let bodies: Vec<_> = fork.entries.drain(..).map(|e| e.body).collect();
    for b in bodies {
        fork.push(b);
    }
    fork.push(EntryBody::Revocation {
        device_ref: [12; 16],
        reason: RevocationReason::Admin,
    });
    fork.push(EntryBody::DeviceConfirmed {
        device_ref: [12; 16],
    });
    let st = fork.state(3, vec![]);
    assert_mismatch_changes_nothing(&mut s, &st);
}

#[test]
fn unpairing_locally_removes_the_link_receipt_devices_and_queue_in_one_step() {
    let mut s = setup();
    s.server.push(EntryBody::DeviceConfirmed {
        device_ref: *s.me.as_bytes(),
    });
    s.store
        .apply_confirmed_state(&s.server.state(0, vec![device(s.me, "PC-1")]))
        .unwrap();
    assert!(s.store.unpair_locally().unwrap());
    assert!(s.store.server_link().unwrap().is_none());
    assert!(s.store.head_receipt().unwrap().is_none());
    assert!(s.store.paired_devices().unwrap().is_empty());
    assert_eq!(
        s.store.consent_log().unwrap().last().unwrap().kind,
        ConsentKind::Unpaired
    );
    assert!(!s.store.unpair_locally().unwrap(), "already unpaired");
}

#[test]
fn importing_a_backup_onto_a_confirmed_pc_unpairs_it_in_the_same_transaction() {
    let mut s = setup();
    s.server.push(EntryBody::DeviceConfirmed {
        device_ref: *s.me.as_bytes(),
    });
    s.store
        .apply_confirmed_state(&s.server.state(0, vec![device(s.me, "PC-1")]))
        .unwrap();
    let op = s
        .store
        .request_enable(Category::Hydration, "notice-1")
        .unwrap();
    s.store
        .accept_enable(
            op,
            3,
            chrono::NaiveDate::from_ymd_opt(2026, 10, 23).unwrap(),
        )
        .unwrap();
    assert!(s.store.sharing_policy(Category::Hydration).unwrap().enabled);

    let clock: Arc<dyn Clock> = Arc::new(ManualClock::new(
        Utc.with_ymd_and_hms(2026, 10, 22, 6, 0, 0).unwrap(),
    ));
    let other = Store::open_in_memory(clock).unwrap();
    let contents = other.backup_contents().unwrap();

    // A refused import changes nothing: the PC stays paired and sharing stays on.
    let mut refused = contents.clone();
    refused.schema_version += 1;
    assert!(s.store.import_contents(&refused).is_err());
    let mut bad_table = contents.clone();
    bad_table
        .tables
        .insert("server_link".into(), contents.tables["settings"].clone());
    assert!(s.store.import_contents(&bad_table).is_err());
    assert!(s.store.server_link().unwrap().is_some());
    assert!(s.store.head_receipt().unwrap().is_some());
    assert!(s.store.sharing_policy(Category::Hydration).unwrap().enabled);

    s.store.import_contents(&contents).unwrap();
    assert!(s.store.server_link().unwrap().is_none());
    assert!(s.store.head_receipt().unwrap().is_none());
    assert!(s.store.paired_devices().unwrap().is_empty());
    for c in Category::ALL.iter().copied() {
        assert!(!s.store.sharing_policy(c).unwrap().enabled, "{c:?}");
    }
    assert!(s.store.outbox().unwrap().is_empty());
    assert_eq!(
        s.store.consent_log().unwrap().last().unwrap().kind,
        ConsentKind::Unpaired
    );
}

#[test]
fn t_s17_a_forged_policy_entry_starts_nothing_and_stops_uploads() {
    use hy_core::protocol::chain::day_number;
    let mut s = setup();
    s.server.push(EntryBody::DeviceConfirmed {
        device_ref: *s.me.as_bytes(),
    });
    let state = s.server.state(0, vec![device(s.me, "PC-1")]);
    assert!(matches!(
        s.store.apply_confirmed_state(&state).unwrap(),
        Some(StateOutcome::Applied { .. })
    ));
    let today = chrono::NaiveDate::from_ymd_opt(2026, 10, 23).unwrap();
    // The genuine entry enables hydration from today; the copy delivered to this PC is
    // altered to start 30 days earlier, under the genuine head.
    let genuine = EntryBody::Policy {
        category: Category::Hydration,
        enabled: true,
        share_from: day_number(today),
        op_id: [5; 16],
        device_ref: [11; 16],
    };
    s.server.push(genuine);
    let mut state = s.server.state(2, vec![device(s.me, "PC-1")]);
    let StateResponse::Confirmed { entries, .. } = &mut state else {
        panic!()
    };
    let forged = EntryBody::Policy {
        category: Category::Hydration,
        enabled: true,
        share_from: day_number(today) - 30,
        op_id: [5; 16],
        device_ref: [11; 16],
    };
    entries[0].body = b64(&forged.encode());
    s.store
        .append_event(
            hy_core::schema::NewEvent::manual(hy_core::schema::EventPayload::WaterLogged {
                amount_ml: 250,
            })
            .at(Utc.with_ymd_and_hms(2026, 10, 23, 5, 0, 0).unwrap()),
        )
        .unwrap();
    assert_mismatch_changes_nothing(&mut s, &state);
    assert!(!s.store.sharing_policy(Category::Hydration).unwrap().enabled);
    assert!(
        s.store.server_status().unwrap().mismatch,
        "persisted for the UI"
    );
    assert_eq!(s.store.queue_share_records(today).unwrap(), 0);
    assert!(s.store.outbox().unwrap().is_empty());
    // The genuine extension then verifies and clears the mismatch.
    let state = s.server.state(2, vec![device(s.me, "PC-1")]);
    assert!(matches!(
        s.store.apply_confirmed_state(&state).unwrap(),
        Some(StateOutcome::Applied { .. })
    ));
    assert!(s.store.sharing_policy(Category::Hydration).unwrap().enabled);
    assert!(!s.store.server_status().unwrap().mismatch);
}
