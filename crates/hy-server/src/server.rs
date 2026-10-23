//! The server's state and its operations, independent of HTTP (V1-5 contract 4, 6.3).
//!
//! Every operation that changes a device or a user's shared state runs in one
//! `BEGIN IMMEDIATE` transaction together with its control-log entry (7.1). Every
//! authenticated request reads the device state from the database with no cache, so a
//! revocation takes effect on the next request (6.3).

use std::net::IpAddr;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use aws_lc_rs::rand::{SecureRandom, SystemRandom};
use chrono::{DateTime, Utc};
use hy_core::clock::Clock;
use hy_core::protocol::chain::{EntryBody, RevocationReason};
use hy_core::protocol::slip::Slip;
use hy_core::protocol::wire::{
    DeviceInfo, DeviceStatus, ENTRY_PAGE, MAX_LABEL, MAX_VERSION, PairRequest, PairResponse,
    StateResponse, WireError,
};
use hy_core::protocol::words::device_check_code;
use hy_core::protocol::{
    PROTOCOL, SLIP_SECRET_LEN, TOKEN_LEN, b64, ct_eq32, slip_hash, token_verifier,
};
use hy_core::secret::Secret;
use hy_core::time::UserZone;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use uuid::Uuid;

use crate::audit::{self, AuditEvent};
use crate::chain;
use crate::db::{h32, id16};
use crate::keys::ServerKeys;
use crate::logging::{self, V};

/// Server settings.
#[derive(Debug, Clone)]
pub struct Config {
    /// The computer name (or IP) written on slips.
    pub host_name: String,
    /// The LAN API port written on slips.
    pub api_port: u16,
    /// Every `/v1/pair` answer takes at least this long, so unknown, used and expired
    /// slips cannot be told apart by timing (4.4).
    pub pair_floor: Duration,
    /// Argon2id cost for the admin passphrase: memory (KiB), passes, lanes.
    pub argon2: (u32, u32, u32),
    /// Pairing attempts allowed per source IP per minute, per IP per hour, and in total
    /// per hour (4.4).
    pub pair_limits: (u32, u32, u32),
}

impl Default for Config {
    fn default() -> Self {
        Self {
            host_name: "localhost".into(),
            api_port: hy_core::protocol::DEFAULT_PORT,
            pair_floor: Duration::from_millis(400),
            // The parameters used by `.hybackup` (contract 6.1).
            argon2: (64 * 1024, 3, 1),
            pair_limits: (5, 30, 60),
        }
    }
}

/// How long a slip is valid.
pub const SLIP_VALIDITY_MS: i64 = 24 * 3600 * 1000;
/// At most this many outstanding slips per user.
pub const MAX_OUTSTANDING_SLIPS: i64 = 3;
/// A pending device is revoked automatically after this long.
pub const PENDING_EXPIRY_MS: i64 = 7 * 24 * 3600 * 1000;
/// Devices unseen for longer are flagged on the admin page.
pub const UNSEEN_FLAG_MS: i64 = 14 * 24 * 3600 * 1000;
/// Audit entries and used slips are kept this long.
pub const AUDIT_RETENTION_MS: i64 = 30 * 24 * 3600 * 1000;
/// Admin sessions end after this much idle time.
pub const SESSION_IDLE_MS: i64 = 30 * 60 * 1000;

/// A device's pairing state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceState {
    /// Waiting for the administrator ('unconfirmed').
    Pending,
    /// Confirmed.
    Active,
    /// Revoked; final.
    Revoked,
}

impl DeviceState {
    fn parse(s: &str) -> Self {
        match s {
            "active" => Self::Active,
            "unconfirmed" => Self::Pending,
            _ => Self::Revoked,
        }
    }
}

/// The device behind a valid token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthedDevice {
    /// The device.
    pub device_ref: [u8; 16],
    /// Its user (from the token, never from a request).
    pub user_id: [u8; 16],
    /// Its origin-device id.
    pub origin_device: [u8; 16],
    /// Pending or active (revoked tokens are refused before this exists).
    pub state: DeviceState,
}

/// Why an admin action was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdminError {
    /// No such user or device.
    NotFound,
    /// The alias is empty, too long, or already used.
    BadAlias,
    /// The user already has three outstanding slips.
    TooManySlips,
    /// The device is not pending.
    NotPending,
    /// The device is already revoked.
    AlreadyRevoked,
    /// The check code does not match the pending row.
    CheckCodeMismatch,
    /// The user is in recovery hold.
    Hold,
    /// The database failed.
    Storage,
}

