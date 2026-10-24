//! Sync on the device (V1-5 contract 7.4, 8.3 to 8.6, 9 and 12). Network steps live in
//! [`crate::link::flow`]; everything here is a synchronous local transaction.
//!
//! - **Chain effects.** [`apply_entries`] runs inside
//!   [`Store::apply_confirmed_state`], only after the head and the extension verified, in
//!   the same transaction as the receipt. A `policy` entry for one of this PC's own
//!   operations marks it accepted; one from another PC is applied at once (owner
//!   decision O2): an enable switches the category on from its `share_from` with a
//!   non-blocking notice and a `remote_enable_applied` consent entry, a disable switches
//!   it off and purges its queue. A `deletion` entry acknowledges this PC's own
//!   tombstone or inserts the other PC's as acknowledged (which purges the queue). A
//!   forged entry never gets here: the extension check refuses it first (T-S17).
//! - **Operations to send** ([`Store::pending_control_ops`]): pending disables, enables
//!   (with the policy seq the PC had when the user acted) and deletions, in the order the
//!   user made them. Results ([`Store::apply_control_results`]): `stale_enable` reverts
//!   the toggle with a notice; `accepted_pending_recovery` keeps the operation as
//!   `pending_recovery` (the server sequences it on release); `accepted` waits for its
//!   entry to arrive through the verified chain.
//! - **Uploads.** [`Store::queue_share_records`] builds this PC's records with the same
//!   serializer as the preview ([`Store::share_record`]); rows the server rejects are
//!   discarded, never re-stamped ([`Store::apply_upload_results`]).
//! - **Regression** ([`Store::regression_reset`]): every category off, the queue purged,
//!   pending enables cancelled, and the user asked to switch categories on again; the
//!   enables already in the chain are not re-applied by the next pull.

use chrono::{DateTime, NaiveDate, Utc};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{
    Store, date_text, insert_consent, parse_date, parse_uuid, read_policy, read_setting,
    sharing_event, to_ms,
};
use crate::error::{Error, Result};
use crate::ids::IdGenerator;
use crate::protocol::chain::{ControlEntry, EntryBody, from_day_number};
use crate::protocol::share;
use crate::protocol::wire::{
    ControlOp, ControlOutcome, ControlResult, DeviceInfo, HistoryRow, MAX_CONTROL_OPS, RowOutcome,
    WireError,
};
use crate::schema::{Category, ConsentKind, ImportProvenance, ReminderOutcome, RoutineOutcome};

/// The notice version recorded with `remote_enable_applied`.
pub const REMOTE_ENABLE_NOTICE: &str = "v1-5-remote-enable-1";
/// Days this PC looks back when building records on an ordinary sync.
pub const SHARE_LOOKBACK_DAYS: u64 = 7;
/// Days the re-send after a release from recovery hold looks back: the server's whole
/// retention window (contract 8.6), which is also how long this PC keeps the detailed
/// events the records are built from (architecture 6.1).
pub const RESEND_WINDOW_DAYS: u64 = 90;

const NOTICES: &str = "server_notices";
const STATUS: &str = "server_status";
const REVISIONS: &str = "share_revisions";
/// The "last sent" text that makes a day be sent again (after a release from hold).
const RESEND: &str = "resend";
/// Set on a release from hold: the next queuing pass reaches back
/// [`RESEND_WINDOW_DAYS`] instead of [`SHARE_LOOKBACK_DAYS`].
const RESEND_PENDING: &str = "share_resend_pending";
/// Set by a regression reset: the next verified pull applies no enable.
const REGRESSION_SUPPRESS: &str = "server_regression_suppress";
/// Notices kept at most (the oldest go first).
const MAX_NOTICES: usize = 20;
/// `store_meta` key: the latest date of tracker tallies that came from another PC's
/// backup. Tallies carry no origin, so no day up to it is shared as this PC's work
/// session (T-L7). Kept in `store_meta`, which a backup never carries.
pub(super) const FOREIGN_TALLIES: &str = "foreign_tallies_through";

/// What a notice is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeKind {
    /// Another of the user's PCs switched a category on (O2).
    RemoteEnable,
    /// Another of the user's PCs switched a category off.
    RemoteDisable,
    /// An enable from this PC was older than a disable elsewhere.
    StaleEnable,
    /// The server saw an older receipt: every category is off until switched on again.
    Regressed,
}

