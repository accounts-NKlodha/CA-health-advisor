//! Sync for confirmed devices (V1-5 contract 7.3, 7.4, 8.1, 8.3, 8.4 and 8.6): the mirror
//! step after each commit, receipt echoes, control operations, uploads, history and
//! receipts.
//!
//! **Order of an acknowledged operation** (architecture 5.3, "Atomic ordering"): one
//! `BEGIN IMMEDIATE` transaction writes the control entry and its effects; after the
//! commit the entry is appended to the mirror and fsynced; then `mirror_hwm` is recorded
//! in a small transaction of its own; only then is the client answered. A signed head
//! never covers an entry the mirror does not hold: heads are issued at `mirror_hwm`.
//!
//! **Control operations** are idempotent by `op_id` or `request_id`: a repeated id gets
//! the stored outcome, never a second application (T-S6). A disable is always accepted;
//! an enable needs `base_seq` at or above the category's latest disable or deletion
//! (`stale_enable` otherwise) and sets `share_from` to the server's date in the user's
//! zone. During recovery hold enables are refused, while disables and deletions are
//! applied at once and acknowledged `accepted_pending_recovery` with no seq; they are
//! sequenced on release (contract 9).
//!
//! **Uploads** pass every check of 8.6 or are rejected per row with a reason; the device
//! discards a rejected row and never re-stamps it.

use chrono::{Days, NaiveDate};
use hy_core::protocol::chain::{
    EntryBody, RevocationReason, category_code, day_number, from_day_number,
};
use hy_core::protocol::head::SignedHead;
use hy_core::protocol::share;
use hy_core::protocol::wire::{
    ControlOp, ControlOutcome, ControlRequest, ControlResponse, ControlResult, HistoryResponse,
    HistoryRow, MAX_CONTROL_OPS, MAX_HISTORY_DAYS, MAX_UPLOAD_ROWS, ReceiptResponse, RowOutcome,
    UploadRequest, UploadResponse, UploadRow, WireError,
};
use hy_core::schema::Category;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use uuid::Uuid;

use crate::audit::{self, AuditEvent};
use crate::chain;
use crate::logging::{self, V};
use crate::server::{AuthedDevice, DeviceState, Server};

/// Days back an upload may be dated, and the server's summary retention (8.6).
pub const RETENTION_DAYS: u64 = 90;
/// Largest stored payload, bytes of canonical JSON.
pub const MAX_PAYLOAD: usize = 1024;
/// A second regression within this window revokes the device (7.4).
pub const REGRESSION_WINDOW_MS: i64 = 24 * 3600 * 1000;

/// One user's policy row for a category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Policy {
    /// On.
    pub enabled: bool,
    /// The first shareable date.
    pub share_from: Option<NaiveDate>,
    /// The latest policy or deletion entry for the category.
    pub seq: u64,
    /// The latest disable or deletion.
    pub off_seq: u64,
}

