//! The seed corpora of the T-S25 fuzz targets (`fuzz/`, tracked work W4), built from the
//! protocol's test vectors: the slips, control-entry bodies, chains, heads and upload
//! payloads the unit tests use. Every seed is deterministic (the head key comes from a
//! fixed seed), so this test checks that each committed `fuzz/corpus/<target>/seed-*`
//! file is exactly what the vectors give today and that each valid seed is still accepted
//! by the decoder it feeds: a format change cannot leave the fuzzer starting from inputs
//! the code now refuses at the first byte.
//!
//! To rewrite the files after a deliberate format change:
//! `HY_WRITE_FUZZ_SEEDS=1 cargo test -p hy-core --test fuzz_seeds`.

// Test helpers outside `#[test]` functions may unwrap too.
#![allow(clippy::unwrap_used)]

use std::path::PathBuf;

use aws_lc_rs::signature::Ed25519KeyPair;
use hy_core::protocol::chain::{self, ControlEntry, EntryBody, RevocationReason};
use hy_core::protocol::head::{self, Head, SignedHead};
use hy_core::protocol::slip::Slip;
use hy_core::protocol::wire::{ReceiptRequest, StateResponse, UploadRequest, WireEntry};
use hy_core::protocol::{SLIP_SECRET_LEN, b64, key_id, share};
use hy_core::schema::Category;
use serde_json::json;

/// Must equal `hy_fuzz::HEAD_KEY_SEED` (fuzz/src/lib.rs).
const HEAD_KEY_SEED: [u8; 32] = [0x42; 32];
const USER: [u8; 16] = [9; 16];

fn head_key() -> Ed25519KeyPair {
    Ed25519KeyPair::from_seed_unchecked(&HEAD_KEY_SEED).unwrap()
}

fn bodies() -> Vec<EntryBody> {
    vec![
        EntryBody::Pairing {
            device_ref: [1; 16],
            origin_device: [2; 16],
            token_verifier: [3; 32],
        },
        EntryBody::DeviceConfirmed {
            device_ref: [1; 16],
        },
        EntryBody::Policy {
            category: Category::Hydration,
            enabled: true,
            share_from: 20_384,
            op_id: [4; 16],
            device_ref: [1; 16],
        },
        EntryBody::Policy {
            category: Category::Hydration,
            enabled: false,
            share_from: 0,
            op_id: [5; 16],
            device_ref: [1; 16],
        },
        EntryBody::Deletion {
            category: Category::PhoneReports,
            from: 20_000,
            to: 20_010,
            request_id: [6; 16],
            device_ref: [1; 16],
        },
        EntryBody::HistoryAccess {
            device_ref: [1; 16],
        },
        EntryBody::Revocation {
            device_ref: [1; 16],
            reason: RevocationReason::SelfUnpair,
        },
        EntryBody::RecoveryDiscard {
            snapshot_id: [7; 16],
        },
    ]
}

fn entries() -> Vec<ControlEntry> {
    bodies()
        .into_iter()
        .zip(1u64..)
        .map(|(body, seq)| ControlEntry {
            user_id: USER,
            seq,
            at_ms: 1_700_000_000_000 + seq,
            body,
        })
        .collect()
}

fn signed_head(seq: u64, chain_hash: [u8; 32]) -> SignedHead {
    let k = head_key();
    Head {
        key_id: key_id(&head::public_key(&k)),
        user_id: USER,
        seq,
        chain_hash,
        issued_at_ms: 1_761_000_000_000,
    }
    .sign(&k)
}

fn genesis() -> [u8; 32] {
    chain::genesis(&key_id(&head::public_key(&head_key())), &USER)
}

/// The `chain_verify` input format (`hy_fuzz::parse_chain`).
fn chain_input(
    start_seq: u64,
    start_hash: [u8; 32],
    end_seq: u64,
    end_hash: [u8; 32],
    run: &[ControlEntry],
) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&USER);
    out.extend_from_slice(&start_seq.to_be_bytes());
    out.extend_from_slice(&start_hash);
    out.extend_from_slice(&end_seq.to_be_bytes());
    out.extend_from_slice(&end_hash);
    for e in run {
        let body = e.body.encode();
        out.extend_from_slice(&e.seq.to_be_bytes());
        out.extend_from_slice(&e.at_ms.to_be_bytes());
        out.push(e.body.kind());
        out.push(u8::try_from(body.len()).unwrap());
        out.extend_from_slice(&body);
    }
    out
}