/// A one-line, non-blocking notice for the sharing page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerNotice {
    /// Its id (for dismissing).
    pub id: u64,
    /// What happened.
    pub kind: NoticeKind,
    /// The category, if any.
    pub category: Option<Category>,
    /// The label of the PC that did it, if known.
    pub from_label: Option<String>,
    /// The date it took effect (the server's `share_from` for an enable).
    pub date: Option<NaiveDate>,
}

/// The link's sync status, for the badge and the sharing page.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerStatus {
    /// The server is in recovery hold for this user: sharing paused.
    pub hold: bool,
    /// The server's history did not verify: uploads stop until it does.
    pub mismatch: bool,
    /// The last successful contact (ms).
    pub last_sync_ms: Option<i64>,
}

fn write_setting<T: Serialize + ?Sized>(
    conn: &Connection,
    key: &str,
    value: &T,
    now: DateTime<Utc>,
) -> Result<()> {
    conn.execute(
        "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        params![key, serde_json::to_string(value)?, to_ms(now)],
    )?;
    Ok(())
}

fn push_notice(
    conn: &Connection,
    now: DateTime<Utc>,
    kind: NoticeKind,
    category: Option<Category>,
    from_label: Option<String>,
    date: Option<NaiveDate>,
) -> Result<()> {
    let mut list: Vec<ServerNotice> = read_setting(conn, NOTICES)?.unwrap_or_default();
    // One notice per category and kind: a newer one replaces the older.
    list.retain(|n| !(n.kind == kind && n.category == category));
    let id = list.iter().map(|n| n.id).max().unwrap_or(0) + 1;
    list.push(ServerNotice {
        id,
        kind,
        category,
        from_label,
        date,
    });
    if list.len() > MAX_NOTICES {
        list.drain(..list.len() - MAX_NOTICES);
    }
    write_setting(conn, NOTICES, &list, now)
}

fn update_status(
    conn: &Connection,
    now: DateTime<Utc>,
    f: impl FnOnce(&mut ServerStatus),
) -> Result<()> {
    let mut s: ServerStatus = read_setting(conn, STATUS)?.unwrap_or_default();
    f(&mut s);
    write_setting(conn, STATUS, &s, now)
}

pub(super) fn record_hold(conn: &Connection, now: DateTime<Utc>) -> Result<()> {
    update_status(conn, now, |s| {
        s.hold = true;
        s.last_sync_ms = Some(to_ms(now));
    })
}

pub(super) fn record_mismatch(conn: &Connection, now: DateTime<Utc>) -> Result<()> {
    update_status(conn, now, |s| s.mismatch = true)
}

