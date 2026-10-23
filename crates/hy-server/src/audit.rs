//! The admin audit log (V1-5 contract 11): fixed event names, random ids and /24
//! network prefixes only. No habit data, aliases, labels, secrets or full addresses.
//! Kept 30 days.

use std::net::IpAddr;

use rusqlite::{Connection, params};

use crate::logging::net_prefix;

/// An audited event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditEvent {
    /// A user was created.
    UserCreated,
    /// A slip was issued.
    SlipIssued,
    /// A slip was redeemed (a pending device was created).
    SlipRedeemed,
    /// A pairing attempt was refused with `slip_invalid`.
    SlipRefused,
    /// A pairing attempt was rate-limited.
    PairRateLimited,
    /// A device was confirmed.
    DeviceConfirmed,
    /// A pending device was rejected.
    DeviceRejected,
    /// An active device was revoked by the administrator.
    DeviceRevoked,
    /// A device unpaired itself.
    DeviceUnpaired,
    /// A pending device expired after 7 days.
    DeviceExpired,
    /// A device was superseded by a new pairing of the same PC.
    DeviceSuperseded,
    /// The administrator signed in.
    AdminLogin,
    /// A sign-in failed.
    AdminLoginFailed,
    /// The administrator signed out.
    AdminLogout,
    /// Every pending device of a user was rejected.
    PendingRejectedForUser,
    /// A device echoed an older receipt than before (restored or cloned PC).
    DeviceRegressed,
    /// A device read the user's history.
    HistoryRead,
    /// A receipt counted as a device's recovery evidence.
    ReceiptRecorded,
    /// A receipt was refused (bad signature, other key or user, no usable token).
    ReceiptRefused,
    /// A user entered recovery hold.
    RecoveryHold,
    /// A user was released from hold.
    UserReleased,
    /// A user's restored history was discarded.
    UserDiscarded,
    /// The global discard path ran.
    GlobalDiscard,
    /// The administrator confirmed the list of users during a hold.
    UsersWitnessed,
    /// An encrypted backup was written.
    BackupWritten,
    /// The database was restored from a backup or a rollback was detected.
    Restored,
}

impl AuditEvent {
    /// The stored name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UserCreated => "user_created",
            Self::SlipIssued => "slip_issued",
            Self::SlipRedeemed => "slip_redeemed",
            Self::SlipRefused => "slip_refused",
            Self::PairRateLimited => "pair_rate_limited",
            Self::DeviceConfirmed => "device_confirmed",
            Self::DeviceRejected => "device_rejected",
            Self::DeviceRevoked => "device_revoked",
            Self::DeviceUnpaired => "device_unpaired",
            Self::DeviceExpired => "device_expired",
            Self::DeviceSuperseded => "device_superseded",
            Self::AdminLogin => "admin_login",
            Self::AdminLoginFailed => "admin_login_failed",
            Self::AdminLogout => "admin_logout",
            Self::PendingRejectedForUser => "pending_rejected_for_user",
            Self::DeviceRegressed => "device_regressed",
            Self::HistoryRead => "history_read",
            Self::ReceiptRecorded => "receipt_recorded",
            Self::ReceiptRefused => "receipt_refused",
            Self::RecoveryHold => "recovery_hold",
            Self::UserReleased => "user_released",
            Self::UserDiscarded => "user_discarded",
            Self::GlobalDiscard => "global_discard",
            Self::UsersWitnessed => "users_witnessed",
            Self::BackupWritten => "backup_written",
            Self::Restored => "restored",
        }
    }
}

/// Record one event.
///
/// # Errors
/// Any SQLite error.
pub fn record(
    conn: &Connection,
    at: i64,
    event: AuditEvent,
    user: Option<&[u8; 16]>,
    device: Option<&[u8; 16]>,
    ip: Option<IpAddr>,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO audit (at, event, user_id, device_ref, net24) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            at,
            event.as_str(),
            user.map(|u| u.to_vec()),
            device.map(|d| d.to_vec()),
            ip.map(net_prefix)
        ],
    )?;
    Ok(())
}

/// One audit row for the admin page.
#[derive(Debug, Clone)]
pub struct AuditRow {
    /// When.
    pub at: i64,
    /// What.
    pub event: String,
    /// User id, if any.
    pub user_id: Option<Vec<u8>>,
    /// Device, if any.
    pub device_ref: Option<Vec<u8>>,
    /// Source network, if any.
    pub net24: Option<String>,
}

/// The newest `limit` entries.
///
/// # Errors
/// Any SQLite error.
pub fn recent(conn: &Connection, limit: usize) -> rusqlite::Result<Vec<AuditRow>> {
    let mut stmt = conn.prepare(
        "SELECT at, event, user_id, device_ref, net24 FROM audit ORDER BY id DESC LIMIT ?1",
    )?;
    stmt.query_map([i64::try_from(limit).unwrap_or(100)], |r| {
        Ok(AuditRow {
            at: r.get(0)?,
            event: r.get(1)?,
            user_id: r.get(2)?,
            device_ref: r.get(3)?,
            net24: r.get(4)?,
        })
    })?
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_event_name_is_allowed_by_the_schema() {
        let conn = crate::db::open_in_memory().unwrap();
        let all = [
            AuditEvent::UserCreated,
            AuditEvent::SlipIssued,
            AuditEvent::SlipRedeemed,
            AuditEvent::SlipRefused,
            AuditEvent::PairRateLimited,
            AuditEvent::DeviceConfirmed,
            AuditEvent::DeviceRejected,
            AuditEvent::DeviceRevoked,
            AuditEvent::DeviceUnpaired,
            AuditEvent::DeviceExpired,
            AuditEvent::DeviceSuperseded,
            AuditEvent::AdminLogin,
            AuditEvent::AdminLoginFailed,
            AuditEvent::AdminLogout,
            AuditEvent::PendingRejectedForUser,
            AuditEvent::DeviceRegressed,
            AuditEvent::HistoryRead,
            AuditEvent::ReceiptRecorded,
            AuditEvent::ReceiptRefused,
            AuditEvent::RecoveryHold,
            AuditEvent::UserReleased,
            AuditEvent::UserDiscarded,
            AuditEvent::GlobalDiscard,
            AuditEvent::UsersWitnessed,
            AuditEvent::BackupWritten,
            AuditEvent::Restored,
        ];
        for e in all {
            record(&conn, 1, e, None, None, Some("10.1.2.3".parse().unwrap())).unwrap();
        }
        let rows = recent(&conn, 100).unwrap();
        assert_eq!(rows.len(), all.len());
        assert!(
            rows.iter()
                .all(|r| r.net24.as_deref() == Some("10.1.2.0/24"))
        );
    }
}
