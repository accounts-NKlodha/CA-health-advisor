//! JSON bodies of the `/v1` API (contract 8.1) and the wire error enum (appendix A).
//!
//! Every body refuses unknown fields. The user id is never part of a request: the server
//! takes it from the token. Errors are one small enum with no free text.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::schema::Category;

/// The wire error enum (contract appendix A). `slip_malformed` is client-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireError {
    /// Unknown, used, voided or expired slip; one answer for all, timing equalised.
    SlipInvalid,
    /// The slip text is not a slip (client only).
    SlipMalformed,
    /// Too many attempts; see `Retry-After`.
    RateLimited,
    /// The origin device is paired to another user.
    OriginConflict,
    /// No token, or a token the server does not know.
    UnknownToken,
    /// The token's device was revoked.
    DeviceRevoked,
    /// The token's device is pending administrator confirmation.
    DeviceUnconfirmed,
    /// The device presented an older receipt than before (restored or cloned PC).
    DeviceRegressed,
    /// An enable based on a policy older than the latest disable or deletion.
    StaleEnable,
    /// The user is in recovery hold.
    RecoveryHold,
    /// A disable or deletion accepted during hold, sequenced on release.
    AcceptedPendingRecovery,
    /// Upload row: the category is off.
    CategoryOff,
    /// Upload row: before `share_from` or before this PC's confirmation date.
    BeforeShareFrom,
    /// Upload row: inside a tombstone.
    Tombstoned,
    /// Upload row: date outside the accepted window.
    DateOutOfRange,
    /// The body does not match its schema.
    SchemaInvalid,
    /// Upload row: the revision is not higher than the stored one.
    RevisionNotHigher,
    /// Upload row: origin is not the token's device.
    OriginMismatch,
    /// The control log cannot be recorded (mirror unavailable or disk full).
    CannotRecord,
    /// The body is larger than the limit.
    PayloadTooLarge,
    /// The protocol version or endpoint is not supported.
    ProtocolUnsupported,
}

impl WireError {
    /// The snake-case name, as sent.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SlipInvalid => "slip_invalid",
            Self::SlipMalformed => "slip_malformed",
            Self::RateLimited => "rate_limited",
            Self::OriginConflict => "origin_conflict",
            Self::UnknownToken => "unknown_token",
            Self::DeviceRevoked => "device_revoked",
            Self::DeviceUnconfirmed => "device_unconfirmed",
            Self::DeviceRegressed => "device_regressed",
            Self::StaleEnable => "stale_enable",
            Self::RecoveryHold => "recovery_hold",
            Self::AcceptedPendingRecovery => "accepted_pending_recovery",
            Self::CategoryOff => "category_off",
            Self::BeforeShareFrom => "before_share_from",
            Self::Tombstoned => "tombstoned",
            Self::DateOutOfRange => "date_out_of_range",
            Self::SchemaInvalid => "schema_invalid",
            Self::RevisionNotHigher => "revision_not_higher",
            Self::OriginMismatch => "origin_mismatch",
            Self::CannotRecord => "cannot_record",
            Self::PayloadTooLarge => "payload_too_large",
            Self::ProtocolUnsupported => "protocol_unsupported",
        }
    }
}

impl std::fmt::Display for WireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Every error response body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ErrorBody {
    /// The error.
    pub error: WireError,
}

/// Maximum length of a device label.
pub const MAX_LABEL: usize = 40;
/// Maximum length of an app version or Windows build string.
pub const MAX_VERSION: usize = 32;

/// `POST /v1/pair` request (contract 4.2 step 5).
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairRequest {
    /// [`super::PROTOCOL`].
    pub protocol: u32,
    /// The slip secret, Crockford base32 (16 characters). Sent once, inside the pinned
    /// channel; never logged.
    pub secret: String,
    /// The PC's origin-device id.
    pub origin_device: Uuid,
    /// A label the user recognises (for example the computer name), at most 40 characters.
    pub device_label: String,
    /// The companion version.
    pub app_version: String,
    /// The Windows build.
    pub windows_build: String,
}

impl std::fmt::Debug for PairRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PairRequest")
            .field("protocol", &self.protocol)
            .field("secret", &crate::secret::REDACTED)
            .field("origin_device", &self.origin_device)
            .finish_non_exhaustive()
    }
}

/// Pairing state of a device as the server reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceStatus {
    /// Waiting for the administrator.
    Pending,
    /// Confirmed by the administrator.
    Confirmed,
}