/// Apply the verified entries' effects (see the module documentation). `entries` are
/// those after the previous receipt; `devices` is the response's device list (labels for
/// the notices). Runs inside the caller's transaction.
#[allow(clippy::too_many_lines)]
pub(super) fn apply_entries(
    tx: &Connection,
    ids: &IdGenerator,
    device_id: Uuid,
    now: DateTime<Utc>,
    entries: &[ControlEntry],
    devices: &[DeviceInfo],
) -> Result<()> {
    let suppress = read_setting::<bool>(tx, REGRESSION_SUPPRESS)?.unwrap_or(false);
    let label_of = |d: &[u8; 16]| {
        devices
            .iter()
            .find(|i| i.device_ref.as_bytes() == d)
            .map(|i| i.label.clone())
    };
    // Only the last policy entry per category in this batch takes effect (a newly
    // confirmed PC receives the whole history at once); every entry still settles this
    // PC's own operations.
    let mut last_policy: Vec<(Category, usize)> = Vec::new();
    let mut last_discard: Option<usize> = None;
    // The seqs before this batch: a deletion later in the batch must not hide an
    // earlier policy entry.
    let mut before = Vec::new();
    for category in Category::ALL.iter().copied() {
        before.push((category, read_policy(tx, category)?.seq));
    }
    for (i, e) in entries.iter().enumerate() {
        match e.body {
            EntryBody::Policy {
                category, op_id, ..
            } => {
                let op = Uuid::from_bytes(op_id).to_string();
                tx.execute(
                    "UPDATE policy_ops SET state = 'accepted', server_seq = ?2, updated_at = ?3
                     WHERE op_id = ?1 AND state IN ('pending', 'pending_recovery')",
                    params![op, i64::try_from(e.seq).unwrap_or(i64::MAX), to_ms(now)],
                )?;
                last_policy.retain(|(c, _)| *c != category);
                last_policy.push((category, i));
            }
            EntryBody::Deletion {
                category,
                from,
                to,
                request_id,
                ..
            } => {
                let (Some(from), Some(to)) = (from_day_number(from), from_day_number(to)) else {
                    continue;
                };
                let id = Uuid::from_bytes(request_id).to_string();
                let seq = i64::try_from(e.seq).unwrap_or(i64::MAX);
                let mine = tx.execute(
                    "UPDATE tombstones SET state = 'acknowledged', server_seq = ?2
                     WHERE request_id = ?1 AND state <> 'acknowledged'",
                    params![id, seq],
                )?;
                tx.execute(
                    "DELETE FROM outbox WHERE category = ?1 AND local_date BETWEEN ?2 AND ?3",
                    params![category, date_text(from), date_text(to)],
                )?;
                if mine == 0 {
                    tx.execute(
                        "INSERT INTO tombstones (request_id, category, from_date, to_date, state, server_seq, created_at)
                         VALUES (?1, ?2, ?3, ?4, 'acknowledged', ?5, ?6)
                         ON CONFLICT (request_id) DO NOTHING",
                        params![id, category, date_text(from), date_text(to), seq, to_ms(now)],
                    )?;
                }
                tx.execute(
                    "UPDATE sharing_policy SET seq = max(seq, ?2), updated_at = ?3 WHERE category = ?1",
                    params![category, seq, to_ms(now)],
                )?;
            }
            EntryBody::RecoveryDiscard { .. } => {
                last_discard = Some(i);
                tx.execute(
                    "UPDATE sharing_policy SET enabled = 0, seq = max(seq, ?1), updated_at = ?2",
                    params![i64::try_from(e.seq).unwrap_or(i64::MAX), to_ms(now)],
                )?;
            }
            _ => {}
        }
    }
    for (category, idx) in last_policy {
        // The user's restored history was discarded after this entry: everything is
        // off until switched on again.
        if last_discard.is_some_and(|d| d > idx) {
            continue;
        }
        let e = &entries[idx];
        let EntryBody::Policy {
            enabled,
            share_from,
            op_id,
            device_ref,
            ..
        } = e.body
        else {
            continue;
        };
        let policy = read_policy(tx, category)?;
        let seq = i64::try_from(e.seq).unwrap_or(i64::MAX);
        let seen = before
            .iter()
            .find(|(c, _)| *c == category)
            .map_or(0, |(_, s)| *s);
        if e.seq <= seen {
            continue;
        }
        let op = Uuid::from_bytes(op_id);
        let own: Option<i64> = tx
            .query_row(
                "SELECT local_seq FROM policy_ops WHERE op_id = ?1",
                [op.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        // The user switched it off here after that operation (or has a disable still
        // to send): this PC stays off, and its disable will follow.
        let off_since: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM policy_ops WHERE category = ?1 AND kind = 'disable'
                 AND (state = 'pending' OR local_seq > ?2))",
            params![category, own.unwrap_or(i64::MAX)],
            |r| r.get(0),
        )?;
        let share_from = from_day_number(share_from);
        if enabled && !off_since && !suppress && share_from.is_some() {
            tx.execute(
                "UPDATE sharing_policy SET enabled = 1, share_from = ?2, seq = max(seq, ?3), updated_at = ?4
                 WHERE category = ?1",
                params![category, share_from.map(date_text), seq, to_ms(now)],
            )?;
            if own.is_some() {
                insert_consent(
                    tx,
                    now,
                    ConsentKind::Enabled,
                    Some(category),
                    None,
                    Some(op),
                )?;
            } else {
                insert_consent(
                    tx,
                    now,
                    ConsentKind::RemoteEnableApplied,
                    Some(category),
                    Some(REMOTE_ENABLE_NOTICE),
                    Some(op),
                )?;
                push_notice(
                    tx,
                    now,
                    NoticeKind::RemoteEnable,
                    Some(category),
                    label_of(&device_ref),
                    share_from,
                )?;
            }
            if !policy.enabled {
                sharing_event(tx, ids, device_id, now, category, true)?;
            }
        } else if !enabled {
            tx.execute(
                "UPDATE sharing_policy SET enabled = 0, seq = max(seq, ?2), updated_at = ?3 WHERE category = ?1",
                params![category, seq, to_ms(now)],
            )?;
            tx.execute("DELETE FROM outbox WHERE category = ?1", [category])?;
            if own.is_none() && policy.enabled {
                insert_consent(
                    tx,
                    now,
                    ConsentKind::Disabled,
                    Some(category),
                    None,
                    Some(op),
                )?;
                push_notice(
                    tx,
                    now,
                    NoticeKind::RemoteDisable,
                    Some(category),
                    label_of(&device_ref),
                    None,
                )?;
                sharing_event(tx, ids, device_id, now, category, false)?;
            }
        } else {
            tx.execute(
                "UPDATE sharing_policy SET seq = max(seq, ?2), updated_at = ?3 WHERE category = ?1",
                params![category, seq, to_ms(now)],
            )?;
        }
    }
    if suppress {
        tx.execute("DELETE FROM settings WHERE key = ?1", [REGRESSION_SUPPRESS])?;
    }
    let was_held = read_setting::<ServerStatus>(tx, STATUS)?.is_some_and(|s| s.hold);
    if was_held {
        // Released from a server restore: rows sent after the restored snapshot are no
        // longer on the server, so every day this PC shared within the server's retention
        // window is sent again with a higher revision (the revisions themselves are kept,
        // so they still increase). The snapshot can be up to 30 days old and the hold can
        // last 7 days, so the next pass reaches back the whole window, not the usual
        // lookback.
        write_setting(tx, RESEND_PENDING, &true, now)?;
        let mut revisions: std::collections::BTreeMap<String, (u32, String)> =
            read_setting(tx, REVISIONS)?.unwrap_or_default();
        for v in revisions.values_mut() {
            // Never equal to a real record, and non-empty so that a day emptied since
            // is still corrected.
            v.1 = RESEND.to_owned();
        }
        write_setting(tx, REVISIONS, &revisions, now)?;
    }
    update_status(tx, now, |s| {
        s.hold = false;
        s.mismatch = false;
        s.last_sync_ms = Some(to_ms(now));
    })
}