fn seeds() -> Vec<(&'static str, &'static str, Vec<u8>)> {
    let mut out = Vec::new();

    // Slips: the unit tests' sample (canonical, grouped, lower case, an IP host).
    let mut fp = [0u8; 32];
    for (i, b) in fp.iter_mut().enumerate() {
        *b = u8::try_from(i * 7 % 256).unwrap();
    }
    let s = Slip::new("office-pc-01", 8443, fp, [0xA5; SLIP_SECRET_LEN]).unwrap();
    let canonical = s.canonical().expose().clone();
    out.push(("slip", "canonical", canonical.clone().into_bytes()));
    out.push((
        "slip",
        "printable",
        s.printable().expose().clone().into_bytes(),
    ));
    out.push(("slip", "lower", canonical.to_lowercase().into_bytes()));
    let ip = Slip::new("192.168.50.10", 8443, [0x11; 32], [0x5A; SLIP_SECRET_LEN]).unwrap();
    out.push((
        "slip",
        "ip-host",
        ip.canonical().expose().clone().into_bytes(),
    ));
    let nb = Slip::new("OFFICE_PC", 65535, [0xFE; 32], [0; SLIP_SECRET_LEN]).unwrap();
    out.push((
        "slip",
        "netbios",
        nb.canonical().expose().clone().into_bytes(),
    ));

    // Control-entry bodies: kind byte, then the body, one per kind.
    for (i, b) in bodies().iter().enumerate() {
        let mut v = vec![b.kind()];
        v.extend_from_slice(&b.encode());
        out.push(("control_entry", NAMES[i], v));
    }

    // Chains: the full chain from the genesis, a suffix from a receipt, and a refused
    // run (wrong end hash).
    let all = entries();
    let g = genesis();
    let (end, tail) = chain::extend(&USER, 0, g, &all).unwrap();
    out.push(("chain_verify", "full", chain_input(0, g, end, tail, &all)));
    let (s3, h3) = chain::extend(&USER, 0, g, &all[..3]).unwrap();
    out.push((
        "chain_verify",
        "suffix",
        chain_input(s3, h3, end, tail, &all[3..]),
    ));
    let mut wrong = tail;
    wrong[0] ^= 1;
    out.push((
        "chain_verify",
        "mismatch",
        chain_input(0, g, end, wrong, &all),
    ));

    // Heads: a receipt body and the bare wire text, signed with the fixed key.
    let h = signed_head(end, tail);
    let body = serde_json::to_vec(&ReceiptRequest { head: h.to_wire() }).unwrap();
    out.push(("head_receipt", "receipt-request", body));
    out.push(("head_receipt", "wire", h.to_wire().into_bytes()));

    // A confirmed state response carrying the whole chain and its head.
    let state = StateResponse::Confirmed {
        alias: "desk-7".into(),
        confirmed_on: "2025-10-21".into(),
        devices: Vec::new(),
        entries: all
            .iter()
            .map(|e| WireEntry {
                seq: e.seq,
                at_ms: e.at_ms,
                kind: e.body.kind(),
                body: b64(&e.body.encode()),
            })
            .collect(),
        more: false,
        head: Some(h.to_wire()),
        hold: false,
    };
    out.push((
        "state_json",
        "confirmed",
        serde_json::to_vec(&state).unwrap(),
    ));
    let pending = StateResponse::Pending {
        alias: "desk-7".into(),
    };
    out.push((
        "state_json",
        "pending",
        serde_json::to_vec(&pending).unwrap(),
    ));

    // An upload with one valid row per category (the share module's vectors).
    let rows = json!({"rows": [
        {"category": "hydration", "origin_device": "0190f0c4-7a2b-7c3d-8e4f-5a6b7c8d9e0f", "local_date": "2025-10-21", "revision": 1,
         "payload": {"v": 1, "ml": 1500, "goal_ml": 2000, "entries": 6, "coverage": 0.8}},
        {"category": "outcomes", "origin_device": "0190f0c4-7a2b-7c3d-8e4f-5a6b7c8d9e0f", "local_date": "2025-10-21", "revision": 2,
         "payload": {"v": 1, "offered": 5, "done": 3, "snoozed": 1, "skipped": 0, "no_response": 1, "routine": "partial"}},
        {"category": "work_session", "origin_device": "0190f0c4-7a2b-7c3d-8e4f-5a6b7c8d9e0f", "local_date": "2025-10-21", "revision": 1,
         "payload": {"v": 1, "active_min": 300, "longest_block_min": 70, "focus_count": 2, "pomodoro_count": 4}},
        {"category": "phone_reports", "origin_device": "0190f0c4-7a2b-7c3d-8e4f-5a6b7c8d9e0f", "local_date": "2025-10-21", "revision": 1,
         "payload": {"v": 1, "work": 2, "personal": 1, "unsure": 0}},
        {"category": "posture_summary", "origin_device": "0190f0c4-7a2b-7c3d-8e4f-5a6b7c8d9e0f", "local_date": "2025-10-21", "revision": 1,
         "payload": {"v": 1, "camera_on_min": 120, "usable_coverage": 0.9, "episodes": 3}}
    ]});
    out.push((
        "upload_json",
        "all-categories",
        serde_json::to_vec(&rows).unwrap(),
    ));
    let refused = json!({"rows": [
        {"category": "hydration", "origin_device": "0190f0c4-7a2b-7c3d-8e4f-5a6b7c8d9e0f", "local_date": "2025-10-21", "revision": 1,
         "payload": {"v": 1, "ml": 1500, "goal_ml": 2000, "entries": 6, "coverage": 0.8, "note": "x"}}
    ]});
    out.push((
        "upload_json",
        "unknown-field",
        serde_json::to_vec(&refused).unwrap(),
    ));
    out
}

