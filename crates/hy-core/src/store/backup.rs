//! Export and import of the full personal backup (architecture 8; wireframe W9).
//!
//! **What a backup holds:** settings (coaching, water, workweek, display, review
//! answers), the goals history, the pet and its edits, the event log, the tracker's
//! per-day tallies, the daily, monthly and imported summaries, the deletion tombstones
//! and the consent log. Every row keeps its own origin device.
//!
//! **What it leaves out:** this PC's identity (its origin-device id stays its own), the
//! coaching engine's working state (rebuilt from the store on the next start), the upload
//! queue, and pairing and sharing state (the server link, policy operations and the
//! sharing switches): after an import this PC is still unpaired and every category stays
//! off, so it must pair again before anything is shared.
//!
//! **Import replaces, never merges**, to avoid double counting: the backed-up tables are
//! emptied and refilled from the file in one transaction, which also drops the engine's
//! saved state so the app starts fresh from the imported data. Two kinds of record are
//! never lost: **tombstones are merged as a union** (a date range deleted on either side
//! stays deleted and is never uploaded again), and the consent log keeps its entries and
//! adds the file's. A file from a newer schema is refused. Nothing changes unless every
//! row is accepted by the schema's own checks and triggers.

use std::collections::BTreeSet;

use rusqlite::types::{Value, ValueRef};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params_from_iter};

use super::{Store, from_ms, parse_date};
use crate::backup::{Contents, KdfParams, Summary, Table};
use crate::error::{Error, Result};
use crate::migrations;
use crate::schema::ConsentKind;

/// How an import treats a table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Emptied, then filled from the file.
    Replace,
    /// The file's rows are added; rows already here stay (by primary key).
    UnionByKey,
    /// The file's entries are added unless the same entry is already here.
    UnionConsent,
}

/// The backed-up tables, in the order they are written and read back (an event's
/// correction comes after the event it corrects, a reminder's offer before its answer).
const TABLES: &[(&str, &str, Mode)] = &[
    ("settings", "key", Mode::Replace),
    ("goals_history", "kind, effective_from", Mode::Replace),
    ("pet_profile", "id", Mode::Replace),
    ("events", "rowid", Mode::Replace),
    ("activity_day", "local_date", Mode::Replace),
    (
        "daily_summary",
        "category, origin_device, local_date",
        Mode::Replace,
    ),
    (
        "monthly_summary",
        "category, origin_device, month",
        Mode::Replace,
    ),
    (
        "imported_summary",
        "category, origin_device, local_date",
        Mode::Replace,
    ),
    ("tombstones", "created_at, request_id", Mode::UnionByKey),
    ("consent_log", "id", Mode::UnionConsent),
];

/// The consent log's identifying columns (its `id` is local and never copied).
const CONSENT_COLUMNS: [&str; 5] = ["at", "kind", "category", "notice_version", "op_ref"];

fn columns(conn: &Connection, table: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info(\"{table}\")"))?;
    let names = stmt
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(names)
}

fn to_json(v: ValueRef<'_>) -> Result<serde_json::Value> {
    Ok(match v {
        ValueRef::Null => serde_json::Value::Null,
        ValueRef::Integer(i) => serde_json::Value::from(i),
        ValueRef::Real(f) => serde_json::Number::from_f64(f)
            .map(serde_json::Value::Number)
            .ok_or_else(|| Error::invalid("a stored number cannot be backed up"))?,
        ValueRef::Text(t) => serde_json::Value::String(
            std::str::from_utf8(t)
                .map_err(|_| Error::invalid("stored text is not UTF-8"))?
                .to_owned(),
        ),
        ValueRef::Blob(_) => return Err(Error::invalid("binary values are never backed up")),
    })
}

fn to_sql(v: &serde_json::Value) -> Result<Value> {
    Ok(match v {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Number(n) => {
            match n.as_i64() {
                Some(i) => Value::Integer(i),
                None => Value::Real(n.as_f64().ok_or_else(|| {
                    Error::Backup("a number in the backup is out of range".into())
                })?),
            }
        }
        serde_json::Value::String(s) => Value::Text(s.clone()),
        _ => {
            return Err(Error::Backup(
                "the backup holds a value of an unknown kind".into(),
            ));
        }
    })
}