/// The record of a day with nothing in it, for categories this PC produces.
fn empty_record(category: Category) -> Option<serde_json::Value> {
    let v = match category {
        Category::Hydration => serde_json::to_value(share::Hydration {
            v: share::SCHEMA_VERSION,
            ml: 0,
            goal_ml: None,
            entries: 0,
            coverage: None,
        }),
        Category::PhoneReports => serde_json::to_value(share::PhoneReports {
            v: share::SCHEMA_VERSION,
            work: 0,
            personal: 0,
            unsure: 0,
        }),
        Category::Outcomes => serde_json::to_value(share::Outcomes {
            v: share::SCHEMA_VERSION,
            offered: 0,
            done: 0,
            snoozed: 0,
            skipped: 0,
            no_response: 0,
            routine: None,
        }),
        Category::WorkSession => serde_json::to_value(share::WorkSession {
            v: share::SCHEMA_VERSION,
            active_min: 0,
            longest_block_min: 0,
            focus_count: 0,
            pomodoro_count: 0,
        }),
        Category::PostureSummary => return None,
    };
    v.ok().filter(|v| share::validate(category, v).is_ok())
}

const fn routine_result(o: RoutineOutcome) -> share::RoutineResult {
    match o {
        RoutineOutcome::Done => share::RoutineResult::Done,
        RoutineOutcome::Partial => share::RoutineResult::Partial,
        RoutineOutcome::None => share::RoutineResult::None,
        RoutineOutcome::Skipped => share::RoutineResult::Skipped,
    }
}

/// The reminder counts of one day on one PC, before bounding.
#[derive(Debug, Default)]
struct ReminderCounts {
    offered: u32,
    done: u32,
    snoozed: u32,
    skipped: u32,
    no_response: u32,
}

impl ReminderCounts {
    /// Bounded to the schema: at most [`share::MAX_DAY_COUNT`] offers, and the answers
    /// never more than the offers.
    fn bounded(&self, routine: Option<share::RoutineResult>) -> share::Outcomes {
        let offered = self.offered.min(share::MAX_DAY_COUNT);
        let mut left = offered;
        let mut take = |n: u32| {
            let n = n.min(left);
            left -= n;
            n
        };
        share::Outcomes {
            v: share::SCHEMA_VERSION,
            offered,
            done: take(self.done),
            snoozed: take(self.snoozed),
            skipped: take(self.skipped),
            no_response: take(self.no_response),
            routine,
        }
    }
}

impl Store {
    /// The link's sync status.
    ///
    /// # Errors
    /// [`Error::Sqlite`] or [`Error::Json`] on a read failure.
    pub fn server_status(&self) -> Result<ServerStatus> {
        Ok(self.setting(STATUS)?.unwrap_or_default())
    }

    /// The sharing notices not yet dismissed, oldest first.
    ///
    /// # Errors
    /// [`Error::Sqlite`] or [`Error::Json`] on a read failure.
    pub fn server_notices(&self) -> Result<Vec<ServerNotice>> {
        Ok(self.setting(NOTICES)?.unwrap_or_default())
    }