const NAMES: [&str; 8] = [
    "pairing",
    "device-confirmed",
    "policy-on",
    "policy-off",
    "deletion",
    "history-access",
    "revocation",
    "recovery-discard",
];

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/corpus")
}

#[test]
fn the_committed_fuzz_seeds_are_the_protocol_vectors() {
    let write = std::env::var_os("HY_WRITE_FUZZ_SEEDS").is_some();
    for (target, name, bytes) in seeds() {
        let path = corpus().join(target).join(format!("seed-{name}"));
        if write {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &bytes).unwrap();
        }
        let committed = std::fs::read(&path).unwrap_or_else(|e| {
            panic!(
                "{}: {e}; run HY_WRITE_FUZZ_SEEDS=1 cargo test -p hy-core --test fuzz_seeds",
                path.display()
            )
        });
        assert_eq!(
            committed,
            bytes,
            "{} differs from the vectors; rewrite with HY_WRITE_FUZZ_SEEDS=1",
            path.display()
        );
    }
}

#[test]
fn the_valid_seeds_are_accepted_by_their_decoders() {
    let pk = head::public_key(&head_key());
    for (target, name, bytes) in seeds() {
        match (target, name) {
            ("slip", _) => {
                Slip::parse(std::str::from_utf8(&bytes).unwrap()).unwrap();
            }
            ("control_entry", _) => {
                EntryBody::decode(bytes[0], &bytes[1..]).unwrap();
            }
            ("head_receipt", "receipt-request") => {
                let r: ReceiptRequest = serde_json::from_slice(&bytes).unwrap();
                SignedHead::from_wire(&r.head)
                    .unwrap()
                    .verify(&pk, &USER)
                    .unwrap();
            }
            ("head_receipt", _) => {
                let h = SignedHead::from_wire(std::str::from_utf8(&bytes).unwrap()).unwrap();
                h.verify(&pk, &USER).unwrap();
            }
            ("state_json", _) => {
                serde_json::from_slice::<StateResponse>(&bytes).unwrap();
            }
            ("upload_json", n) => {
                let req: UploadRequest = serde_json::from_slice(&bytes).unwrap();
                let valid = req
                    .rows
                    .iter()
                    .all(|r| share::validate(r.category, &r.payload).is_ok());
                assert_eq!(valid, n == "all-categories", "{n}");
            }
            _ => {}
        }
    }
    // The full chain verifies from the genesis to the signed head's position.
    let all = entries();
    let (end, tail) = chain::extend(&USER, 0, genesis(), &all).unwrap();
    chain::verify_extension(&USER, 0, genesis(), &all, end, &tail).unwrap();
}