fn quoted(names: &[String]) -> String {
    names
        .iter()
        .map(|c| format!("\"{c}\""))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Check one table of the file against this build's schema: a known table, known and
/// distinct columns, every row the right length.
fn check_table(conn: &Connection, name: &str, t: &Table) -> Result<Mode> {
    let Some(&(_, _, mode)) = TABLES.iter().find(|(n, _, _)| *n == name) else {
        return Err(Error::Backup(format!(
            "the backup holds an unknown table {name:?}"
        )));
    };
    let known: BTreeSet<String> = columns(conn, name)?.into_iter().collect();
    let mut seen = BTreeSet::new();
    for c in &t.columns {
        if !known.contains(c) || !seen.insert(c) {
            return Err(Error::Backup(format!(
                "the backup's {name} table has an unknown or repeated column {c:?}"
            )));
        }
    }
    if mode == Mode::UnionConsent
        && CONSENT_COLUMNS
            .iter()
            .any(|c| !seen.contains(&(*c).to_owned()))
    {
        return Err(Error::Backup(
            "the backup's consent log is incomplete".into(),
        ));
    }
    if t.rows.iter().any(|r| r.len() != t.columns.len()) {
        return Err(Error::Backup(format!(
            "a row of the backup's {name} table has the wrong number of values"
        )));
    }
    Ok(mode)
}

fn summary_of(conn: &Connection, exported_at: chrono::DateTime<chrono::Utc>) -> Result<Summary> {
    let count = |sql: &str| -> Result<usize> {
        let n: i64 = conn.query_row(sql, [], |r| r.get(0))?;
        Ok(usize::try_from(n).unwrap_or(0))
    };
    let (from, to): (Option<String>, Option<String>) = conn.query_row(
        "SELECT MIN(d), MAX(d) FROM (SELECT local_date AS d FROM events
                                     UNION ALL SELECT local_date FROM activity_day)",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok(Summary {
        exported_at,
        events: count("SELECT COUNT(*) FROM events")?,
        days: count("SELECT COUNT(*) FROM activity_day")?,
        from: from.as_deref().map(parse_date).transpose()?,
        to: to.as_deref().map(parse_date).transpose()?,
    })
}