    /// Dismiss a notice.
    ///
    /// # Errors
    /// [`Error::Sqlite`] on a write failure.
    pub fn dismiss_server_notice(&self, id: u64) -> Result<()> {
        let mut list = self.server_notices()?;
        list.retain(|n| n.id != id);
        self.put_setting(NOTICES, &list)
    }

    /// Whether queued rows may be uploaded now: the link is confirmed and its last state
    /// verified (no history mismatch, no recovery hold).
    ///
    /// # Errors
    /// [`Error::Sqlite`] on a read failure.
    pub fn uploads_allowed(&self) -> Result<bool> {
        let Some(link) = self.server_link()? else {
            return Ok(false);
        };
        let status = self.server_status()?;
        Ok(link.confirmed && !status.mismatch && !status.hold)
    }

    /// The control operations to send, in the order the user made them (at most one
    /// batch).
    ///
    /// # Errors
    /// [`Error::Sqlite`] on a read failure.
    pub fn pending_control_ops(&self) -> Result<Vec<ControlOp>> {
        let mut ops: Vec<(i64, ControlOp)> = Vec::new();
        let policy: Vec<(String, Category, String, i64, i64)> = self
            .conn
            .prepare(
                "SELECT op_id, category, kind, base_seq, created_at FROM policy_ops
                 WHERE state = 'pending' AND link_device = (SELECT device_ref FROM server_link)
                 ORDER BY local_seq",
            )?
            .query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        for (id, category, kind, base_seq, at) in policy {
            let op_id = parse_uuid(&id)?;
            ops.push((
                at,
                if kind == "enable" {
                    ControlOp::Enable {
                        op_id,
                        category,
                        base_seq: u64::try_from(base_seq).unwrap_or(0),
                    }
                } else {
                    ControlOp::Disable { op_id, category }
                },
            ));
        }
        let deletions: Vec<(String, Category, String, String, i64)> = self
            .conn
            .prepare(
                "SELECT request_id, category, from_date, to_date, created_at FROM tombstones
                 WHERE state = 'pending' AND link_device = (SELECT device_ref FROM server_link)
                 ORDER BY created_at, rowid",
            )?
            .query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        for (id, category, from, to, at) in deletions {
            ops.push((
                at,
                ControlOp::Delete {
                    request_id: parse_uuid(&id)?,
                    category,
                    from,
                    to,
                },
            ));
        }
        // A stable sort keeps the policy order among equal times.
        ops.sort_by_key(|(at, _)| *at);
        ops.truncate(MAX_CONTROL_OPS);
        Ok(ops.into_iter().map(|(_, op)| op).collect())
    }