/// `POST /v1/pair` response (contract 4.2 step 6). It holds none of the user's state.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairResponse {
    /// The user, as a random id.
    pub user_id: Uuid,
    /// This device's reference.
    pub device_ref: Uuid,
    /// The user's display alias.
    pub alias: String,
    /// The device token, base64url. Returned once; never logged.
    pub token: String,
    /// The Ed25519 head-signing public key, base64url.
    pub head_key: String,
    /// The server's fingerprint, base64url; must equal the pin.
    pub server_fingerprint: String,
    /// Always `pending`.
    pub status: DeviceStatus,
}

impl std::fmt::Debug for PairResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PairResponse")
            .field("user_id", &self.user_id)
            .field("device_ref", &self.device_ref)
            .field("token", &crate::secret::REDACTED)
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

/// One of the user's devices, as shown in "PCs paired to you" (contract 4.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceInfo {
    /// The device reference.
    pub device_ref: Uuid,
    /// Its label.
    pub label: String,
    /// When it paired (ms).
    pub paired_at_ms: i64,
    /// When it was last seen (ms), if ever.
    pub last_seen_ms: Option<i64>,
    /// Confirmed by the administrator.
    pub confirmed: bool,
}

/// One control entry on the wire: the fixed binary body, base64url.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireEntry {
    /// The entry's seq.
    pub seq: u64,
    /// Server wall time (ms).
    pub at_ms: u64,
    /// The kind byte.
    pub kind: u8,
    /// The body bytes, base64url.
    pub body: String,
}

/// `GET /v1/state` response.
///
/// A pending device receives only `{status: "pending", alias}` (contract 4.5). A confirmed
/// device also receives its chain entries after `after_seq`, the signed head and the
/// device list and whether the user is in recovery hold. Policy changes and tombstones
/// reach devices only as chain entries, so a device applies them only after the head
/// and the extension verify (7.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum StateResponse {
    /// Pending: status and alias only.
    Pending {
        /// The user's alias.
        alias: String,
    },
    /// Confirmed.
    Confirmed {
        /// The user's alias.
        alias: String,
        /// The device's confirmation date in the user's zone (`YYYY-MM-DD`).
        confirmed_on: String,
        /// The user's devices.
        devices: Vec<DeviceInfo>,
        /// Chain entries with seq greater than `after_seq`, oldest first, at most 500.
        entries: Vec<WireEntry>,
        /// True when more entries follow (ask again with the last seq).
        more: bool,
        /// The user's current signed head (absent during recovery hold).
        head: Option<String>,
        /// The user is in recovery hold: sharing is paused (contract 9).
        hold: bool,
    },
}

/// Most operations in one `POST /v1/control`.
pub const MAX_CONTROL_OPS: usize = 50;
/// Most rows in one `POST /v1/upload`.
pub const MAX_UPLOAD_ROWS: usize = 100;
/// Longest date range `GET /v1/history` serves, in days.
pub const MAX_HISTORY_DAYS: i64 = 90;

/// One control operation (contract 8.1 and 8.3), idempotent by its id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ControlOp {
    /// Switch a category on. Accepted only if `base_seq` is at least the seq of the
    /// category's latest disable or deletion.
    Enable {
        /// Idempotency key.
        op_id: Uuid,
        /// The category.
        category: Category,
        /// The device's latest known policy seq for the category.
        base_seq: u64,
    },
    /// Switch a category off. Always accepted.
    Disable {
        /// Idempotency key.
        op_id: Uuid,
        /// The category.
        category: Category,
    },
    /// Delete shared data for a date range (both ends included). Always accepted.
    Delete {
        /// Idempotency key.
        request_id: Uuid,
        /// The category.
        category: Category,
        /// First date, `YYYY-MM-DD`.
        from: String,
        /// Last date, `YYYY-MM-DD`.
        to: String,
    },
}

impl ControlOp {
    /// The operation's idempotency key.
    #[must_use]
    pub fn id(&self) -> Uuid {
        match self {
            Self::Enable { op_id, .. } | Self::Disable { op_id, .. } => *op_id,
            Self::Delete { request_id, .. } => *request_id,
        }
    }
}

/// `POST /v1/control` request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlRequest {
    /// At most [`MAX_CONTROL_OPS`] operations, applied in order.
    pub ops: Vec<ControlOp>,
}

/// What happened to one control operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum ControlOutcome {
    /// Recorded at this seq (the same seq again for a repeated id).
    Accepted {
        /// The entry's seq.
        seq: u64,
    },
    /// The enable was older than the latest disable or deletion.
    StaleEnable {
        /// The seq of that disable or deletion.
        latest_seq: u64,
    },
    /// Recovery hold: applied, and sequenced on release (no seq yet).
    AcceptedPendingRecovery,
    /// Refused.
    Rejected {
        /// Why.
        error: WireError,
    },
}

/// One result of `POST /v1/control`. (`deny_unknown_fields` cannot be combined with a
/// flattened field; the outcome's own enum still refuses unknown tags.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlResult {
    /// The operation's id.
    pub id: Uuid,
    /// The outcome.
    #[serde(flatten)]
    pub outcome: ControlOutcome,
}

