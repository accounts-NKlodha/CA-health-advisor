//! T-S25: a `GET /v1/state` response as the companion reads it (`hy_core::store::link`):
//! the JSON, each entry's base64url body through the body decoder, the signed head under
//! the fixed key, and the chain from the genesis to the head. No input may panic.
#![no_main]

use hy_core::protocol::chain::{self, ControlEntry, EntryBody};
use hy_core::protocol::head::SignedHead;
use hy_core::protocol::wire::StateResponse;
use hy_core::protocol::{key_id, unb64};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(StateResponse::Confirmed { entries, head, .. }) =
        serde_json::from_slice::<StateResponse>(data)
    else {
        return;
    };
    let Some(Ok(head)) = head.as_deref().map(SignedHead::from_wire) else {
        return;
    };
    let pk = hy_fuzz::head_public();
    let user = head.head.user_id;
    if head.verify(&pk, &user).is_err() {
        return;
    }
    let mut decoded = Vec::with_capacity(entries.len());
    for e in &entries {
        let Some(body) = unb64(&e.body, 256) else {
            return;
        };
        let Ok(body) = EntryBody::decode(e.kind, &body) else {
            return;
        };
        decoded.push(ControlEntry {
            user_id: user,
            seq: e.seq,
            at_ms: e.at_ms,
            body,
        });
    }
    let g = chain::genesis(&key_id(&pk), &user);
    if chain::verify_extension(&user, 0, g, &decoded, head.head.seq, &head.head.chain_hash).is_ok()
    {
        assert_eq!(
            head.head.seq,
            decoded.len() as u64,
            "a full chain from seq 1"
        );
    }
});