    /// Record the server's answers to a control batch.
    ///
    /// # Errors
    /// [`Error::Sqlite`] on a write failure, in which case nothing changed.
    pub fn apply_control_results(&mut self, results: &[ControlResult]) -> Result<()> {
        let now = self.clock.now();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        for r in results {
            let id = r.id.to_string();
            let op: Option<(Category, String)> = tx
                .query_row(
                    "SELECT category, kind FROM policy_ops WHERE op_id = ?1 AND state = 'pending'",
                    [&id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            match (r.outcome, op) {
                (ControlOutcome::StaleEnable { .. }, Some((category, _)))
                | (
                    ControlOutcome::Rejected {
                        error: WireError::StaleEnable | WireError::SchemaInvalid,
                    },
                    Some((category, _)),
                ) => {
                    tx.execute(
                        "UPDATE policy_ops SET state = 'rejected_stale', updated_at = ?2 WHERE op_id = ?1",
                        params![id, to_ms(now)],
                    )?;
                    insert_consent(
                        &tx,
                        now,
                        ConsentKind::StaleEnableRejected,
                        Some(category),
                        None,
                        Some(r.id),
                    )?;
                    push_notice(
                        &tx,
                        now,
                        NoticeKind::StaleEnable,
                        Some(category),
                        None,
                        None,
                    )?;
                }
                (ControlOutcome::AcceptedPendingRecovery, Some(_)) => {
                    tx.execute(
                        "UPDATE policy_ops SET state = 'pending_recovery', updated_at = ?2 WHERE op_id = ?1",
                        params![id, to_ms(now)],
                    )?;
                }
                (ControlOutcome::AcceptedPendingRecovery, None) => {
                    tx.execute(
                        "UPDATE tombstones SET state = 'pending_recovery' WHERE request_id = ?1 AND state = 'pending'",
                        [&id],
                    )?;
                }
                // Accepted: settled when its entry arrives through the verified chain.
                // Held enables stay pending and go again after release.
                _ => {}
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Remove sent rows from the queue: accepted rows, and rejected rows, which are
    /// discarded and never re-stamped (8.6). `ids` are the outbox ids in the order sent.
    ///
    /// # Errors
    /// [`Error::Invalid`] if the counts differ; [`Error::Sqlite`] on a write failure.
    pub fn apply_upload_results(&mut self, ids: &[i64], results: &[RowOutcome]) -> Result<()> {
        if ids.len() != results.len() {
            return Err(Error::invalid("one result per row"));
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        for id in ids {
            tx.execute("DELETE FROM outbox WHERE id = ?1", [id])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// `device_regressed` (contract 7.4): every category off, the queue purged, pending
    /// enables cancelled, a `regression_reset` consent entry and a notice asking the user
    /// to switch each category on again. Policy entries the server already holds are not
    /// re-applied by the next pull.
    ///
    /// # Errors
    /// [`Error::Sqlite`] on a write failure, in which case nothing changed.
    pub fn regression_reset(&mut self) -> Result<()> {
        let now = self.clock.now();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "UPDATE sharing_policy SET enabled = 0, updated_at = ?1 WHERE enabled = 1",
            [to_ms(now)],
        )?;
        tx.execute("DELETE FROM outbox", [])?;
        tx.execute(
            "UPDATE policy_ops SET state = 'rejected_stale', updated_at = ?1
             WHERE kind = 'enable' AND state = 'pending'",
            [to_ms(now)],
        )?;
        // The entries the next pull brings are history this copy cannot judge: no enable
        // among them is applied (the flag is cleared after that pull).
        write_setting(&tx, REGRESSION_SUPPRESS, &true, now)?;
        insert_consent(&tx, now, ConsentKind::RegressionReset, None, None, None)?;
        push_notice(&tx, now, NoticeKind::Regressed, None, None, None)?;
        tx.commit()?;
        Ok(())
    }

    /// The record this PC would send for a category and date, in the category's schema
    /// (the preview, SHR-01, renders exactly this). Only entries recorded on this PC
    /// count: rows imported from another PC's backup are never sent under this PC's id.
    /// `None` when nothing was recorded, and always for `posture_summary` (V2).
    ///
    /// - `hydration` and `phone_reports`: this PC's current entries of the day.
    /// - `outcomes`: the reminders this PC offered that day with their answers (see
    ///   [`Store::reminder_counts`]) and the day's best routine result.
    /// - `work_session`: the tracker's tallies of the day, in whole minutes; none for a
    ///   day whose tallies may have come from another PC's backup ([`FOREIGN_TALLIES`]).
    ///
    /// # Errors
    /// [`Error::Sqlite`] on a read failure.
    pub fn share_record(
        &self,
        category: Category,
        date: NaiveDate,
    ) -> Result<Option<serde_json::Value>> {
        let Some(f) = self
            .day_facts_for(date, date, Some(self.device_id))?
            .into_iter()
            .next()
        else {
            return Ok(None);
        };
        let value = match category {
            Category::Hydration => {
                if f.water_entries == 0 {
                    return Ok(None);
                }
                serde_json::to_value(share::Hydration {
                    v: share::SCHEMA_VERSION,
                    ml: f.water_ml.min(share::MAX_WATER_ML),
                    goal_ml: f
                        .water_goal_ml
                        .filter(|g| *g > 0 && *g <= share::MAX_WATER_ML),
                    entries: f.water_entries.min(share::MAX_DAY_COUNT),
                    coverage: None,
                })?
            }
            Category::PhoneReports => {
                let p = f.phone;
                if p.work + p.personal + p.unsure == 0 {
                    return Ok(None);
                }
                serde_json::to_value(share::PhoneReports {
                    v: share::SCHEMA_VERSION,
                    work: p.work.min(share::MAX_DAY_COUNT),
                    personal: p.personal.min(share::MAX_DAY_COUNT),
                    unsure: p.unsure.min(share::MAX_DAY_COUNT),
                })?
            }
            Category::Outcomes => {
                let counts = self.reminder_counts(date)?;
                let routine = f.routine.map(routine_result);
                if counts.offered == 0 && routine.is_none() {
                    return Ok(None);
                }
                serde_json::to_value(counts.bounded(routine))?
            }
            Category::WorkSession => {
                // Tallies from another PC's backup are never sent as this PC's.
                let foreign: Option<String> = self
                    .conn
                    .query_row(
                        "SELECT value FROM store_meta WHERE key = ?1",
                        [FOREIGN_TALLIES],
                        |r| r.get(0),
                    )
                    .optional()?;
                if foreign.is_some_and(|d| date_text(date) <= d) {
                    return Ok(None);
                }
                let Some(t) = f.tally else {
                    return Ok(None);
                };
                let minutes = |s: u64| u32::try_from(s / 60).unwrap_or(u32::MAX);
                // Work time includes quiet time counted as work (an estimate,
                // architecture 4.3), which is also what the longest block is made of.
                let active =
                    minutes(t.work_active_s.saturating_add(t.work_quiet_s)).min(share::DAY_MINUTES);
                let focus = t.focus_sessions.min(share::MAX_DAY_COUNT);
                let pomodoro = t.pomodoro_rounds.min(share::MAX_DAY_COUNT);
                if active == 0 && focus == 0 && pomodoro == 0 {
                    return Ok(None);
                }
                serde_json::to_value(share::WorkSession {
                    v: share::SCHEMA_VERSION,
                    active_min: active,
                    longest_block_min: minutes(t.longest_block_s).min(active),
                    focus_count: focus,
                    pomodoro_count: pomodoro,
                })?
            }
            // V2: no posture data exists in V1.
            Category::PostureSummary => return Ok(None),
        };
        Ok(share::validate(category, &value).ok())
    }

    /// The reminders this PC offered on `date`, every activity, each counted with its
    /// answer, if any: "done" and "started" as done (a started routine's result is the
    /// record's `routine`), "skipped" and "not now" as skipped. An answer counts on its
    /// offer's date, so answers never outnumber offers.
    fn reminder_counts(&self, date: NaiveDate) -> Result<ReminderCounts> {
        let mut stmt = self.conn.prepare(
            "SELECT json_extract(r.payload, '$.outcome') FROM events o
             LEFT JOIN events r ON r.type = 'reminder.response'
                 AND r.origin_device = o.origin_device
                 AND json_extract(r.payload, '$.reminder_id') = json_extract(o.payload, '$.reminder_id')
             WHERE o.type = 'reminder.offered' AND o.local_date = ?1 AND o.origin_device = ?2",
        )?;
        let answers = stmt
            .query_map(params![date_text(date), self.device_id.to_string()], |r| {
                r.get::<_, Option<String>>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut c = ReminderCounts::default();
        for a in answers {
            c.offered = c.offered.saturating_add(1);
            let slot = match a.and_then(|t| t.parse::<ReminderOutcome>().ok()) {
                Some(ReminderOutcome::Done | ReminderOutcome::Started) => &mut c.done,
                Some(ReminderOutcome::Snoozed) => &mut c.snoozed,
                Some(ReminderOutcome::Skipped | ReminderOutcome::Dismissed) => &mut c.skipped,
                Some(ReminderOutcome::NoResponse) => &mut c.no_response,
                // Not answered yet (or an answer this build does not know).
                None => continue,
            };
            *slot = slot.saturating_add(1);
        }
        Ok(c)
    }

    /// Queue this PC's changed records for the shareable days (on or after
    /// `share_from`, this PC's confirmation date, and the last
    /// [`SHARE_LOOKBACK_DAYS`] days). Returns how many were queued.
    ///
    /// The first pass after a release from recovery hold reaches back
    /// [`RESEND_WINDOW_DAYS`] (the server's retention window). Beyond the usual lookback
    /// it rebuilds only the days this PC had shared before (those in its revision record,
    /// which is kept for the same window): it restores what the server held, and never
    /// backfills a day that was never shared. Consent, `share_from`, the confirmation
    /// date and tombstones bound it exactly as they bound an ordinary pass.
    ///
    /// # Errors
    /// [`Error::Sqlite`] on a read or write failure.
    pub fn queue_share_records(&mut self, today: NaiveDate) -> Result<usize> {
        let Some(link) = self.server_link()? else {
            return Ok(0);
        };
        let (true, Some(confirmed_on)) = (link.confirmed, link.confirmed_on) else {
            return Ok(0);
        };
        if self.server_status()?.mismatch {
            return Ok(0);
        }
        let mut revisions: std::collections::BTreeMap<String, (u32, String)> =
            self.setting(REVISIONS)?.unwrap_or_default();
        let resend = self.setting::<bool>(RESEND_PENDING)?.unwrap_or(false);
        let recent = today
            .checked_sub_days(chrono::Days::new(SHARE_LOOKBACK_DAYS))
            .unwrap_or(today);
        let first_day = if resend {
            today
                .checked_sub_days(chrono::Days::new(RESEND_WINDOW_DAYS))
                .unwrap_or(today)
        } else {
            recent
        }
        .max(confirmed_on);
        let mut queued = 0;
        for category in Category::ALL.iter().copied() {
            let policy = self.sharing_policy(category)?;
            let (true, Some(from)) = (policy.enabled, policy.share_from) else {
                continue;
            };
            let mut day = first_day.max(from);
            while day <= today {
                let next = day.succ_opt();
                let key = format!("{}:{}", category.as_str(), date_text(day));
                if day < recent && !revisions.contains_key(&key) {
                    // Older than the usual lookback and never shared: not backfilled.
                    match next {
                        Some(n) => day = n,
                        None => break,
                    }
                    continue;
                }
                let (rev, last) = revisions.get(&key).cloned().unwrap_or((0, String::new()));
                // A day with nothing left (its last entry undone) is sent as an empty
                // record if, and only if, something was shared for it before: the
                // server's earlier total is corrected, and a never-recorded day still
                // sends nothing.
                let record = match self.share_record(category, day)? {
                    Some(r) => Some(r),
                    None if !last.is_empty() => empty_record(category),
                    None => None,
                };
                if let Some(record) = record {
                    let text = record.to_string();
                    if last != text {
                        match self.enqueue_upload(category, day, rev + 1, &record) {
                            Ok(_) => {
                                revisions.insert(key, (rev + 1, text));
                                queued += 1;
                            }
                            Err(Error::UploadBlocked(_)) => {}
                            Err(e) => return Err(e),
                        }
                    }
                }
                match next {
                    Some(n) => day = n,
                    None => break,
                }
            }
        }
        let cutoff = date_text(
            today
                .checked_sub_days(chrono::Days::new(RESEND_WINDOW_DAYS))
                .unwrap_or(today),
        );
        revisions.retain(|k, _| k.split_once(':').is_some_and(|(_, d)| d >= cutoff.as_str()));
        self.put_setting(REVISIONS, &revisions)?;
        if resend {
            self.conn
                .execute("DELETE FROM settings WHERE key = ?1", [RESEND_PENDING])?;
        }
        Ok(queued)
    }

    /// Store "Restore from server" rows as read-only imported summaries keyed by origin
    /// (architecture 5.4). Rows recorded by this PC are skipped (they are its own
    /// totals); a local backup import keeps precedence. Returns the rows stored.
    ///
    /// # Errors
    /// [`Error::Sqlite`] on a write failure.
    pub fn import_server_history(&self, rows: &[HistoryRow]) -> Result<usize> {
        let mut n = 0;
        for r in rows {
            if r.origin_device == self.device_id {
                continue;
            }
            let Ok(date) = parse_date(&r.local_date) else {
                continue;
            };
            if share::validate(r.category, &r.payload).is_err() {
                continue;
            }
            let s = super::DailySummary {
                category: r.category,
                origin_device: r.origin_device,
                period: date,
                revision: r.revision.max(1),
                coverage: None,
                payload: r.payload.clone(),
            };
            if self.upsert_imported_summary(&s, ImportProvenance::ServerRestore)? {
                n += 1;
            }
        }
        Ok(n)
    }

    /// Note a successful contact (for "last synced").
    ///
    /// # Errors
    /// [`Error::Sqlite`] on a write failure.
    pub fn note_sync(&self) -> Result<()> {
        let now = self.clock.now();
        update_status(&self.conn, now, |s| s.last_sync_ms = Some(to_ms(now)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reminder_counts_are_bounded_so_answers_never_outnumber_offers() {
        let c = ReminderCounts {
            offered: 900,
            done: 450,
            snoozed: 30,
            skipped: 300,
            no_response: 100,
        };
        let o = c.bounded(Some(share::RoutineResult::Partial));
        assert_eq!(o.offered, share::MAX_DAY_COUNT);
        assert_eq!(
            (o.done, o.snoozed, o.skipped, o.no_response),
            (450, 30, 20, 0)
        );
        let v = serde_json::to_value(o).unwrap();
        assert!(share::validate(Category::Outcomes, &v).is_ok());
        // The empty records validate too (sent only to correct a day shared before).
        for c in [Category::Outcomes, Category::WorkSession] {
            assert!(empty_record(c).is_some(), "{c:?}");
        }
        assert!(empty_record(Category::PostureSummary).is_none());
    }
}