/// `POST /v1/control` response: per-operation results and the new head (absent during
/// hold). The entries themselves arrive with the next `GET /v1/state`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlResponse {
    /// One per operation, in order.
    pub results: Vec<ControlResult>,
    /// The user's head after these operations.
    pub head: Option<String>,
}

/// One summary row to upload (contract 8.6).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UploadRow {
    /// The category.
    pub category: Category,
    /// The PC that recorded it; must be the token's own device.
    pub origin_device: Uuid,
    /// `YYYY-MM-DD`.
    pub local_date: String,
    /// Higher than any revision stored for this row.
    pub revision: u32,
    /// The category's schema ([`super::share`]).
    pub payload: serde_json::Value,
}

/// `POST /v1/upload` request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UploadRequest {
    /// At most [`MAX_UPLOAD_ROWS`] rows.
    pub rows: Vec<UploadRow>,
}

/// What happened to one upload row. A rejected row is discarded by the device, never
/// re-stamped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum RowOutcome {
    /// Stored.
    Accepted,
    /// Refused.
    Rejected {
        /// Why.
        error: WireError,
    },
}

/// `POST /v1/upload` response: one outcome per row, in order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UploadResponse {
    /// The outcomes.
    pub results: Vec<RowOutcome>,
}

/// One row of `GET /v1/history` ("Restore from server"): read-only, keyed by origin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryRow {
    /// The category.
    pub category: Category,
    /// The PC that recorded it.
    pub origin_device: Uuid,
    /// `YYYY-MM-DD`.
    pub local_date: String,
    /// Its revision.
    pub revision: u32,
    /// The payload.
    pub payload: serde_json::Value,
}

/// `GET /v1/history` response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryResponse {
    /// The rows, by date.
    pub rows: Vec<HistoryRow>,
}

/// `POST /v1/receipt` request: this device's stored receipt (contract 9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReceiptRequest {
    /// The signed head, wire form.
    pub head: String,
}

/// `POST /v1/receipt` response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReceiptResponse {
    /// Counted as this device's evidence.
    pub recorded: bool,
}

/// The header every authenticated request from a confirmed device carries: its stored
/// receipt (contract 7.4, "Echo").
pub const RECEIPT_HEADER: &str = "hy-receipt";

/// Most chain entries in one state response (contract 7.4).
pub const ENTRY_PAGE: usize = 500;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_use_the_appendix_names() {
        let all = [
            WireError::SlipInvalid,
            WireError::SlipMalformed,
            WireError::RateLimited,
            WireError::OriginConflict,
            WireError::UnknownToken,
            WireError::DeviceRevoked,
            WireError::DeviceUnconfirmed,
            WireError::DeviceRegressed,
            WireError::StaleEnable,
            WireError::RecoveryHold,
            WireError::AcceptedPendingRecovery,
            WireError::CategoryOff,
            WireError::BeforeShareFrom,
            WireError::Tombstoned,
            WireError::DateOutOfRange,
            WireError::SchemaInvalid,
            WireError::RevisionNotHigher,
            WireError::OriginMismatch,
            WireError::CannotRecord,
            WireError::PayloadTooLarge,
            WireError::ProtocolUnsupported,
        ];
        assert_eq!(all.len(), 21);
        for e in all {
            let json = serde_json::to_string(&ErrorBody { error: e }).unwrap();
            assert_eq!(json, format!("{{\"error\":\"{}\"}}", e.as_str()));
            let back: ErrorBody = serde_json::from_str(&json).unwrap();
            assert_eq!(back.error, e);
        }
    }

    #[test]
    fn a_pending_state_carries_status_and_alias_only() {
        let json = serde_json::to_value(StateResponse::Pending { alias: "A".into() }).unwrap();
        assert_eq!(json, serde_json::json!({"status": "pending", "alias": "A"}));
    }

    #[test]
    fn unknown_fields_are_refused_and_secrets_are_not_debug_printed() {
        let body = serde_json::json!({
            "protocol": 1, "secret": "CANARYSECRET0000", "origin_device": Uuid::nil(),
            "device_label": "pc", "app_version": "0.1.0", "windows_build": "26100",
            "user_id": Uuid::nil()
        });
        assert!(serde_json::from_value::<PairRequest>(body).is_err());
        let ok: PairRequest = serde_json::from_value(serde_json::json!({
            "protocol": 1, "secret": "CANARYSECRET0000", "origin_device": Uuid::nil(),
            "device_label": "pc", "app_version": "0.1.0", "windows_build": "26100"
        }))
        .unwrap();
        assert!(!format!("{ok:?}").contains("CANARY"));
    }
}
