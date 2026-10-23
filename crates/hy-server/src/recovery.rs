//! Startup checks, recovery hold, release and discard (V1-5 contract 7.3 and 9;
//! architecture 5.3).
//!
//! **Startup** ([`Server::startup`], before the API opens). The mirror is read and
//! checked; then, for each user, the database tail is compared with the mirror tail and
//! `mirror_hwm`:
//!
//! | Database vs mirror for U | Action |
//! | --- | --- |
//! | Equal, same hash | Open |
//! | Database ahead, mirror tail ≥ `mirror_hwm(U)` | Re-append the missing entries (T-J2) |
//! | Mirror ahead | The database was rolled back or restored: **every user enters hold** |
//! | Mirror tail < `mirror_hwm(U)` | The mirror is an old copy: global discard |
//! | Different hash at a common seq; broken chain; foreign or missing mirror | Global discard |
//!
//! A restore through `hy-server restore` also sets the hold explicitly. Entering hold
//! voids every unused slip and replays the mirror entries newer than the snapshot,
//! re-applying their effects: `pairing` entries carry the token verifier, so devices
//! paired after the snapshot authenticate with their own tokens (T-S16).
//!
//! **Release** of user U needs all of: the mirror (checked at startup), a receipt from
//! every device U's chain lists as confirmed and not revoked (a pending device never
//! received a head, so it has none to present), each presented with that
//! device's own token, at or below U's tail with a matching hash, and the administrator's
//! witness of the list of users and of U's devices. Release is immediate once complete;
//! disables and deletions accepted during hold are then sequenced. Elapsed time is never
//! evidence: at the 7-day maximum an incomplete user is discarded.
//!
//! **Per-user discard** (U only): U's shared summaries are deleted, U's devices revoked
//! (`recovery_discard`) and U's categories switched off, in U's chain. **Global discard**
//! does that for every user and starts a new mirror.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use hy_core::protocol::chain::{EntryBody, RevocationReason, from_day_number};
use hy_core::protocol::token_verifier;
use hy_core::protocol::wire::{ControlOp, ControlOutcome};
use hy_core::schema::Category;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

use crate::audit::{self, AuditEvent};
use crate::chain::{self, StoredEntry};
use crate::db::id16;
use crate::logging::{self, V};
use crate::mirror::{self, Mirror, MirrorError, Scan};
use crate::server::{AdminError, DeviceState, Server};
use crate::sync::{RETENTION_DAYS, apply_data_effect, read_policy, write_policy};

/// The maximum recovery hold (owner decision, 21 Oct 2026).
pub const HOLD_MAX_MS: i64 = 7 * 24 * 3600 * 1000;

/// What [`Server::startup`] found and did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Startup {
    /// No mirror existed yet: one was created holding every entry.
    Created,
    /// The mirror and the database agree.
    Opened,
    /// Entries committed but not yet mirrored were re-appended (and a torn final line
    /// truncated, if there was one).
    Repaired {
        /// Entries re-appended.
        reappended: u64,
        /// A torn final line was truncated.
        torn: bool,
    },
    /// The database was restored or rolled back: every user is in recovery hold.
    Hold {
        /// Mirror entries replayed into the database.
        replayed: u64,
    },
    /// The global discard path ran.
    GlobalDiscard {
        /// Why (a fixed name).
        reason: &'static str,
    },
}

/// One user in hold, for the admin page.
#[derive(Debug, Clone)]
pub struct HoldRow {
    /// The user.
    pub user_id: [u8; 16],
    /// The alias.
    pub alias: String,
    /// When the hold started.
    pub started_at: i64,
    /// The administrator confirmed this user's devices.
    pub witnessed: bool,
    /// The user's tail.
    pub tail: u64,
    /// The user's devices that must check in: (device, label, state, evidence seq).
    pub devices: Vec<([u8; 16], String, DeviceState, Option<u64>)>,
}

fn meta(conn: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
        .optional()
}

