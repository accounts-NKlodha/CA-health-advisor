//! T-S25: the `POST /v1/upload` body as the server reads it (`hy_server::api::read_json`
//! then `hy_server::sync::check_row`): the JSON, then each row's date and its category's
//! payload schema. No input may panic; a payload that validates comes back canonical, the
//! canonical form validates to itself, and its stored text validates again.
#![no_main]

use hy_core::protocol::share;
use hy_core::protocol::wire::{UploadRequest, UploadResponse};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = serde_json::from_slice::<UploadResponse>(data);
    let Ok(req) = serde_json::from_slice::<UploadRequest>(data) else {
        return;
    };
    for row in &req.rows {
        if row.local_date.len() == 10 {
            let _ = chrono::NaiveDate::parse_from_str(&row.local_date, "%Y-%m-%d");
        }
        if let Ok(canonical) = share::validate(row.category, &row.payload) {
            assert_eq!(
                share::validate(row.category, &canonical).as_ref(),
                Ok(&canonical),
                "the canonical payload validates to itself"
            );
            let text = canonical.to_string();
            let back: serde_json::Value =
                serde_json::from_str(&text).expect("canonical JSON reads back");
            assert!(
                share::validate(row.category, &back).is_ok(),
                "the stored text validates again"
            );
        }
    }
});