fn date_text(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

fn parse_date(s: &str) -> Option<NaiveDate> {
    if s.len() != 10 {
        return None;
    }
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

fn u(n: i64) -> u64 {
    u64::try_from(n).unwrap_or(0)
}

fn i(n: u64) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// Read a user's policy row for a category (all off when absent).
///
/// # Errors
/// Any SQLite error.
pub fn read_policy(
    conn: &Connection,
    user: &[u8; 16],
    category: Category,
) -> rusqlite::Result<Policy> {
    let row: Option<(bool, Option<String>, i64, i64)> = conn
        .query_row(
            "SELECT enabled, share_from, seq, off_seq FROM policy WHERE user_id = ?1 AND category = ?2",
            params![user.as_slice(), category.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    Ok(
        row.map_or_else(Policy::default, |(enabled, from, seq, off)| Policy {
            enabled,
            share_from: from.as_deref().and_then(parse_date),
            seq: u(seq),
            off_seq: u(off),
        }),
    )
}

/// Write a user's policy row for a category.
///
/// # Errors
/// Any SQLite error.
pub fn write_policy(
    conn: &Connection,
    user: &[u8; 16],
    category: Category,
    p: &Policy,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO policy (user_id, category, enabled, share_from, seq, off_seq)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (user_id, category) DO UPDATE SET enabled = excluded.enabled,
             share_from = excluded.share_from, seq = excluded.seq, off_seq = excluded.off_seq",
        params![
            user.as_slice(),
            category.as_str(),
            p.enabled,
            p.share_from.map(date_text),
            i(p.seq),
            i(p.off_seq)
        ],
    )?;
    Ok(())
}

/// Apply the effect of a policy or deletion entry (live, on replay and on release).
///
/// # Errors
/// Any SQLite error.
pub fn apply_data_effect(
    conn: &Connection,
    user: &[u8; 16],
    seq: u64,
    at_ms: i64,
    body: &EntryBody,
) -> rusqlite::Result<()> {
    match *body {
        EntryBody::Policy {
            category,
            enabled,
            share_from,
            ..
        } => {
            let mut p = read_policy(conn, user, category)?;
            p.enabled = enabled;
            p.share_from = if enabled {
                hy_core::protocol::chain::from_day_number(share_from)
            } else {
                None
            };
            p.seq = p.seq.max(seq);
            if !enabled {
                p.off_seq = p.off_seq.max(seq);
            }
            write_policy(conn, user, category, &p)
        }
        EntryBody::Deletion {
            category,
            from,
            to,
            request_id,
            ..
        } => {
            let (Some(from), Some(to)) = (
                hy_core::protocol::chain::from_day_number(from),
                hy_core::protocol::chain::from_day_number(to),
            ) else {
                return Ok(());
            };
            delete_range(
                conn,
                user,
                category,
                from,
                to,
                &request_id,
                Some(seq),
                at_ms,
            )?;
            let mut p = read_policy(conn, user, category)?;
            p.seq = p.seq.max(seq);
            p.off_seq = p.off_seq.max(seq);
            write_policy(conn, user, category, &p)
        }
        _ => Ok(()),
    }
}

/// Insert (or sequence) a tombstone and delete the summaries it covers (8.4).
#[allow(clippy::too_many_arguments)]
fn delete_range(
    conn: &Connection,
    user: &[u8; 16],
    category: Category,
    from: NaiveDate,
    to: NaiveDate,
    request_id: &[u8; 16],
    seq: Option<u64>,
    at_ms: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO tombstones (request_id, user_id, category, from_date, to_date, seq, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT (request_id) DO UPDATE SET seq = coalesce(tombstones.seq, excluded.seq)",
        params![
            request_id.as_slice(),
            user.as_slice(),
            category.as_str(),
            date_text(from),
            date_text(to),
            seq.map(i),
            at_ms
        ],
    )?;
    conn.execute(
        "DELETE FROM summaries WHERE user_id = ?1 AND category = ?2 AND local_date BETWEEN ?3 AND ?4",
        params![
            user.as_slice(),
            category.as_str(),
            date_text(from),
            date_text(to)
        ],
    )?;
    Ok(())
}

fn outcome_text(o: &ControlOutcome) -> String {
    serde_json::to_string(o).unwrap_or_default()
}

/// The policy for `category` as the mirror holds it: the last `policy` entry at or below
/// `hwm`. An enable committed but not yet mirrored (a crash or mirror failure between
/// commit and append) authorises nothing (Codex review of PR 17).
fn durable_policy(
    tx: &Connection,
    user: &[u8; 16],
    category: Category,
    hwm: u64,
) -> rusqlite::Result<(bool, Option<NaiveDate>)> {
    let mut stmt = tx.prepare(
        "SELECT kind, body FROM control_log WHERE user_id = ?1 AND kind = 1 AND seq <= ?2
         ORDER BY seq DESC",
    )?;
    let mut rows = stmt.query(params![user.as_slice(), i(hwm)])?;
    while let Some(r) = rows.next()? {
        let (kind, body): (u8, Vec<u8>) = (r.get(0)?, r.get(1)?);
        if let Ok(EntryBody::Policy {
            category: c,
            enabled,
            share_from,
            ..
        }) = EntryBody::decode(kind, &body)
            && c == category
        {
            return Ok((enabled, from_day_number(share_from)));
        }
    }
    Ok((false, None))
}

/// Whether the device's `device_confirmed` entry is at or below `hwm` (mirrored).
fn confirmation_mirrored(
    tx: &Connection,
    user: &[u8; 16],
    device: &[u8; 16],
    hwm: u64,
) -> rusqlite::Result<bool> {
    let mut stmt = tx.prepare(
        "SELECT kind, body FROM control_log WHERE user_id = ?1 AND kind = 4 AND seq <= ?2",
    )?;
    let mut rows = stmt.query(params![user.as_slice(), i(hwm)])?;
    while let Some(r) = rows.next()? {
        let (kind, body): (u8, Vec<u8>) = (r.get(0)?, r.get(1)?);
        if let Ok(EntryBody::DeviceConfirmed { device_ref }) = EntryBody::decode(kind, &body)
            && &device_ref == device
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The device behind the request, re-read inside the operation's own transaction: it
/// must still be confirmed, not revoked, and still belong to the same user and origin.
/// Authentication happens before the request body arrives, so an administrator's
/// revocation in between must stop the operation (Codex review of PR 17).
fn still_authorised(
    tx: &Connection,
    dev: &AuthedDevice,
) -> rusqlite::Result<Result<(), WireError>> {
    let row: Option<(Vec<u8>, Vec<u8>, String)> = tx
        .query_row(
            "SELECT user_id, origin_device, state FROM devices WHERE device_ref = ?1",
            [dev.device_ref.as_slice()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    Ok(match row {
        None => Err(WireError::UnknownToken),
        Some((user, origin, state)) => {
            if user != dev.user_id || origin != dev.origin_device {
                Err(WireError::UnknownToken)
            } else if state == "active" {
                Ok(())
            } else if state == "unconfirmed" {
                Err(WireError::DeviceUnconfirmed)
            } else {
                Err(WireError::DeviceRevoked)
            }
        }
    })
}

impl Server {
    // ------------------------------------------------------------- mirror step

    /// Append the user's unmirrored entries to the mirror and fsync, then record
    /// `mirror_hwm` (contract 7.3). False when the mirror is unavailable; the entries
    /// stay committed and are appended by the next call or at startup.
    pub(crate) fn mirror_sync(&self, conn: &Connection, user: &[u8; 16]) -> bool {
        let mut guard = self.mirror_guard();
        let Some(m) = guard.as_mut() else {
            return false;
        };
        let after = m.tail_seq(user);
        let Ok(rows) = chain::rows_after(conn, user, after) else {
            return false;
        };
        if let Err(_e) = m.append(&rows) {
            logging::log(
                "mirror_write_failed",
                &[("user", V::Id(user)), ("count", V::N(rows.len() as u64))],
            );
            return false;
        }
        if !rows.is_empty() && self.take_fault(|f| &mut f.stop_after_mirror) {
            return false;
        }
        let seq = m.tail_seq(user);
        conn.execute(
            "INSERT INTO mirror_hwm (user_id, seq) VALUES (?1, ?2)
             ON CONFLICT (user_id) DO UPDATE SET seq = max(seq, excluded.seq)",
            params![user.as_slice(), i(seq)],
        )
        .is_ok()
    }

    /// The user's `mirror_hwm`: the highest seq a head may cover.
    pub(crate) fn mirrored_seq(conn: &Connection, user: &[u8; 16]) -> u64 {
        conn.query_row(
            "SELECT seq FROM mirror_hwm WHERE user_id = ?1",
            [user.as_slice()],
            |r| r.get::<_, i64>(0),
        )
        .map_or(0, u)
    }

    /// A signed head at the user's `mirror_hwm` (wire form).
    pub(crate) fn mirrored_head(&self, conn: &Connection, user: &[u8; 16]) -> Option<String> {
        let seq = Self::mirrored_seq(conn, user);
        chain::head_at(conn, &self.keys, user, seq, u(self.now_ms()))
            .ok()
            .map(|h| h.to_wire())
    }

    // ------------------------------------------------------------- receipts

    /// Check the receipt a confirmed device echoes in `HY-Receipt` (contract 7.4,
    /// "Echo" and "Regression"). An echo is never recovery evidence. A valid echo lower
    /// than the device's highest is a restored or cloned PC: `device_regressed`, and a
    /// second time within 24 h the device is revoked (`superseded`). Off during hold.
    ///
    /// # Errors
    /// `device_regressed` or `device_revoked`.
    pub fn check_echo(&self, dev: &AuthedDevice, echo: Option<&str>) -> Result<(), WireError> {
        let Some(text) = echo else {
            return Ok(());
        };
        if dev.state != DeviceState::Active {
            return Ok(());
        }
        let Ok(h) = SignedHead::from_wire(text) else {
            return Ok(());
        };
        if h.verify(&self.keys.head_public, &dev.user_id).is_err() {
            return Ok(());
        }
        let now = self.now_ms();
        let mut conn = self.db();
        let res = (|| -> rusqlite::Result<Result<(), WireError>> {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let (hold, max, regressed): (bool, i64, Option<i64>) = tx.query_row(
                "SELECT u.hold, d.max_echoed_seq, d.regressed_at FROM devices d
                 JOIN users u ON u.user_id = d.user_id WHERE d.device_ref = ?1",
                [dev.device_ref.as_slice()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?;
            if hold {
                return Ok(Ok(()));
            }
            let seq = h.head.seq;
            if seq >= u(max) {
                if seq > u(max) {
                    tx.execute(
                        "UPDATE devices SET max_echoed_seq = ?2 WHERE device_ref = ?1",
                        params![dev.device_ref.as_slice(), i(seq)],
                    )?;
                    tx.commit()?;
                }
                return Ok(Ok(()));
            }
            if regressed.is_some_and(|at| now - at < REGRESSION_WINDOW_MS) {
                self.revoke_in(
                    &tx,
                    &dev.device_ref,
                    &dev.user_id,
                    RevocationReason::Superseded,
                    now,
                )?;
                audit::record(
                    &tx,
                    now,
                    AuditEvent::DeviceSuperseded,
                    Some(&dev.user_id),
                    Some(&dev.device_ref),
                    None,
                )?;
                tx.commit()?;
                return Ok(Err(WireError::DeviceRevoked));
            }
            // The PC now holds this older receipt: it is the new baseline, so the same
            // copy does not regress again on its next request.
            tx.execute(
                "UPDATE devices SET regressed_at = ?2, max_echoed_seq = ?3 WHERE device_ref = ?1",
                params![dev.device_ref.as_slice(), now, i(seq)],
            )?;
            audit::record(
                &tx,
                now,
                AuditEvent::DeviceRegressed,
                Some(&dev.user_id),
                Some(&dev.device_ref),
                None,
            )?;
            tx.commit()?;
            Ok(Err(WireError::DeviceRegressed))
        })();
        let out = res.map_err(|_| WireError::CannotRecord)?;
        match out {
            Err(WireError::DeviceRevoked) => {
                self.mirror_sync(&conn, &dev.user_id);
                logging::log(
                    "device_revoked",
                    &[
                        ("user", V::Id(&dev.user_id)),
                        ("device", V::Id(&dev.device_ref)),
                        ("reason", V::Name("superseded")),
                    ],
                );
            }
            Err(WireError::DeviceRegressed) => logging::log(
                "device_regressed",
                &[
                    ("user", V::Id(&dev.user_id)),
                    ("device", V::Id(&dev.device_ref)),
                ],
            ),
            _ => {}
        }
        out
    }

    /// Check a validly signed receipt for its user against that user's chain: a receipt
    /// beyond the tail, or whose chain hash differs at its seq, proves the restored
    /// history incomplete or forked, so the user's discard path runs (architecture 5.3,
    /// "Fail closed"; T-D9, T-D12). This holds whoever presents it, with or without a
    /// token. Returns true if a discard ran.
    pub(crate) fn receipt_incident(&self, h: &SignedHead) -> bool {
        let user = h.head.user_id;
        if h.verify(&self.keys.head_public, &user).is_err() {
            return false;
        }
        let beyond = {
            let conn = self.db();
            let known: bool = conn
                .query_row(
                    "SELECT EXISTS (SELECT 1 FROM users WHERE user_id = ?1)",
                    [user.as_slice()],
                    |r| r.get(0),
                )
                .unwrap_or(false);
            if !known {
                return false;
            }
            match chain::hash_at(&conn, &self.keys.key_id, &user, h.head.seq) {
                Ok(Some(hash)) => hash != h.head.chain_hash,
                Ok(None) => true,
                Err(_) => return false,
            }
        };
        if !beyond {
            return false;
        }
        logging::log(
            "receipt_beyond_history",
            &[("user", V::Id(&user)), ("seq", V::N(h.head.seq))],
        );
        self.discard_user(&user, "receipt_beyond_history").is_ok()
    }

    /// The incident check for a receipt sent without a usable token: never evidence.
    pub fn receipt_without_device(&self, body: &[u8]) {
        let Ok(req) = serde_json::from_slice::<hy_core::protocol::wire::ReceiptRequest>(body)
        else {
            return;
        };
        let Ok(h) = SignedHead::from_wire(&req.head) else {
            return;
        };
        let now = self.now_ms();
        let _ = audit::record(
            &self.db(),
            now,
            AuditEvent::ReceiptRefused,
            None,
            None,
            None,
        );
        self.receipt_incident(&h);
    }

    /// `POST /v1/receipt` from an authenticated, confirmed device: its stored receipt
    /// counts as **this** device's evidence during hold (contract 9), recorded against
    /// the device from the token, never from the body.
    ///
    /// # Errors
    /// `schema_invalid` for a malformed head; `cannot_record` on a storage failure.
    pub fn present_receipt(
        &self,
        dev: &AuthedDevice,
        head: &str,
    ) -> Result<ReceiptResponse, WireError> {
        let h = SignedHead::from_wire(head).map_err(|_| WireError::SchemaInvalid)?;
        let now = self.now_ms();
        let refused = |what: &'static str| {
            let _ = audit::record(
                &self.db(),
                now,
                AuditEvent::ReceiptRefused,
                Some(&dev.user_id),
                Some(&dev.device_ref),
                None,
            );
            logging::log(
                "receipt_refused",
                &[
                    ("user", V::Id(&dev.user_id)),
                    ("device", V::Id(&dev.device_ref)),
                    ("reason", V::Name(what)),
                ],
            );
            Ok(ReceiptResponse { recorded: false })
        };
        if h.verify(&self.keys.head_public, &h.head.user_id).is_err() {
            return refused("bad_signature");
        }
        if self.receipt_incident(&h) {
            return Ok(ReceiptResponse { recorded: false });
        }
        if h.head.user_id != dev.user_id {
            return refused("other_user");
        }
        let recorded = {
            let mut conn = self.db();
            let tx = conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|_| WireError::CannotRecord)?;
            still_authorised(&tx, dev).map_err(|_| WireError::CannotRecord)??;
            let hold: bool = tx
                .query_row(
                    "SELECT hold FROM users WHERE user_id = ?1",
                    [dev.user_id.as_slice()],
                    |r| r.get(0),
                )
                .map_err(|_| WireError::CannotRecord)?;
            if hold {
                tx.execute(
                    "INSERT INTO evidence (user_id, device_ref, seq, recorded_at) VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT (user_id, device_ref) DO UPDATE SET seq = excluded.seq,
                         recorded_at = excluded.recorded_at",
                    params![
                        dev.user_id.as_slice(),
                        dev.device_ref.as_slice(),
                        i(h.head.seq),
                        now
                    ],
                )
                .map_err(|_| WireError::CannotRecord)?;
                audit::record(
                    &tx,
                    now,
                    AuditEvent::ReceiptRecorded,
                    Some(&dev.user_id),
                    Some(&dev.device_ref),
                    None,
                )
                .map_err(|_| WireError::CannotRecord)?;
            }
            tx.commit().map_err(|_| WireError::CannotRecord)?;
            hold
        };
        if recorded {
            logging::log(
                "receipt_recorded",
                &[
                    ("user", V::Id(&dev.user_id)),
                    ("device", V::Id(&dev.device_ref)),
                    ("seq", V::N(h.head.seq)),
                ],
            );
            let _ = self.try_release(&dev.user_id);
        }
        Ok(ReceiptResponse { recorded })
    }

    // ------------------------------------------------------------- control

    fn control_op(
        &self,
        tx: &Transaction<'_>,
        dev: &AuthedDevice,
        op: &ControlOp,
        hold: bool,
        now: i64,
    ) -> rusqlite::Result<ControlOutcome> {
        let user = &dev.user_id;
        let id = op.id();
        let stored: Option<(Vec<u8>, String)> = tx
            .query_row(
                "SELECT user_id, outcome FROM control_ops WHERE op_id = ?1",
                [id.as_bytes().as_slice()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((owner, text)) = stored {
            if owner.as_slice() != user.as_slice() {
                return Ok(ControlOutcome::Rejected {
                    error: WireError::SchemaInvalid,
                });
            }
            return Ok(
                serde_json::from_str(&text).unwrap_or(ControlOutcome::Rejected {
                    error: WireError::CannotRecord,
                }),
            );
        }
        let at = u(now);
        let outcome = match op {
            ControlOp::Enable {
                op_id,
                category,
                base_seq,
            } => {
                if hold {
                    // Not stored: the device keeps it pending and sends it after release.
                    return Ok(ControlOutcome::Rejected {
                        error: WireError::RecoveryHold,
                    });
                }
                let p = read_policy(tx, user, *category)?;
                if *base_seq < p.off_seq {
                    ControlOutcome::StaleEnable {
                        latest_seq: p.off_seq,
                    }
                } else {
                    let zone = Self::user_zone(tx, user)?;
                    let today = zone.local_date(self.clock.now());
                    let body = EntryBody::Policy {
                        category: *category,
                        enabled: true,
                        share_from: day_number(today),
                        op_id: *op_id.as_bytes(),
                        device_ref: dev.device_ref,
                    };
                    let (entry, _) = chain::append(tx, &self.keys.key_id, user, at, body)?;
                    apply_data_effect(tx, user, entry.seq, now, &body)?;
                    ControlOutcome::Accepted { seq: entry.seq }
                }
            }
            ControlOp::Disable { op_id, category } => {
                let body = EntryBody::Policy {
                    category: *category,
                    enabled: false,
                    share_from: 0,
                    op_id: *op_id.as_bytes(),
                    device_ref: dev.device_ref,
                };
                if hold {
                    let mut p = read_policy(tx, user, *category)?;
                    p.enabled = false;
                    p.share_from = None;
                    write_policy(tx, user, *category, &p)?;
                    self.queue_pending(tx, dev, op, now)?;
                    ControlOutcome::AcceptedPendingRecovery
                } else {
                    let (entry, _) = chain::append(tx, &self.keys.key_id, user, at, body)?;
                    apply_data_effect(tx, user, entry.seq, now, &body)?;
                    ControlOutcome::Accepted { seq: entry.seq }
                }
            }
            ControlOp::Delete {
                request_id,
                category,
                from,
                to,
            } => {
                let (Some(f), Some(t)) = (parse_date(from), parse_date(to)) else {
                    return Ok(ControlOutcome::Rejected {
                        error: WireError::SchemaInvalid,
                    });
                };
                if f > t {
                    return Ok(ControlOutcome::Rejected {
                        error: WireError::SchemaInvalid,
                    });
                }
                if hold {
                    delete_range(tx, user, *category, f, t, request_id.as_bytes(), None, now)?;
                    self.queue_pending(tx, dev, op, now)?;
                    ControlOutcome::AcceptedPendingRecovery
                } else {
                    let body = EntryBody::Deletion {
                        category: *category,
                        from: day_number(f),
                        to: day_number(t),
                        request_id: *request_id.as_bytes(),
                        device_ref: dev.device_ref,
                    };
                    let (entry, _) = chain::append(tx, &self.keys.key_id, user, at, body)?;
                    apply_data_effect(tx, user, entry.seq, now, &body)?;
                    ControlOutcome::Accepted { seq: entry.seq }
                }
            }
        };
        tx.execute(
            "INSERT INTO control_ops (op_id, user_id, outcome, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![
                id.as_bytes().as_slice(),
                user.as_slice(),
                outcome_text(&outcome),
                now
            ],
        )?;
        Ok(outcome)
    }

    fn queue_pending(
        &self,
        tx: &Transaction<'_>,
        dev: &AuthedDevice,
        op: &ControlOp,
        now: i64,
    ) -> rusqlite::Result<()> {
        tx.execute(
            "INSERT INTO pending_ops (user_id, device_ref, op, received_at) VALUES (?1, ?2, ?3, ?4)",
            params![
                dev.user_id.as_slice(),
                dev.device_ref.as_slice(),
                serde_json::to_string(op).unwrap_or_default(),
                now
            ],
        )?;
        Ok(())
    }

    /// `POST /v1/control` (contract 8.1, 8.3, 8.4).
    ///
    /// # Errors
    /// `schema_invalid` for an empty or oversized batch; `cannot_record` when the mirror
    /// is unavailable (nothing is acknowledged without it) or storage fails.
    pub fn control(
        &self,
        dev: &AuthedDevice,
        req: &ControlRequest,
    ) -> Result<ControlResponse, WireError> {
        if req.ops.is_empty() || req.ops.len() > MAX_CONTROL_OPS {
            return Err(WireError::SchemaInvalid);
        }
        let now = self.now_ms();
        let mut conn = self.db();
        let hold: bool = conn
            .query_row(
                "SELECT hold FROM users WHERE user_id = ?1",
                [dev.user_id.as_slice()],
                |r| r.get(0),
            )
            .map_err(|_| WireError::CannotRecord)?;
        // The mirror must be writable before anything is recorded (contract 12).
        if !hold && !self.mirror_sync(&conn, &dev.user_id) {
            return Err(WireError::CannotRecord);
        }
        let results = (|| -> rusqlite::Result<Result<Option<Vec<ControlResult>>, WireError>> {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            if let Err(e) = still_authorised(&tx, dev)? {
                return Ok(Err(e));
            }
            let mut out = Vec::with_capacity(req.ops.len());
            for op in &req.ops {
                let outcome = self.control_op(&tx, dev, op, hold, now)?;
                out.push(ControlResult {
                    id: op.id(),
                    outcome,
                });
            }
            if self.take_fault(|f| &mut f.fail_before_commit) {
                // Dropping the transaction rolls everything back (T-J1).
                return Ok(Ok(None));
            }
            tx.commit()?;
            Ok(Ok(Some(out)))
        })()
        .map_err(|_| WireError::CannotRecord)??
        .ok_or(WireError::CannotRecord)?;
        if req
            .ops
            .iter()
            .any(|o| matches!(o, ControlOp::Delete { .. }))
        {
            // Overwrite deleted content in the WAL as well (8.4).
            let _ = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
        }
        if self.take_fault(|f| &mut f.crash_after_commit) {
            // T-J2: committed, not mirrored, not acknowledged.
            return Err(WireError::CannotRecord);
        }
        let head = if hold {
            None
        } else {
            if !self.mirror_sync(&conn, &dev.user_id) {
                return Err(WireError::CannotRecord);
            }
            self.mirrored_head(&conn, &dev.user_id)
        };
        logging::log(
            "control",
            &[
                ("user", V::Id(&dev.user_id)),
                ("device", V::Id(&dev.device_ref)),
                ("count", V::N(results.len() as u64)),
            ],
        );
        Ok(ControlResponse { results, head })
    }

    // ------------------------------------------------------------- uploads

    #[allow(clippy::too_many_arguments)]
    fn check_row(
        tx: &Transaction<'_>,
        dev: &AuthedDevice,
        row: &UploadRow,
        confirmed_on: Option<NaiveDate>,
        hold: bool,
        today: NaiveDate,
        hwm: u64,
    ) -> rusqlite::Result<Result<(NaiveDate, String), WireError>> {
        if row.origin_device.as_bytes() != &dev.origin_device {
            return Ok(Err(WireError::OriginMismatch));
        }
        let Some(date) = parse_date(&row.local_date) else {
            return Ok(Err(WireError::SchemaInvalid));
        };
        if confirmed_on.is_none_or(|c| date < c) {
            return Ok(Err(WireError::BeforeShareFrom));
        }
        if hold {
            return Ok(Err(WireError::RecoveryHold));
        }
        let p = read_policy(tx, &dev.user_id, row.category)?;
        if !p.enabled {
            return Ok(Err(WireError::CategoryOff));
        }
        // Only what the mirror holds authorises: a live change newer than `mirror_hwm`
        // counts only where it narrows sharing.
        let mut share_from = p.share_from;
        if p.seq > hwm {
            let (on, from) = durable_policy(tx, &dev.user_id, row.category, hwm)?;
            if !on {
                return Ok(Err(WireError::CategoryOff));
            }
            share_from = share_from.max(from);
        }
        if share_from.is_none_or(|f| date < f) {
            return Ok(Err(WireError::BeforeShareFrom));
        }
        let tombstoned: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM tombstones WHERE user_id = ?1 AND category = ?2
                 AND ?3 BETWEEN from_date AND to_date)",
            params![
                dev.user_id.as_slice(),
                row.category.as_str(),
                row.local_date
            ],
            |r| r.get(0),
        )?;
        if tombstoned {
            return Ok(Err(WireError::Tombstoned));
        }
        let latest = today.checked_add_days(Days::new(1)).unwrap_or(today);
        let earliest = today
            .checked_sub_days(Days::new(RETENTION_DAYS))
            .unwrap_or(today);
        if date > latest || date < earliest {
            return Ok(Err(WireError::DateOutOfRange));
        }
        let Ok(canonical) = share::validate(row.category, &row.payload) else {
            return Ok(Err(WireError::SchemaInvalid));
        };
        let text = canonical.to_string();
        if text.len() > MAX_PAYLOAD {
            return Ok(Err(WireError::SchemaInvalid));
        }
        let stored: Option<i64> = tx
            .query_row(
                "SELECT revision FROM summaries WHERE user_id = ?1 AND category = ?2
                 AND origin_device = ?3 AND local_date = ?4",
                params![
                    dev.user_id.as_slice(),
                    row.category.as_str(),
                    dev.origin_device.as_slice(),
                    row.local_date
                ],
                |r| r.get(0),
            )
            .optional()?;
        if stored.is_some_and(|s| s >= i64::from(row.revision)) {
            return Ok(Err(WireError::RevisionNotHigher));
        }
        Ok(Ok((date, text)))
    }

    /// `POST /v1/upload` (contract 8.6): each row is accepted only if every check holds.
    ///
    /// # Errors
    /// `schema_invalid` for an empty or oversized batch; `cannot_record` on storage
    /// failure (nothing is half-applied).
    pub fn upload(
        &self,
        dev: &AuthedDevice,
        req: &UploadRequest,
    ) -> Result<UploadResponse, WireError> {
        if req.rows.is_empty() || req.rows.len() > MAX_UPLOAD_ROWS {
            return Err(WireError::SchemaInvalid);
        }
        let now = self.now_ms();
        let mut conn = self.db();
        let results = (|| -> rusqlite::Result<Result<Vec<RowOutcome>, WireError>> {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            if let Err(e) = still_authorised(&tx, dev)? {
                return Ok(Err(e));
            }
            let (confirmed_on, hold): (Option<String>, bool) = tx.query_row(
                "SELECT d.confirmed_on, u.hold FROM devices d JOIN users u ON u.user_id = d.user_id
                 WHERE d.device_ref = ?1",
                [dev.device_ref.as_slice()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            let hwm = Self::mirrored_seq(&tx, &dev.user_id);
            // The confirmation itself must be mirrored before the device may upload.
            let confirmed_on = confirmed_on
                .as_deref()
                .and_then(parse_date)
                .filter(|_| {
                    confirmation_mirrored(&tx, &dev.user_id, &dev.device_ref, hwm)
                        .unwrap_or(false)
                });
            let today = Self::user_zone(&tx, &dev.user_id)?.local_date(self.clock.now());
            let mut out = Vec::with_capacity(req.rows.len());
            for row in &req.rows {
                match Self::check_row(&tx, dev, row, confirmed_on, hold, today, hwm)? {
                    Ok((date, payload)) => {
                        tx.execute(
                            "INSERT INTO summaries (user_id, category, origin_device, local_date,
                                 revision, payload, device_ref, received_at)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                             ON CONFLICT (user_id, category, origin_device, local_date) DO UPDATE SET
                                 revision = excluded.revision, payload = excluded.payload,
                                 device_ref = excluded.device_ref, received_at = excluded.received_at",
                            params![
                                dev.user_id.as_slice(),
                                row.category.as_str(),
                                dev.origin_device.as_slice(),
                                date_text(date),
                                row.revision,
                                payload,
                                dev.device_ref.as_slice(),
                                now
                            ],
                        )?;
                        out.push(RowOutcome::Accepted);
                    }
                    Err(error) => out.push(RowOutcome::Rejected { error }),
                }
            }
            tx.commit()?;
            Ok(Ok(out))
        })()
        .map_err(|_| WireError::CannotRecord)??;
        let accepted = results
            .iter()
            .filter(|r| **r == RowOutcome::Accepted)
            .count();
        logging::log(
            "upload",
            &[
                ("user", V::Id(&dev.user_id)),
                ("device", V::Id(&dev.device_ref)),
                ("count", V::N(results.len() as u64)),
                ("accepted", V::N(accepted as u64)),
            ],
        );
        Ok(UploadResponse { results })
    }

    // ------------------------------------------------------------- history

    /// `GET /v1/history` ("Restore from server", contract 8.1): the user's daily
    /// summaries in the range, keyed by origin, read-only. Each read is a
    /// `history_access` entry in the user's chain, so the user can see it (T-S18).
    ///
    /// # Errors
    /// `schema_invalid` for a bad range; `recovery_hold` during hold; `cannot_record` on
    /// storage failure.
    pub fn history(
        &self,
        dev: &AuthedDevice,
        from: &str,
        to: &str,
    ) -> Result<HistoryResponse, WireError> {
        let (Some(f), Some(t)) = (parse_date(from), parse_date(to)) else {
            return Err(WireError::SchemaInvalid);
        };
        if f > t || (t - f).num_days() >= MAX_HISTORY_DAYS {
            return Err(WireError::SchemaInvalid);
        }
        let now = self.now_ms();
        let mut conn = self.db();
        // The read is a chain entry: nothing is answered unless the mirror can hold it.
        if !self.mirror_sync(&conn, &dev.user_id) {
            return Err(WireError::CannotRecord);
        }
        let res = (|| -> rusqlite::Result<Result<Vec<HistoryRow>, WireError>> {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            if let Err(e) = still_authorised(&tx, dev)? {
                return Ok(Err(e));
            }
            let hold: bool = tx.query_row(
                "SELECT hold FROM users WHERE user_id = ?1",
                [dev.user_id.as_slice()],
                |r| r.get(0),
            )?;
            if hold {
                return Ok(Err(WireError::RecoveryHold));
            }
            let rows = {
                let mut stmt = tx.prepare(
                    "SELECT category, origin_device, local_date, revision, payload FROM summaries
                     WHERE user_id = ?1 AND local_date BETWEEN ?2 AND ?3
                     ORDER BY local_date, category, origin_device",
                )?;
                stmt.query_map(params![dev.user_id.as_slice(), from, to], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        crate::db::id16(r.get(1)?)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, u32>(3)?,
                        r.get::<_, String>(4)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?
            };
            let rows = rows
                .into_iter()
                .filter_map(|(cat, origin, date, revision, payload)| {
                    Some(HistoryRow {
                        category: Category::ALL.iter().copied().find(|c| c.as_str() == cat)?,
                        origin_device: Uuid::from_bytes(origin),
                        local_date: date,
                        revision,
                        payload: serde_json::from_str(&payload).ok()?,
                    })
                })
                .collect();
            chain::append(
                &tx,
                &self.keys.key_id,
                &dev.user_id,
                u(now),
                EntryBody::HistoryAccess {
                    device_ref: dev.device_ref,
                },
            )?;
            audit::record(
                &tx,
                now,
                AuditEvent::HistoryRead,
                Some(&dev.user_id),
                Some(&dev.device_ref),
                None,
            )?;
            tx.commit()?;
            Ok(Ok(rows))
        })();
        let rows = res.map_err(|_| WireError::CannotRecord)??;
        if !self.mirror_sync(&conn, &dev.user_id) {
            return Err(WireError::CannotRecord);
        }
        Ok(HistoryResponse { rows })
    }

    /// The category code of a category (for logs).
    #[must_use]
    pub fn category_log_code(c: Category) -> u64 {
        u64::from(category_code(c))
    }
}