impl Store {
    /// Whether anything was recorded yet (an event or a tracker tally).
    ///
    /// # Errors
    /// A read error.
    pub fn has_history(&self) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM events) OR EXISTS (SELECT 1 FROM activity_day)",
            [],
            |r| r.get(0),
        )?)
    }

    /// Everything a backup holds, read in one consistent snapshot.
    ///
    /// # Errors
    /// Any read error.
    pub fn backup_contents(&self) -> Result<Contents> {
        let tx = self.conn.unchecked_transaction()?;
        let mut tables = std::collections::BTreeMap::new();
        for (name, order, _) in TABLES {
            let cols = columns(&tx, name)?;
            let mut stmt = tx.prepare(&format!(
                "SELECT {} FROM \"{name}\" ORDER BY {order}",
                quoted(&cols)
            ))?;
            let mut rows = Vec::new();
            let mut q = stmt.query([])?;
            while let Some(r) = q.next()? {
                let row = (0..cols.len())
                    .map(|i| to_json(r.get_ref(i)?))
                    .collect::<Result<Vec<_>>>()?;
                rows.push(row);
            }
            tables.insert(
                (*name).to_owned(),
                Table {
                    columns: cols,
                    rows,
                },
            );
        }
        let created: Option<String> = tx
            .query_row(
                "SELECT value FROM store_meta WHERE key = 'created_at'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        Ok(Contents {
            format: crate::backup::FORMAT_VERSION,
            schema_version: migrations::latest_version(),
            app_version: crate::VERSION.to_owned(),
            exported_at: self.clock.now(),
            origin_device: self.device_id,
            store_created_at: created.and_then(|t| t.parse().ok()),
            tables,
        })
    }

    /// Write a backup file ("Export my data"), encrypted with `password`.
    ///
    /// # Errors
    /// [`Error::Invalid`] for a password that is too short, or any read error.
    pub fn export_backup(&self, password: &str) -> Result<(Vec<u8>, Summary)> {
        self.export_backup_with(password, KdfParams::DEFAULT)
    }

    /// [`Store::export_backup`] with a chosen key cost (tests use a cheap one).
    ///
    /// # Errors
    /// See [`Store::export_backup`].
    pub fn export_backup_with(
        &self,
        password: &str,
        params: KdfParams,
    ) -> Result<(Vec<u8>, Summary)> {
        crate::backup::check_password(password)?;
        let contents = self.backup_contents()?;
        let summary = summary_of(&self.conn, contents.exported_at)?;
        Ok((contents.seal(password, params)?, summary))
    }

    /// Replace this PC's data with a backup file's ("Import a backup"). A paired PC is
    /// unpaired in the same transaction and every category is switched off.
    ///
    /// # Errors
    /// [`Error::Backup`] for a wrong password, a damaged or newer file, or a row the
    /// schema refuses; nothing was changed.
    pub fn import_backup(&mut self, file: &[u8], password: &str) -> Result<Summary> {
        let contents = Contents::open(file, password)?;
        self.import_contents(&contents)
    }

    /// [`Store::import_backup`] for contents already decrypted.
    ///
    /// # Errors
    /// See [`Store::import_backup`].
    pub fn import_contents(&mut self, c: &Contents) -> Result<Summary> {
        if c.schema_version == 0 || c.schema_version > migrations::latest_version() {
            return Err(Error::Backup(
                "the backup was made by a newer version of HealthierYou; update first".into(),
            ));
        }
        let mut checked = Vec::new();
        for (name, t) in &c.tables {
            checked.push((name.as_str(), t, check_table(&self.conn, name, t)?));
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        // The engine starts fresh from the imported data; nothing queued is kept.
        tx.execute("DELETE FROM coach_state", [])?;
        tx.execute("DELETE FROM outbox", [])?;
        for (name, _, mode) in TABLES.iter().rev() {
            if *mode == Mode::Replace {
                tx.execute(&format!("DELETE FROM \"{name}\""), [])?;
            }
        }
        for (name, _, _) in TABLES {
            let Some((_, t, mode)) = checked.iter().find(|(n, _, _)| n == name) else {
                continue;
            };
            let sql = match mode {
                Mode::Replace => format!(
                    "INSERT INTO \"{name}\" ({}) VALUES ({})",
                    quoted(&t.columns),
                    vec!["?"; t.columns.len()].join(", ")
                ),
                Mode::UnionByKey => format!(
                    "INSERT INTO \"{name}\" ({}) VALUES ({}) ON CONFLICT DO NOTHING",
                    quoted(&t.columns),
                    vec!["?"; t.columns.len()].join(", ")
                ),
                Mode::UnionConsent => String::new(),
            };
            for row in &t.rows {
                let values = row.iter().map(to_sql).collect::<Result<Vec<_>>>()?;
                let done = if *mode == Mode::UnionConsent {
                    let get = |col: &str| {
                        t.columns
                            .iter()
                            .position(|c| c == col)
                            .map_or(Value::Null, |i| values[i].clone())
                    };
                    let v: Vec<Value> = CONSENT_COLUMNS.iter().map(|c| get(c)).collect();
                    tx.execute(
                        "INSERT INTO consent_log (at, kind, category, notice_version, op_ref)
                         SELECT ?1, ?2, ?3, ?4, ?5
                         WHERE NOT EXISTS (SELECT 1 FROM consent_log
                             WHERE at IS ?1 AND kind IS ?2 AND category IS ?3
                               AND notice_version IS ?4 AND op_ref IS ?5)",
                        params_from_iter(v.iter()),
                    )
                } else {
                    tx.execute(&sql, params_from_iter(values.iter()))
                };
                done.map_err(|e| {
                    Error::Backup(format!(
                        "a row of the backup's {name} table was refused: {e}"
                    ))
                })?;
            }
        }
        // The first day of use is the earlier of the two stores'.
        if let Some(theirs) = c.store_created_at {
            from_ms(theirs)?;
            tx.execute(
                "UPDATE store_meta SET value = ?1
                 WHERE key = 'created_at' AND CAST(value AS INTEGER) > ?2",
                rusqlite::params![theirs.to_string(), theirs],
            )?;
        }
        // Tracker tallies carry no origin. After another PC's backup, no day they cover
        // is ever shared as this PC's work session (T-L7); the mark lives in
        // `store_meta`, which no backup carries, and only ever moves later.
        if c.origin_device != self.device_id {
            let latest: Option<String> =
                tx.query_row("SELECT MAX(local_date) FROM activity_day", [], |r| r.get(0))?;
            if let Some(latest) = latest {
                tx.execute(
                    "INSERT INTO store_meta (key, value) VALUES (?1, ?2)
                     ON CONFLICT (key) DO UPDATE SET value = max(value, excluded.value)",
                    rusqlite::params![super::sync::FOREIGN_TALLIES, latest],
                )?;
            }
        }
        // The imported data replaces this PC's, so its server link no longer describes
        // it: unpair in the same transaction (the link's removal trigger clears the
        // receipt, the device cache and the queue) and switch every category off. The PC
        // pairs again from a new slip. A refused import rolls this back with the rest.
        let paired: bool = tx.query_row("SELECT EXISTS (SELECT 1 FROM server_link)", [], |r| {
            r.get(0)
        })?;
        let now = self.clock.now();
        tx.execute(
            "UPDATE sharing_policy SET enabled = 0, updated_at = ?1 WHERE enabled = 1",
            [super::to_ms(now)],
        )?;
        if paired {
            tx.execute("DELETE FROM server_link", [])?;
            super::insert_consent(&tx, now, ConsentKind::Unpaired, None, None, None)?;
        }
        let summary = summary_of(&tx, c.exported_at)?;
        tx.commit()?;
        Ok(summary)
    }
}