impl std::fmt::Display for AdminError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotFound => "Not found.",
            Self::BadAlias => "The alias must be 1 to 40 characters and not already used.",
            Self::TooManySlips => "This user already has three outstanding slips.",
            Self::NotPending => "That PC is not waiting for confirmation.",
            Self::AlreadyRevoked => "That PC is already revoked.",
            Self::CheckCodeMismatch => {
                "The check code does not match this PC. Do not confirm it; ask the user to read it again."
            }
            Self::Hold => "This user's shared history is in recovery hold.",
            Self::Storage => "The server database failed. Nothing was changed.",
        })
    }
}

impl From<rusqlite::Error> for AdminError {
    fn from(_: rusqlite::Error) -> Self {
        Self::Storage
    }
}

/// An internal failure while answering a request (the caller answers 503).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// A wire error for the client.
    Wire(WireError),
    /// Rate-limited; retry after this many seconds.
    RateLimited(u64),
}

impl From<WireError> for Refusal {
    fn from(e: WireError) -> Self {
        Self::Wire(e)
    }
}

impl From<rusqlite::Error> for Refusal {
    fn from(_: rusqlite::Error) -> Self {
        Self::Wire(WireError::CannotRecord)
    }
}

/// One device row for the admin page.
#[derive(Debug, Clone)]
pub struct DeviceRow {
    /// The device.
    pub device_ref: [u8; 16],
    /// Its user.
    pub user_id: [u8; 16],
    /// Its label.
    pub label: String,
    /// Its state.
    pub state: DeviceState,
    /// The check code (shown for pending rows).
    pub check_code: String,
    /// When it paired.
    pub created_at: i64,
    /// When it was confirmed.
    pub confirmed_at: Option<i64>,
    /// When it was last seen.
    pub last_seen: Option<i64>,
    /// Why it was revoked.
    pub revoke_reason: Option<String>,
}

/// One user for the admin page.
#[derive(Debug, Clone)]
pub struct UserRow {
    /// The user.
    pub user_id: [u8; 16],
    /// The alias.
    pub alias: String,
    /// In recovery hold.
    pub hold: bool,
    /// Outstanding slips.
    pub outstanding_slips: i64,
}

/// The server.
pub struct Server {
    db: Mutex<Connection>,
    /// The server's keys.
    pub keys: ServerKeys,
    /// The clock (injected in tests).
    pub clock: Arc<dyn Clock>,
    /// Settings.
    pub config: Config,
    rng: SystemRandom,
}

impl std::fmt::Debug for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Server")
            .field("keys", &self.keys)
            .finish_non_exhaustive()
    }
}

fn ms(t: DateTime<Utc>) -> i64 {
    t.timestamp_millis()
}

fn label_ok(s: &str, max: usize) -> bool {
    let n = s.chars().count();
    n >= 1 && n <= max && !s.chars().any(char::is_control) && s.trim() == s
}

/// `device_ref, user_id, origin_device, token_verifier, state` of one device.
type DeviceAuthRow = (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>, String);

impl Server {
    /// A server over an opened, migrated database.
    #[must_use]
    pub fn new(conn: Connection, keys: ServerKeys, clock: Arc<dyn Clock>, config: Config) -> Self {
        Self {
            db: Mutex::new(conn),
            keys,
            clock,
            config,
            rng: SystemRandom::new(),
        }
    }

    /// The database connection (single writer).
    pub fn db(&self) -> MutexGuard<'_, Connection> {
        self.db.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn now_ms(&self) -> i64 {
        ms(self.clock.now())
    }

    fn random<const N: usize>(&self) -> [u8; N] {
        let mut out = [0u8; N];
        // aws-lc's system RNG does not fail on supported platforms; if it ever did, an
        // all-zero value would be refused by the unique constraints rather than reused.
        let _ = self.rng.fill(&mut out);
        out
    }

    fn user_zone(conn: &Connection, user: &[u8; 16]) -> rusqlite::Result<UserZone> {
        let zone: String = conn.query_row(
            "SELECT zone FROM users WHERE user_id = ?1",
            [user.as_slice()],
            |r| r.get(0),
        )?;
        Ok(UserZone::parse(&zone).unwrap_or_default())
    }

