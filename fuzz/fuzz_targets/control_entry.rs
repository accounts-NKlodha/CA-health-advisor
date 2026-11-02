//! T-S25: the control-entry body decoder. Input: the kind byte, then the body. No input
//! may panic; a body that decodes is exact (it encodes back to the same bytes, same kind).
#![no_main]

use hy_core::protocol::chain::{ControlEntry, EntryBody};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some((&kind, body)) = data.split_first() else {
        return;
    };
    let Ok(decoded) = EntryBody::decode(kind, body) else {
        return;
    };
    assert_eq!(decoded.kind(), kind, "the kind is kept");
    assert_eq!(decoded.encode(), body, "a decoded body is canonical");
    let entry = ControlEntry {
        user_id: [1; 16],
        seq: 1,
        at_ms: 0,
        body: decoded,
    };
    let _ = entry.hash(&[0; 32]);
});