fn set_meta(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

fn hex16(s: &str) -> Option<[u8; 16]> {
    let v = data_encoding::HEXLOWER_PERMISSIVE
        .decode(s.as_bytes())
        .ok()?;
    v.try_into().ok()
}

fn users_of(conn: &Connection) -> rusqlite::Result<Vec<[u8; 16]>> {
    conn.prepare("SELECT user_id FROM users ORDER BY created_at, user_id")?
        .query_map([], |r| id16(r.get(0)?))?
        .collect()
}

fn i(n: u64) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

impl Server {
    fn snapshot_id(conn: &Connection) -> [u8; 16] {
        meta(conn, "snapshot_id")
            .ok()
            .flatten()
            .and_then(|s| hex16(&s))
            .unwrap_or_default()
    }

    /// Run the startup comparison against the mirror at `path` (see the module
    /// documentation). The API must not open before this returns.
    ///
    /// # Errors
    /// A description if the database chain itself is broken or the mirror cannot be
    /// written at all; the server must not start.
    pub fn startup(&self, path: &Path) -> Result<Startup, String> {
        *self.mirror_path_guard() = Some(path.to_path_buf());
        let mut conn = self.db();
        chain::verify_all(&conn, &self.keys.key_id).map_err(|e| format!("control log: {e}"))?;
        let stored_id = meta(&conn, "mirror_id")
            .map_err(|e| e.to_string())?
            .and_then(|s| hex16(&s));
        let restore_pending = meta(&conn, "restore_pending")
            .map_err(|e| e.to_string())?
            .is_some();
        let scan = match (mirror::scan(path, &self.keys.key_id), stored_id) {
            (Err(MirrorError::Missing), None) => {
                self.new_mirror(&conn, path)?;
                return Ok(Startup::Created);
            }
            (Err(e), _) => {
                let reason = match e {
                    MirrorError::Missing => "mirror_missing",
                    MirrorError::Unreadable => "mirror_unreadable",
                    MirrorError::Corrupt => "mirror_corrupt",
                    MirrorError::Foreign => "mirror_foreign",
                };
                return self.global_discard_locked(&mut conn, reason);
            }
            (Ok(scan), id) if Some(scan.header.mirror_id) != id => {
                let _ = scan;
                return self.global_discard_locked(&mut conn, "mirror_foreign");
            }
            (Ok(scan), _) => scan,
        };
        let mut users: BTreeSet<[u8; 16]> = users_of(&conn)
            .map_err(|e| e.to_string())?
            .into_iter()
            .collect();
        users.extend(scan.tails.keys().copied());
        let mut restored = restore_pending;
        let mut reappend = 0u64;
        for u in &users {
            let (db_tail, _) =
                chain::tail(&conn, &self.keys.key_id, u).map_err(|e| e.to_string())?;
            let (m_tail, _) = scan.tail(u);
            let hwm = Self::mirrored_seq(&conn, u);
            let common = db_tail.min(m_tail);
            if common > 0 {
                let db_hash = chain::hash_at(&conn, &self.keys.key_id, u, common)
                    .map_err(|e| e.to_string())?;
                if db_hash != scan.hash_at(u, common) {
                    return self.global_discard_locked(&mut conn, "mirror_fork");
                }
            }
            if m_tail > db_tail {
                restored = true;
            } else if db_tail > m_tail {
                if m_tail < hwm {
                    return self.global_discard_locked(&mut conn, "mirror_behind");
                }
                reappend += db_tail - m_tail;
            }
        }
        let m = Mirror::open(path, &scan).map_err(|e| format!("mirror: {e}"))?;
        *self.mirror_guard() = Some(m);
        let outcome = if restored {
            let replayed = self
                .enter_hold_all(&mut conn, &scan)
                .map_err(|e| format!("recovery hold: {e}"))?;
            Startup::Hold { replayed }
        } else if reappend > 0 || scan.torn {
            Startup::Repaired {
                reappended: reappend,
                torn: scan.torn,
            }
        } else {
            Startup::Opened
        };
        // Every committed entry is mirrored, and its high-water recorded, before any
        // client is answered (T-J6).
        for u in users_of(&conn).map_err(|e| e.to_string())? {
            if !self.mirror_sync(&conn, &u) {
                return Err("the mirror cannot be written".into());
            }
        }
        chain::verify_all(&conn, &self.keys.key_id).map_err(|e| format!("control log: {e}"))?;
        logging::log(
            "startup_check",
            &[(
                "outcome",
                V::Name(match outcome {
                    Startup::Created => "created",
                    Startup::Opened => "opened",
                    Startup::Repaired { .. } => "repaired",
                    Startup::Hold { .. } => "recovery_hold",
                    Startup::GlobalDiscard { .. } => "global_discard",
                }),
            )],
        );
        Ok(outcome)
    }

    /// Create a new mirror at `path` holding every entry in the database.
    fn new_mirror(&self, conn: &Connection, path: &Path) -> Result<(), String> {
        let id: [u8; 16] = self
            .random()
            .map_err(|_| "the random source failed".to_owned())?;
        let m = Mirror::create(path, id, self.keys.key_id, self.now_ms())
            .map_err(|e| format!("mirror: {e}"))?;
        set_meta(conn, "mirror_id", &hy_core::protocol::hex(&id)).map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM mirror_hwm", [])
            .map_err(|e| e.to_string())?;
        *self.mirror_guard() = Some(m);
        for u in users_of(conn).map_err(|e| e.to_string())? {
            if !self.mirror_sync(conn, &u) {
                return Err("the mirror cannot be written".into());
            }
        }
        Ok(())
    }

    /// Every user enters recovery hold; unused slips are voided; mirror entries newer
    /// than the snapshot are replayed with their effects. Returns the entries replayed.
    fn enter_hold_all(&self, conn: &mut Connection, scan: &Scan) -> rusqlite::Result<u64> {
        let now = self.now_ms();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        // A user created after the snapshot exists only in the mirror (no alias there).
        for u in scan.tails.keys() {
            let known: bool = tx.query_row(
                "SELECT EXISTS (SELECT 1 FROM users WHERE user_id = ?1)",
                [u.as_slice()],
                |r| r.get(0),
            )?;
            if !known {
                tx.execute(
                    "INSERT INTO users (user_id, alias, created_at) VALUES (?1, ?2, ?3)",
                    params![
                        u.as_slice(),
                        format!("Restored user {}", hy_core::protocol::hex(&u[..4])),
                        now
                    ],
                )?;
            }
        }
        let mut replayed = 0u64;
        for u in users_of(&tx)? {
            let (db_tail, _) = chain::tail(&tx, &self.keys.key_id, &u)?;
            tx.execute(
                "UPDATE users SET hold = 1, hold_started_at = ?2, snapshot_hwm = ?3, witnessed = 0
                 WHERE user_id = ?1",
                params![u.as_slice(), now, i(db_tail)],
            )?;
            for e in scan
                .entries
                .iter()
                .filter(|e| e.user_id == u && e.seq > db_tail)
            {
                chain::insert_verbatim(&tx, e)?;
                self.replay_effect(&tx, e)?;
                replayed += 1;
            }
            audit::record(&tx, now, AuditEvent::RecoveryHold, Some(&u), None, None)?;
        }
        tx.execute("DELETE FROM evidence", [])?;
        tx.execute(
            "DELETE FROM meta WHERE key IN ('users_witnessed', 'restore_pending')",
            [],
        )?;
        tx.execute(
            "UPDATE slips SET voided = 1 WHERE used_at IS NULL AND voided = 0",
            [],
        )?;
        audit::record(&tx, now, AuditEvent::Restored, None, None, None)?;
        tx.commit()?;
        logging::log("recovery_hold", &[("replayed", V::N(replayed))]);
        Ok(replayed)
    }

    /// Re-apply one replayed entry's effect (contract 9, "Authentication after a
    /// restore").
    fn replay_effect(&self, tx: &Transaction<'_>, e: &StoredEntry) -> rusqlite::Result<()> {
        let Ok(body) = EntryBody::decode(e.kind, &e.body) else {
            return Ok(());
        };
        let at = i(e.at_ms);
        let user = &e.user_id;
        match body {
            EntryBody::Pairing {
                device_ref,
                origin_device,
                token_verifier: verifier,
            } => {
                let verifier = if self.take_fault(|f| &mut f.replay_without_verifier) {
                    // T-S16's negative case: without the verifier the device is unknown.
                    token_verifier(
                        &self
                            .random::<32>()
                            .map_err(|_| rusqlite::Error::InvalidQuery)?,
                    )
                } else {
                    verifier
                };
                tx.execute(
                    "INSERT INTO devices (device_ref, user_id, origin_device, label, app_version,
                         windows_build, token_verifier, state, created_at)
                     VALUES (?1, ?2, ?3, 'Restored PC', 'unknown', 'unknown', ?4, 'unconfirmed', ?5)
                     ON CONFLICT (device_ref) DO NOTHING",
                    params![
                        device_ref.as_slice(),
                        user.as_slice(),
                        origin_device.as_slice(),
                        verifier.as_slice(),
                        at
                    ],
                )?;
            }
            EntryBody::DeviceConfirmed { device_ref } => {
                let zone = Self::user_zone(tx, user)?;
                let when = chrono::DateTime::from_timestamp_millis(at).unwrap_or_default();
                tx.execute(
                    "UPDATE devices SET state = 'active', confirmed_at = ?2, confirmed_on = ?3
                     WHERE device_ref = ?1 AND state = 'unconfirmed'",
                    params![
                        device_ref.as_slice(),
                        at,
                        zone.local_date(when).format("%Y-%m-%d").to_string()
                    ],
                )?;
            }
            EntryBody::Revocation { device_ref, reason } => {
                tx.execute(
                    "UPDATE devices SET state = 'revoked', revoked_at = ?2, revoke_reason = ?3
                     WHERE device_ref = ?1 AND state <> 'revoked'",
                    params![device_ref.as_slice(), at, reason.as_str()],
                )?;
            }
            EntryBody::Policy { op_id, .. }
            | EntryBody::Deletion {
                request_id: op_id, ..
            } => {
                apply_data_effect(tx, user, e.seq, at, &body)?;
                // A device retrying the operation gets its original result.
                tx.execute(
                    "INSERT INTO control_ops (op_id, user_id, outcome, created_at) VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT (op_id) DO UPDATE SET outcome = excluded.outcome",
                    params![
                        op_id.as_slice(),
                        user.as_slice(),
                        serde_json::to_string(&ControlOutcome::Accepted { seq: e.seq })
                            .unwrap_or_default(),
                        at
                    ],
                )?;
            }
            EntryBody::RecoveryDiscard { .. } => {
                Self::discard_effect(tx, user, e.seq)?;
            }
            EntryBody::HistoryAccess { .. } => {}
        }
        Ok(())
    }

    /// The data effect of a `recovery_discard` entry: summaries deleted, categories off.
    fn discard_effect(tx: &Transaction<'_>, user: &[u8; 16], seq: u64) -> rusqlite::Result<()> {
        for c in Category::ALL.iter().copied() {
            let mut p = read_policy(tx, user, c)?;
            p.enabled = false;
            p.share_from = None;
            p.seq = p.seq.max(seq);
            p.off_seq = p.off_seq.max(seq);
            write_policy(tx, user, c, &p)?;
        }
        tx.execute(
            "DELETE FROM summaries WHERE user_id = ?1",
            [user.as_slice()],
        )?;
        Ok(())
    }

    /// The per-user discard path inside a transaction.
    fn discard_user_in(
        &self,
        tx: &Transaction<'_>,
        user: &[u8; 16],
        now: i64,
    ) -> rusqlite::Result<()> {
        let snapshot_id = Self::snapshot_id(tx);
        let (entry, _) = chain::append(
            tx,
            &self.keys.key_id,
            user,
            u64::try_from(now).unwrap_or(0),
            EntryBody::RecoveryDiscard { snapshot_id },
        )?;
        Self::discard_effect(tx, user, entry.seq)?;
        let devices: Vec<[u8; 16]> = tx
            .prepare("SELECT device_ref FROM devices WHERE user_id = ?1 AND state <> 'revoked'")?
            .query_map([user.as_slice()], |r| id16(r.get(0)?))?
            .collect::<rusqlite::Result<_>>()?;
        for d in &devices {
            self.revoke_in(tx, d, user, RevocationReason::RecoveryDiscard, now)?;
        }
        tx.execute(
            "DELETE FROM pending_ops WHERE user_id = ?1",
            [user.as_slice()],
        )?;
        tx.execute("DELETE FROM evidence WHERE user_id = ?1", [user.as_slice()])?;
        tx.execute(
            "UPDATE users SET hold = 0, hold_started_at = NULL, witnessed = 0 WHERE user_id = ?1",
            [user.as_slice()],
        )?;
        audit::record(tx, now, AuditEvent::UserDiscarded, Some(user), None, None)?;
        Ok(())
    }

    /// The per-user discard path for `user` (architecture 5.3, "Fail closed").
    ///
    /// # Errors
    /// [`AdminError::NotFound`] or [`AdminError::Storage`].
    pub fn discard_user(&self, user: &[u8; 16], reason: &'static str) -> Result<(), AdminError> {
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
        self.discard_user_in(&tx, user, now)?;
        tx.commit()?;
        self.mirror_sync(&conn, user);
        logging::log(
            "user_discarded",
            &[("user", V::Id(user)), ("reason", V::Name(reason))],
        );
        Ok(())
    }

    /// The global discard path: every user discarded and a new mirror started.
    fn global_discard_locked(
        &self,
        conn: &mut Connection,
        reason: &'static str,
    ) -> Result<Startup, String> {
        let now = self.now_ms();
        let path: PathBuf = self
            .mirror_path_guard()
            .clone()
            .ok_or_else(|| "no mirror path".to_owned())?;
        *self.mirror_guard() = None;
        (|| -> rusqlite::Result<()> {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            for u in users_of(&tx)? {
                self.discard_user_in(&tx, &u, now)?;
            }
            tx.execute(
                "UPDATE slips SET voided = 1 WHERE used_at IS NULL AND voided = 0",
                [],
            )?;
            tx.execute(
                "DELETE FROM meta WHERE key IN ('users_witnessed', 'restore_pending', 'mirror_id')",
                [],
            )?;
            audit::record(&tx, now, AuditEvent::GlobalDiscard, None, None, None)?;
            tx.commit()
        })()
        .map_err(|e| format!("global discard: {e}"))?;
        if path.exists() {
            let aside = path.with_extension(format!("discarded-{now}"));
            std::fs::rename(&path, &aside).map_err(|e| format!("mirror: {e}"))?;
        }
        self.new_mirror(conn, &path)?;
        logging::log("global_discard", &[("reason", V::Name(reason))]);
        Ok(Startup::GlobalDiscard { reason })
    }

    /// The global discard path, from the admin page ("the list of users is not right").
    ///
    /// # Errors
    /// A description if storage or the mirror fails.
    pub fn global_discard(&self, reason: &'static str) -> Result<Startup, String> {
        let mut conn = self.db();
        self.global_discard_locked(&mut conn, reason)
    }

    /// Release `user` if every piece of evidence is present (contract 9). Returns true if
    /// the user was released.
    ///
    /// # Errors
    /// [`AdminError::Storage`].
    pub fn try_release(&self, user: &[u8; 16]) -> Result<bool, AdminError> {
        let now = self.now_ms();
        let mut conn = self.db();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row: Option<(bool, bool)> = tx
            .query_row(
                "SELECT hold, witnessed FROM users WHERE user_id = ?1",
                [user.as_slice()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((true, true)) = row else {
            return Ok(false);
        };
        if meta(&tx, "users_witnessed")?.as_deref() != Some("1") {
            return Ok(false);
        }
        let (tail, _) = chain::tail(&tx, &self.keys.key_id, user)?;
        let listed: Vec<([u8; 16], Option<i64>)> = tx
            .prepare(
                "SELECT d.device_ref, e.seq FROM devices d
                 LEFT JOIN evidence e ON e.device_ref = d.device_ref AND e.user_id = d.user_id
                 WHERE d.user_id = ?1 AND d.state NOT IN ('unconfirmed', 'revoked')",
            )?
            .query_map([user.as_slice()], |r| Ok((id16(r.get(0)?)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        if listed
            .iter()
            .any(|(_, seq)| seq.is_none_or(|s| u64::try_from(s).unwrap_or(u64::MAX) > tail))
        {
            return Ok(false);
        }
        // Disables and deletions accepted during hold are sequenced now, in order.
        let pending: Vec<(i64, Vec<u8>, String)> = tx
            .prepare("SELECT id, device_ref, op FROM pending_ops WHERE user_id = ?1 ORDER BY id")?
            .query_map([user.as_slice()], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for (_, device, op) in &pending {
            let Ok(op) = serde_json::from_str::<ControlOp>(op) else {
                continue;
            };
            let device_ref = id16(device.clone())?;
            let body = match &op {
                ControlOp::Disable { op_id, category } => EntryBody::Policy {
                    category: *category,
                    enabled: false,
                    share_from: 0,
                    op_id: *op_id.as_bytes(),
                    device_ref,
                },
                ControlOp::Delete {
                    request_id,
                    category,
                    from,
                    to,
                } => {
                    let parse = |s: &str| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok();
                    let (Some(f), Some(t)) = (parse(from), parse(to)) else {
                        continue;
                    };
                    EntryBody::Deletion {
                        category: *category,
                        from: hy_core::protocol::chain::day_number(f),
                        to: hy_core::protocol::chain::day_number(t),
                        request_id: *request_id.as_bytes(),
                        device_ref,
                    }
                }
                ControlOp::Enable { .. } => continue,
            };
            let (entry, _) = chain::append(
                &tx,
                &self.keys.key_id,
                user,
                u64::try_from(now).unwrap_or(0),
                body,
            )?;
            apply_data_effect(&tx, user, entry.seq, now, &body)?;
            tx.execute(
                "UPDATE control_ops SET outcome = ?2 WHERE op_id = ?1",
                params![
                    op.id().as_bytes().as_slice(),
                    serde_json::to_string(&ControlOutcome::Accepted { seq: entry.seq })
                        .unwrap_or_default()
                ],
            )?;
        }
        tx.execute(
            "DELETE FROM pending_ops WHERE user_id = ?1",
            [user.as_slice()],
        )?;
        // Regression checks resume from the evidence receipts.
        tx.execute(
            "UPDATE devices SET regressed_at = NULL, max_echoed_seq = coalesce(
                 (SELECT seq FROM evidence e WHERE e.device_ref = devices.device_ref), max_echoed_seq)
             WHERE user_id = ?1",
            [user.as_slice()],
        )?;
        tx.execute("DELETE FROM evidence WHERE user_id = ?1", [user.as_slice()])?;
        tx.execute(
            "UPDATE users SET hold = 0, hold_started_at = NULL, witnessed = 0 WHERE user_id = ?1",
            [user.as_slice()],
        )?;
        audit::record(&tx, now, AuditEvent::UserReleased, Some(user), None, None)?;
        tx.commit()?;
        self.mirror_sync(&conn, user);
        logging::log(
            "user_released",
            &[
                ("user", V::Id(user)),
                ("sequenced", V::N(pending.len() as u64)),
            ],
        );
        Ok(true)
    }

    /// The administrator's witness of the list of users (contract 9). `false` ("the list
    /// is not right") takes the global discard path.
    ///
    /// # Errors
    /// [`AdminError::Storage`].
    pub fn witness_users(&self, list_is_right: bool) -> Result<(), AdminError> {
        if !list_is_right {
            return self
                .global_discard("users_not_witnessed")
                .map(|_| ())
                .map_err(|_| AdminError::Storage);
        }
        let now = self.now_ms();
        let held: Vec<[u8; 16]> = {
            let conn = self.db();
            set_meta(&conn, "users_witnessed", "1")?;
            audit::record(&conn, now, AuditEvent::UsersWitnessed, None, None, None)?;
            conn.prepare("SELECT user_id FROM users WHERE hold = 1")?
                .query_map([], |r| id16(r.get(0)?))?
                .collect::<rusqlite::Result<_>>()?
        };
        for u in &held {
            self.try_release(u)?;
        }
        Ok(())
    }

    /// The administrator's witness of one user's devices: every device paired for the
    /// user is listed and none was paired or revoked after the user's last mirror entry.
    /// `false` takes the user's discard path.
    ///
    /// # Errors
    /// [`AdminError::NotFound`] if the user is not in hold; [`AdminError::Storage`].
    pub fn witness_user(&self, user: &[u8; 16], devices_are_right: bool) -> Result<(), AdminError> {
        {
            let conn = self.db();
            let hold: Option<bool> = conn
                .query_row(
                    "SELECT hold FROM users WHERE user_id = ?1",
                    [user.as_slice()],
                    |r| r.get(0),
                )
                .optional()?;
            if hold != Some(true) {
                return Err(AdminError::NotFound);
            }
            if devices_are_right {
                conn.execute(
                    "UPDATE users SET witnessed = 1 WHERE user_id = ?1",
                    [user.as_slice()],
                )?;
            }
        }
        if devices_are_right {
            self.try_release(user)?;
            Ok(())
        } else {
            self.discard_user(user, "devices_not_witnessed")
        }
    }

    /// Users in hold, with each device's evidence, for the admin page.
    ///
    /// # Errors
    /// [`AdminError::Storage`].
    pub fn hold_rows(&self) -> Result<Vec<HoldRow>, AdminError> {
        let conn = self.db();
        let users: Vec<([u8; 16], String, i64, bool)> = conn
            .prepare(
                "SELECT user_id, alias, coalesce(hold_started_at, 0), witnessed FROM users
                 WHERE hold = 1 ORDER BY created_at",
            )?
            .query_map([], |r| {
                Ok((id16(r.get(0)?)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        let mut out = Vec::new();
        for (user_id, alias, started_at, witnessed) in users {
            let (tail, _) = chain::tail(&conn, &self.keys.key_id, &user_id)?;
            let devices = conn
                .prepare(
                    "SELECT d.device_ref, d.label, d.state, e.seq FROM devices d
                     LEFT JOIN evidence e ON e.device_ref = d.device_ref
                     WHERE d.user_id = ?1 AND d.state <> 'revoked' ORDER BY d.created_at",
                )?
                .query_map([user_id.as_slice()], |r| {
                    Ok((
                        id16(r.get(0)?)?,
                        r.get::<_, String>(1)?,
                        if r.get::<_, String>(2)? == "active" {
                            DeviceState::Active
                        } else {
                            DeviceState::Pending
                        },
                        r.get::<_, Option<i64>>(3)?
                            .map(|s| u64::try_from(s).unwrap_or(0)),
                    ))
                })?
                .collect::<rusqlite::Result<_>>()?;
            out.push(HoldRow {
                user_id,
                alias,
                started_at,
                witnessed,
                tail,
                devices,
            });
        }
        Ok(out)
    }

    /// Whether the administrator has confirmed the list of users during this hold.
    #[must_use]
    pub fn users_witnessed(&self) -> bool {
        meta(&self.db(), "users_witnessed")
            .ok()
            .flatten()
            .as_deref()
            == Some("1")
    }

    /// Daily recovery and retention work: a user still in hold at the 7-day maximum is
    /// discarded (time alone never releases); summaries older than 90 days are pruned.
    ///
    /// # Errors
    /// A storage error.
    pub fn recovery_maintenance(&self) -> rusqlite::Result<usize> {
        let now = self.now_ms();
        let expired: Vec<[u8; 16]> = {
            let conn = self.db();
            let cutoff = self
                .clock
                .now()
                .date_naive()
                .checked_sub_days(chrono::Days::new(RETENTION_DAYS))
                .unwrap_or_default()
                .format("%Y-%m-%d")
                .to_string();
            conn.execute("DELETE FROM summaries WHERE local_date < ?1", [cutoff])?;
            conn.prepare("SELECT user_id FROM users WHERE hold = 1 AND hold_started_at <= ?1")?
                .query_map([now - HOLD_MAX_MS], |r| id16(r.get(0)?))?
                .collect::<rusqlite::Result<_>>()?
        };
        for u in &expired {
            let _ = self.discard_user(u, "hold_maximum");
        }
        Ok(expired.len())
    }

    /// The share-from date of a category for a user (tests and the simulator).
    ///
    /// # Errors
    /// A storage error.
    pub fn policy_of(
        &self,
        user: &[u8; 16],
        category: Category,
    ) -> rusqlite::Result<crate::sync::Policy> {
        read_policy(&self.db(), user, category)
    }
}

/// The date of a day number, as text (for tests).
#[must_use]
pub fn day_text(n: u32) -> String {
    from_day_number(n).map_or_else(String::new, |d| d.format("%Y-%m-%d").to_string())
}
