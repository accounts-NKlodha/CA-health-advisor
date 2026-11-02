//! T-S25: the signed head and receipt decoder: a `POST /v1/receipt` body
//! (`{"head": "<wire>"}`) or the bare wire text the `hy-receipt` header and the state and
//! control responses carry. No input may panic; a head that decodes has exactly one wire
//! form, and verification under the fixed key never panics and accepts a head only for
//! its own user and its own signature.
#![no_main]

use hy_core::protocol::head::{HeadError, SignedHead};
use hy_core::protocol::wire::{ReceiptRequest, ReceiptResponse};
use libfuzzer_sys::fuzz_target;

fn check(text: &str) {
    let Ok(h) = SignedHead::from_wire(text) else {
        return;
    };
    assert_eq!(h.to_wire(), text, "one wire form per head");
    assert_eq!(SignedHead::from_wire(&h.to_wire()), Ok(h));
    let pk = hy_fuzz::head_public();
    if h.verify(&pk, &h.head.user_id).is_ok() {
        let mut other = h.head.user_id;
        other[0] ^= 1;
        assert_eq!(h.verify(&pk, &other), Err(HeadError::WrongUser));
        let mut bad = h;
        bad.sig[0] ^= 1;
        assert!(bad.verify(&pk, &h.head.user_id).is_err());
    }
}

fuzz_target!(|data: &[u8]| {
    let _ = serde_json::from_slice::<ReceiptResponse>(data);
    if let Ok(req) = serde_json::from_slice::<ReceiptRequest>(data) {
        check(&req.head);
    } else if let Ok(text) = std::str::from_utf8(data) {
        check(text);
    }
});