    // ------------------------------------------------------------------ admin

    /// Create a user with a display alias only.
    ///
    /// # Errors
    /// [`AdminError::BadAlias`] if the alias is empty, too long or taken.
    pub fn create_user(&self, alias: &str) -> Result<[u8; 16], AdminError> {
        let alias = alias.trim();
        if !label_ok(alias, MAX_LABEL) {
            return Err(AdminError::BadAlias);
        }
        let user: [u8; 16] = self.random();
        let now = self.now_ms();
        let conn = self.db();
        conn.execute(
            "INSERT INTO users (user_id, alias, created_at) VALUES (?1, ?2, ?3)",
            params![user.as_slice(), alias, now],
        )
        .map_err(|_| AdminError::BadAlias)?;
        audit::record(&conn, now, AuditEvent::UserCreated, Some(&user), None, None)?;
        Ok(user)
    }

    /// Issue a slip for a user (contract 4.2 step 1). The secret is shown once and only
    /// its hash is stored.
    ///
    /// # Errors
    /// [`AdminError::NotFound`] or [`AdminError::TooManySlips`].
    pub fn issue_slip(&self, user: &[u8; 16]) -> Result<Slip, AdminError> {
        let now = self.now_ms();
        let mut conn = self.db();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let exists: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM users WHERE user_id = ?1)",
            [user.as_slice()],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(AdminError::NotFound);
        }
        let outstanding: i64 = tx.query_row(
            "SELECT COUNT(*) FROM slips WHERE user_id = ?1 AND used_at IS NULL AND voided = 0 AND expires_at > ?2",
            params![user.as_slice(), now],
            |r| r.get(0),
        )?;
        if outstanding >= MAX_OUTSTANDING_SLIPS {
            return Err(AdminError::TooManySlips);
        }
        let secret: [u8; SLIP_SECRET_LEN] = self.random();
        tx.execute(
            "INSERT INTO slips (slip_hash, user_id, created_at, expires_at) VALUES (?1, ?2, ?3, ?4)",
            params![
                slip_hash(&secret).as_slice(),
                user.as_slice(),
                now,
                now + SLIP_VALIDITY_MS
            ],
        )?;
        audit::record(&tx, now, AuditEvent::SlipIssued, Some(user), None, None)?;
        tx.commit()?;
        Slip::new(
            &self.config.host_name,
            self.config.api_port,
            self.keys.fingerprint,
            secret,
        )
        .map_err(|_| AdminError::Storage)
    }

    fn revoke_in(
        &self,
        conn: &Connection,
        device: &[u8; 16],
        user: &[u8; 16],
        reason: RevocationReason,
        now: i64,
    ) -> rusqlite::Result<()> {
        conn.execute(
            "UPDATE devices SET state = 'revoked', revoked_at = ?2, revoke_reason = ?3 WHERE device_ref = ?1",
            params![device.as_slice(), now, reason.as_str()],
        )?;
        chain::append(
            conn,
            &self.keys.key_id,
            user,
            u64::try_from(now).unwrap_or(0),
            EntryBody::Revocation {
                device_ref: *device,
                reason,
            },
        )?;
        Ok(())
    }

    /// Confirm a pending device after the user read out the six words and this check
    /// code (contract 4.2 step 9). This is the **only** way a device becomes active.
    ///
    /// # Errors
    /// [`AdminError`] if the device is not pending, the code does not match, or the user
    /// is in recovery hold.
    pub fn confirm_device(&self, device: &[u8; 16], check_code: &str) -> Result<(), AdminError> {
        let now = self.now_ms();
        let mut conn = self.db();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row: Option<(Vec<u8>, String)> = tx
            .query_row(
                "SELECT user_id, state FROM devices WHERE device_ref = ?1",
                [device.as_slice()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let (user, state) = row.ok_or(AdminError::NotFound)?;
        let user = id16(user)?;
        if DeviceState::parse(&state) != DeviceState::Pending {
            return Err(AdminError::NotPending);
        }
        let typed: String = check_code
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '-')
            .map(|c| match c.to_ascii_uppercase() {
                'O' => '0',
                'I' | 'L' => '1',
                u => u,
            })
            .collect();
        if typed != device_check_code(device) {
            return Err(AdminError::CheckCodeMismatch);
        }
        let hold: bool = tx.query_row(
            "SELECT hold FROM users WHERE user_id = ?1",
            [user.as_slice()],
            |r| r.get(0),
        )?;
        if hold {
            return Err(AdminError::Hold);
        }
        let (entry, _) = chain::append(
            &tx,
            &self.keys.key_id,
            &user,
            u64::try_from(now).unwrap_or(0),
            EntryBody::DeviceConfirmed {
                device_ref: *device,
            },
        )?;
        let zone = Self::user_zone(&tx, &user)?;
        let at = DateTime::from_timestamp_millis(i64::try_from(entry.at_ms).unwrap_or(0))
            .unwrap_or_default();
        let confirmed_on = zone.local_date(at).format("%Y-%m-%d").to_string();
        tx.execute(
            "UPDATE devices SET state = 'active', confirmed_at = ?2, confirmed_on = ?3 WHERE device_ref = ?1",
            params![device.as_slice(), now, confirmed_on],
        )?;
        audit::record(
            &tx,
            now,
            AuditEvent::DeviceConfirmed,
            Some(&user),
            Some(device),
            None,
        )?;
        tx.commit()?;
        logging::log(
            "device_confirmed",
            &[
                ("user", V::Id(&user)),
                ("device", V::Id(device)),
                ("seq", V::N(entry.seq)),
            ],
        );
        Ok(())
    }

    /// Reject a pending device or revoke an active one (admin page). Allowed during
    /// recovery hold (it can only reduce access).
    ///
    /// # Errors
    /// [`AdminError::NotFound`] or [`AdminError::AlreadyRevoked`].
    pub fn revoke_device(&self, device: &[u8; 16]) -> Result<(), AdminError> {
        let now = self.now_ms();
        let mut conn = self.db();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row: Option<(Vec<u8>, String)> = tx
            .query_row(
                "SELECT user_id, state FROM devices WHERE device_ref = ?1",
                [device.as_slice()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let (user, state) = row.ok_or(AdminError::NotFound)?;
        let user = id16(user)?;
        let state = DeviceState::parse(&state);
        if state == DeviceState::Revoked {
            return Err(AdminError::AlreadyRevoked);
        }
        self.revoke_in(&tx, device, &user, RevocationReason::Admin, now)?;
        let event = if state == DeviceState::Pending {
            AuditEvent::DeviceRejected
        } else {
            AuditEvent::DeviceRevoked
        };
        audit::record(&tx, now, event, Some(&user), Some(device), None)?;
        tx.commit()?;
        logging::log(
            "device_revoked",
            &[
                ("user", V::Id(&user)),
                ("device", V::Id(device)),
                ("reason", V::Name("admin")),
            ],
        );
        Ok(())
    }

    /// Reject every pending device of a user (when the user reports `slip_invalid`,
    /// contract TH3). Returns how many were rejected.
    ///
    /// # Errors
    /// [`AdminError::NotFound`] if the user does not exist.
    pub fn reject_pending_for_user(&self, user: &[u8; 16]) -> Result<usize, AdminError> {
        let now = self.now_ms();
        let mut conn = self.db();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let exists: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM users WHERE user_id = ?1)",
            [user.as_slice()],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(AdminError::NotFound);
        }
        let pending: Vec<[u8; 16]> = tx
            .prepare("SELECT device_ref FROM devices WHERE user_id = ?1 AND state = 'unconfirmed'")?
            .query_map([user.as_slice()], |r| id16(r.get(0)?))?
            .collect::<rusqlite::Result<_>>()?;
        for d in &pending {
            self.revoke_in(&tx, d, user, RevocationReason::Admin, now)?;
        }
        audit::record(
            &tx,
            now,
            AuditEvent::PendingRejectedForUser,
            Some(user),
            None,
            None,
        )?;
        tx.commit()?;
        Ok(pending.len())
    }

    /// Users for the admin page.
    ///
    /// # Errors
    /// [`AdminError::Storage`].
    pub fn users(&self) -> Result<Vec<UserRow>, AdminError> {
        let now = self.now_ms();
        let conn = self.db();
        let mut stmt = conn.prepare(
            "SELECT u.user_id, u.alias, u.hold,
                    (SELECT COUNT(*) FROM slips s WHERE s.user_id = u.user_id AND s.used_at IS NULL
                         AND s.voided = 0 AND s.expires_at > ?1)
             FROM users u ORDER BY u.created_at, u.alias",
        )?;
        let rows = stmt
            .query_map([now], |r| {
                Ok(UserRow {
                    user_id: id16(r.get(0)?)?,
                    alias: r.get(1)?,
                    hold: r.get(2)?,
                    outstanding_slips: r.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Devices for the admin page.
    ///
    /// # Errors
    /// [`AdminError::Storage`].
    pub fn devices(&self) -> Result<Vec<DeviceRow>, AdminError> {
        let conn = self.db();
        let mut stmt = conn.prepare(
            "SELECT device_ref, user_id, label, state, created_at, confirmed_at, last_seen, revoke_reason
             FROM devices ORDER BY created_at, device_ref",
        )?;
        let rows = stmt
            .query_map([], |r| {
                let device_ref = id16(r.get(0)?)?;
                Ok(DeviceRow {
                    check_code: device_check_code(&device_ref),
                    device_ref,
                    user_id: id16(r.get(1)?)?,
                    label: r.get(2)?,
                    state: DeviceState::parse(&r.get::<_, String>(3)?),
                    created_at: r.get(4)?,
                    confirmed_at: r.get(5)?,
                    last_seen: r.get(6)?,
                    revoke_reason: r.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Set or replace the admin passphrase (Argon2id; contract 6.1).
    ///
    /// # Errors
    /// [`AdminError::BadAlias`] if the passphrase is shorter than 12 characters;
    /// [`AdminError::Storage`] otherwise.
    pub fn set_admin_passphrase(&self, passphrase: &Secret<String>) -> Result<(), AdminError> {
        if passphrase.expose().chars().count() < 12 {
            return Err(AdminError::BadAlias);
        }
        let salt: [u8; 16] = self.random();
        let stored = crate::admin::hash_passphrase(passphrase, &salt, self.config.argon2)
            .ok_or(AdminError::Storage)?;
        let now = self.now_ms();
        self.db().execute(
            "INSERT INTO admin (id, passphrase_hash, failures, locked_until, updated_at)
             VALUES (1, ?1, 0, 0, ?2)
             ON CONFLICT (id) DO UPDATE SET passphrase_hash = excluded.passphrase_hash,
                 failures = 0, locked_until = 0, updated_at = excluded.updated_at",
            params![stored, now],
        )?;
        self.db().execute("DELETE FROM admin_sessions", [])?;
        Ok(())
    }

    // -------------------------------------------------------------- pairing

    /// Count one pairing attempt from `ip` against the persisted limits (4.4). Returns
    /// the seconds to wait when a limit is reached.
    ///
    /// # Errors
    /// [`Refusal::RateLimited`] when over a limit; a storage error otherwise.
    pub fn count_pair_attempt(&self, ip: IpAddr) -> Result<(), Refusal> {
        let now = self.now_ms();
        let minute = now - now.rem_euclid(60_000);
        let hour = now - now.rem_euclid(3_600_000);
        let (per_min, per_hour, global) = self.config.pair_limits;
        let ip_s = ip.to_string();
        let scopes = [
            (format!("ipm:{ip_s}"), minute, 60_000, per_min),
            (format!("iph:{ip_s}"), hour, 3_600_000, per_hour),
            ("global".to_owned(), hour, 3_600_000, global),
        ];
        let mut conn = self.db();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut wait: Option<i64> = None;
        for (scope, start, len, limit) in &scopes {
            let count: i64 = tx
                .query_row(
                    "SELECT count FROM rate_counters WHERE scope = ?1 AND window_start = ?2",
                    params![scope, start],
                    |r| r.get(0),
                )
                .optional()?
                .unwrap_or(0);
            if count >= i64::from(*limit) {
                let w = start + len - now;
                wait = Some(wait.map_or(w, |x| x.max(w)));
            }
        }
        if let Some(w) = wait {
            audit::record(&tx, now, AuditEvent::PairRateLimited, None, None, Some(ip))?;
            tx.commit()?;
            let secs = u64::try_from((w + 999) / 1000).unwrap_or(1).max(1);
            return Err(Refusal::RateLimited(secs));
        }
        for (scope, start, _, _) in &scopes {
            tx.execute(
                "INSERT INTO rate_counters (scope, window_start, count) VALUES (?1, ?2, 1)
                 ON CONFLICT (scope, window_start) DO UPDATE SET count = count + 1",
                params![scope, start],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Redeem a slip (contract 4.4): one `BEGIN IMMEDIATE` transaction that marks the
    /// slip used (exactly one row), creates the pending device and appends its `pairing`
    /// entry. Any failure rolls everything back.
    ///
    /// # Errors
    /// `slip_invalid` for an unknown, used, voided or expired slip (one answer for all);
    /// `recovery_hold`, `origin_conflict`, `schema_invalid`, `protocol_unsupported`.
    pub fn redeem(
        &self,
        req: &PairRequest,
        ip: IpAddr,
    ) -> Result<(PairResponse, Secret<[u8; TOKEN_LEN]>), Refusal> {
        if req.protocol != PROTOCOL {
            return Err(WireError::ProtocolUnsupported.into());
        }
        if !label_ok(&req.device_label, MAX_LABEL)
            || !label_ok(&req.app_version, MAX_VERSION)
            || !label_ok(&req.windows_build, MAX_VERSION)
        {
            return Err(WireError::SchemaInvalid.into());
        }
        let Some(secret) = hy_core::protocol::slip::decode_secret(&req.secret) else {
            self.audit_refused(ip);
            return Err(WireError::SlipInvalid.into());
        };
        let now = self.now_ms();
        let origin = *req.origin_device.as_bytes();
        let outcome = (|| -> Result<_, Refusal> {
            let mut conn = self.db();
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let used = tx.execute(
                "UPDATE slips SET used_at = ?1
                 WHERE slip_hash = ?2 AND used_at IS NULL AND expires_at > ?1 AND voided = 0",
                params![now, slip_hash(secret.expose()).as_slice()],
            )?;
            if used != 1 {
                return Err(WireError::SlipInvalid.into());
            }
            let (user, alias, hold): (Vec<u8>, String, bool) = tx.query_row(
                "SELECT u.user_id, u.alias, u.hold FROM slips s JOIN users u ON u.user_id = s.user_id
                 WHERE s.slip_hash = ?1",
                [slip_hash(secret.expose()).as_slice()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?;
            let user = id16(user)?;
            if hold {
                return Err(WireError::RecoveryHold.into());
            }
            let conflict: bool = tx.query_row(
                "SELECT EXISTS (SELECT 1 FROM devices
                 WHERE origin_device = ?1 AND user_id <> ?2 AND state <> 'revoked')",
                params![origin.as_slice(), user.as_slice()],
                |r| r.get(0),
            )?;
            if conflict {
                return Err(WireError::OriginConflict.into());
            }
            // The same user pairing the same PC again: the old device is superseded.
            let old: Vec<[u8; 16]> = tx
                .prepare(
                    "SELECT device_ref FROM devices WHERE origin_device = ?1 AND user_id = ?2 AND state <> 'revoked'",
                )?
                .query_map(params![origin.as_slice(), user.as_slice()], |r| id16(r.get(0)?))?
                .collect::<rusqlite::Result<_>>()?;
            for d in &old {
                self.revoke_in(&tx, d, &user, RevocationReason::Superseded, now)?;
                audit::record(
                    &tx,
                    now,
                    AuditEvent::DeviceSuperseded,
                    Some(&user),
                    Some(d),
                    None,
                )?;
            }
            let token = Secret::new(self.random::<TOKEN_LEN>());
            let verifier = token_verifier(token.expose());
            let device: [u8; 16] = self.random();
            tx.execute(
                "INSERT INTO devices (device_ref, user_id, origin_device, label, app_version,
                     windows_build, token_verifier, state, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'unconfirmed', ?8)",
                params![
                    device.as_slice(),
                    user.as_slice(),
                    origin.as_slice(),
                    req.device_label,
                    req.app_version,
                    req.windows_build,
                    verifier.as_slice(),
                    now
                ],
            )?;
            let (entry, _) = chain::append(
                &tx,
                &self.keys.key_id,
                &user,
                u64::try_from(now).unwrap_or(0),
                EntryBody::Pairing {
                    device_ref: device,
                    origin_device: origin,
                    token_verifier: verifier,
                },
            )?;
            audit::record(
                &tx,
                now,
                AuditEvent::SlipRedeemed,
                Some(&user),
                Some(&device),
                Some(ip),
            )?;
            tx.commit()?;
            Ok((user, alias, device, token, entry.seq))
        })();
        match outcome {
            Ok((user, alias, device, token, seq)) => {
                logging::log(
                    "slip_redeemed",
                    &[
                        ("user", V::Id(&user)),
                        ("device", V::Id(&device)),
                        ("seq", V::N(seq)),
                    ],
                );
                Ok((
                    PairResponse {
                        user_id: Uuid::from_bytes(user),
                        device_ref: Uuid::from_bytes(device),
                        alias,
                        token: b64(token.expose()),
                        head_key: b64(&self.keys.head_public),
                        server_fingerprint: b64(&self.keys.fingerprint),
                        status: DeviceStatus::Pending,
                    },
                    token,
                ))
            }
            Err(e) => {
                if e == Refusal::Wire(WireError::SlipInvalid) {
                    self.audit_refused(ip);
                }
                Err(e)
            }
        }
    }

    fn audit_refused(&self, ip: IpAddr) {
        let conn = self.db();
        let _ = audit::record(
            &conn,
            self.now_ms(),
            AuditEvent::SlipRefused,
            None,
            None,
            Some(ip),
        );
    }

    // -------------------------------------------------------------- tokens

    /// The device behind a bearer token. The state is read from the database on every
    /// call (no cache), so a revocation applies to the next request.
    ///
    /// # Errors
    /// `unknown_token` for a missing, malformed or unknown token; `device_revoked` for a
    /// revoked device (its verifier is kept so the two can be told apart).
    pub fn authenticate(&self, token: Option<&[u8; TOKEN_LEN]>) -> Result<AuthedDevice, WireError> {
        let token = token.ok_or(WireError::UnknownToken)?;
        let verifier = token_verifier(token);
        let conn = self.db();
        let row: Option<DeviceAuthRow> = conn
            .query_row(
                "SELECT device_ref, user_id, origin_device, token_verifier, state FROM devices WHERE token_verifier = ?1",
                [verifier.as_slice()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()
            .map_err(|_| WireError::CannotRecord)?;
        let (device, user, origin, stored, state) = row.ok_or(WireError::UnknownToken)?;
        let stored = h32(stored).map_err(|_| WireError::UnknownToken)?;
        if !ct_eq32(&stored, &verifier) {
            return Err(WireError::UnknownToken);
        }
        let state = DeviceState::parse(&state);
        if state == DeviceState::Revoked {
            return Err(WireError::DeviceRevoked);
        }
        let device = id16(device).map_err(|_| WireError::UnknownToken)?;
        conn.execute(
            "UPDATE devices SET last_seen = ?2 WHERE device_ref = ?1",
            params![device.as_slice(), self.now_ms()],
        )
        .map_err(|_| WireError::CannotRecord)?;
        Ok(AuthedDevice {
            device_ref: device,
            user_id: id16(user).map_err(|_| WireError::UnknownToken)?,
            origin_device: id16(origin).map_err(|_| WireError::UnknownToken)?,
            state,
        })
    }

    /// `GET /v1/state` for an authenticated device (contract 4.5 and 8.1). A pending
    /// device learns only its status and the user's alias.
    ///
    /// # Errors
    /// `cannot_record` on a storage failure.
    pub fn state_for(
        &self,
        dev: &AuthedDevice,
        after_seq: u64,
    ) -> Result<StateResponse, WireError> {
        let now = self.now_ms();
        let conn = self.db();
        let store_err = |_| WireError::CannotRecord;
        let alias: String = conn
            .query_row(
                "SELECT alias FROM users WHERE user_id = ?1",
                [dev.user_id.as_slice()],
                |r| r.get(0),
            )
            .map_err(store_err)?;
        if dev.state != DeviceState::Active {
            return Ok(StateResponse::Pending { alias });
        }
        let (confirmed_on, hold): (String, bool) = conn
            .query_row(
                "SELECT d.confirmed_on, u.hold FROM devices d JOIN users u ON u.user_id = d.user_id
                 WHERE d.device_ref = ?1",
                [dev.device_ref.as_slice()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(store_err)?;
        let devices = conn
            .prepare(
                "SELECT device_ref, label, created_at, last_seen, state FROM devices
                 WHERE user_id = ?1 AND state <> 'revoked' ORDER BY created_at, device_ref",
            )
            .and_then(|mut s| {
                s.query_map([dev.user_id.as_slice()], |r| {
                    Ok(DeviceInfo {
                        device_ref: Uuid::from_bytes(id16(r.get(0)?)?),
                        label: r.get(1)?,
                        paired_at_ms: r.get(2)?,
                        last_seen_ms: r.get(3)?,
                        confirmed: r.get::<_, String>(4)? == "active",
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()
            })
            .map_err(store_err)?;
        let mut entries = chain::entries_after(&conn, &dev.user_id, after_seq, ENTRY_PAGE + 1)
            .map_err(store_err)?;
        let more = entries.len() > ENTRY_PAGE;
        entries.truncate(ENTRY_PAGE);
        // No new head is sent to a user in recovery hold (architecture 5.3).
        let head = if hold {
            None
        } else {
            Some(
                chain::signed_head(
                    &conn,
                    &self.keys,
                    &dev.user_id,
                    u64::try_from(now).unwrap_or(0),
                )
                .map_err(store_err)?
                .to_wire(),
            )
        };
        Ok(StateResponse::Confirmed {
            alias,
            confirmed_on,
            devices,
            entries,
            more,
            head,
        })
    }

    /// `POST /v1/unpair`: the device revokes itself (reason `self`).
    ///
    /// # Errors
    /// `cannot_record` on a storage failure.
    pub fn unpair(&self, dev: &AuthedDevice) -> Result<(), WireError> {
        let now = self.now_ms();
        let mut conn = self.db();
        let res = (|| -> rusqlite::Result<()> {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            self.revoke_in(
                &tx,
                &dev.device_ref,
                &dev.user_id,
                RevocationReason::SelfUnpair,
                now,
            )?;
            audit::record(
                &tx,
                now,
                AuditEvent::DeviceUnpaired,
                Some(&dev.user_id),
                Some(&dev.device_ref),
                None,
            )?;
            tx.commit()
        })();
        res.map_err(|_| WireError::CannotRecord)?;
        logging::log(
            "device_revoked",
            &[
                ("user", V::Id(&dev.user_id)),
                ("device", V::Id(&dev.device_ref)),
                ("reason", V::Name("self")),
            ],
        );
        Ok(())
    }

    // ---------------------------------------------------------- maintenance

    /// Periodic work: revoke devices pending for 7 days (`unconfirmed_expired`), and
    /// prune old audit entries, finished slips, rate counters and idle admin sessions.
    /// Returns the number of devices revoked.
    ///
    /// # Errors
    /// A storage error; nothing is half-applied.
    pub fn maintenance(&self) -> rusqlite::Result<usize> {
        let now = self.now_ms();
        let mut conn = self.db();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let expired: Vec<([u8; 16], [u8; 16])> = tx
            .prepare(
                "SELECT device_ref, user_id FROM devices WHERE state = 'unconfirmed' AND created_at <= ?1",
            )?
            .query_map([now - PENDING_EXPIRY_MS], |r| Ok((id16(r.get(0)?)?, id16(r.get(1)?)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for (d, u) in &expired {
            self.revoke_in(&tx, d, u, RevocationReason::UnconfirmedExpired, now)?;
            audit::record(&tx, now, AuditEvent::DeviceExpired, Some(u), Some(d), None)?;
        }
        tx.execute(
            "DELETE FROM audit WHERE at < ?1",
            [now - AUDIT_RETENTION_MS],
        )?;
        tx.execute(
            "DELETE FROM slips WHERE (used_at IS NOT NULL OR voided = 1 OR expires_at <= ?1)
                 AND created_at < ?1",
            [now - AUDIT_RETENTION_MS],
        )?;
        tx.execute(
            "DELETE FROM rate_counters WHERE window_start < ?1",
            [now - 2 * 3_600_000],
        )?;
        tx.execute(
            "DELETE FROM admin_sessions WHERE last_used < ?1",
            [now - SESSION_IDLE_MS],
        )?;
        tx.commit()?;
        if !expired.is_empty() {
            logging::log("pending_expired", &[("count", V::N(expired.len() as u64))]);
        }
        Ok(expired.len())
    }
}
