//! The server side of the per-user control log (V1-5 contract 7.1 and 7.2).
//!
//! Every operation that changes a user's devices or shared data appends one entry to
//! that user's chain **in the same transaction** as its effect: callers pass the open
//! transaction. Sequence numbers come from the chain itself (the previous entry plus
//! one), never from wall time.
//!
//! After each commit the new entries are appended to the off-volume mirror
//! ([`crate::mirror`]) and `mirror_hwm` is recorded; a signed head never covers an entry
//! the mirror does not hold ([`head_at`]).

use hy_core::protocol::b64;
use hy_core::protocol::chain::{ControlEntry, EntryBody, genesis};
use hy_core::protocol::head::{Head, SignedHead};
use hy_core::protocol::wire::WireEntry;
use rusqlite::{Connection, OptionalExtension, params};

use crate::db::{h32, id16};
use crate::keys::ServerKeys;

/// The user's chain tail: `(seq, hash)`, or `(0, genesis)` for an empty chain.
///
/// # Errors
/// Any SQLite error.
pub fn tail(
    conn: &Connection,
    key_id: &[u8; 32],
    user: &[u8; 16],
) -> rusqlite::Result<(u64, [u8; 32])> {
    let row = conn
        .query_row(
            "SELECT seq, hash FROM control_log WHERE user_id = ?1 ORDER BY seq DESC LIMIT 1",
            [user.as_slice()],
            |r| Ok((r.get::<_, i64>(0)?, h32(r.get(1)?)?)),
        )
        .optional()?;
    Ok(match row {
        Some((seq, hash)) => (u64::try_from(seq).unwrap_or(0), hash),
        None => (0, genesis(key_id, user)),
    })
}

/// Append one entry to `user`'s chain and return it with its hash.
///
/// # Errors
/// Any SQLite error (the caller's transaction then rolls back).
pub fn append(
    conn: &Connection,
    key_id: &[u8; 32],
    user: &[u8; 16],
    at_ms: u64,
    body: EntryBody,
) -> rusqlite::Result<(ControlEntry, [u8; 32])> {
    let (seq, prev) = tail(conn, key_id, user)?;
    let entry = ControlEntry {
        user_id: *user,
        seq: seq + 1,
        at_ms,
        body,
    };
    let hash = entry.hash(&prev);
    conn.execute(
        "INSERT INTO control_log (user_id, seq, at_ms, kind, body, prev_hash, hash)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            user.as_slice(),
            i64::try_from(entry.seq).unwrap_or(i64::MAX),
            i64::try_from(at_ms).unwrap_or(i64::MAX),
            entry.body.kind(),
            entry.body.encode(),
            prev.as_slice(),
            hash.as_slice()
        ],
    )?;
    Ok((entry, hash))
}

/// The user's entries with seq greater than `after` and at most `upto`, oldest first, at
/// most `limit`.
///
/// # Errors
/// Any SQLite error, or a conversion error for a damaged row.
pub fn entries_after(
    conn: &Connection,
    user: &[u8; 16],
    after: u64,
    upto: u64,
    limit: usize,
) -> rusqlite::Result<Vec<WireEntry>> {
    let mut stmt = conn.prepare(
        "SELECT seq, at_ms, kind, body FROM control_log
         WHERE user_id = ?1 AND seq > ?2 AND seq <= ?4 ORDER BY seq LIMIT ?3",
    )?;
    stmt.query_map(
        params![
            user.as_slice(),
            i64::try_from(after).unwrap_or(i64::MAX),
            i64::try_from(limit).unwrap_or(i64::MAX),
            i64::try_from(upto).unwrap_or(i64::MAX)
        ],
        |r| {
            Ok(WireEntry {
                seq: u64::try_from(r.get::<_, i64>(0)?).unwrap_or(0),
                at_ms: u64::try_from(r.get::<_, i64>(1)?).unwrap_or(0),
                kind: r.get(2)?,
                body: b64(&r.get::<_, Vec<u8>>(3)?),
            })
        },
    )?
    .collect()
}

/// One stored entry with its hashes, as the mirror holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredEntry {
    /// The user.
    pub user_id: [u8; 16],
    /// Its seq.
    pub seq: u64,
    /// Server wall time (ms).
    pub at_ms: u64,
    /// The kind byte.
    pub kind: u8,
    /// The encoded body.
    pub body: Vec<u8>,
    /// The previous entry's hash (the genesis for seq 1).
    pub prev: [u8; 32],
    /// This entry's hash.
    pub hash: [u8; 32],
}

/// The user's stored entries with seq greater than `after`, oldest first.
///
/// # Errors
/// Any SQLite error, or a conversion error for a damaged row.
pub fn rows_after(
    conn: &Connection,
    user: &[u8; 16],
    after: u64,
) -> rusqlite::Result<Vec<StoredEntry>> {
    let mut stmt = conn.prepare(
        "SELECT seq, at_ms, kind, body, prev_hash, hash FROM control_log
         WHERE user_id = ?1 AND seq > ?2 ORDER BY seq",
    )?;
    stmt.query_map(
        params![user.as_slice(), i64::try_from(after).unwrap_or(i64::MAX)],
        |r| {
            Ok(StoredEntry {
                user_id: *user,
                seq: u64::try_from(r.get::<_, i64>(0)?).unwrap_or(0),
                at_ms: u64::try_from(r.get::<_, i64>(1)?).unwrap_or(0),
                kind: r.get(2)?,
                body: r.get(3)?,
                prev: h32(r.get(4)?)?,
                hash: h32(r.get(5)?)?,
            })
        },
    )?
    .collect()
}

/// Insert an entry exactly as stored elsewhere (mirror replay after a restore, contract
/// 9). The caller has verified its chain and applies its effects.
///
/// # Errors
/// Any SQLite error.
pub fn insert_verbatim(conn: &Connection, e: &StoredEntry) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO control_log (user_id, seq, at_ms, kind, body, prev_hash, hash)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            e.user_id.as_slice(),
            i64::try_from(e.seq).unwrap_or(i64::MAX),
            i64::try_from(e.at_ms).unwrap_or(i64::MAX),
            e.kind,
            e.body,
            e.prev.as_slice(),
            e.hash.as_slice()
        ],
    )?;
    Ok(())
}

/// The hash of the user's entry at `seq` (the genesis for 0), if it exists.
///
/// # Errors
/// Any SQLite error.
pub fn hash_at(
    conn: &Connection,
    key_id: &[u8; 32],
    user: &[u8; 16],
    seq: u64,
) -> rusqlite::Result<Option<[u8; 32]>> {
    if seq == 0 {
        return Ok(Some(genesis(key_id, user)));
    }
    conn.query_row(
        "SELECT hash FROM control_log WHERE user_id = ?1 AND seq = ?2",
        params![user.as_slice(), i64::try_from(seq).unwrap_or(i64::MAX)],
        |r| h32(r.get(0)?),
    )
    .optional()
}

/// A signed head for the user's chain at `seq` (at most the tail).
///
/// # Errors
/// Any SQLite error.
pub fn head_at(
    conn: &Connection,
    keys: &ServerKeys,
    user: &[u8; 16],
    seq: u64,
    now_ms: u64,
) -> rusqlite::Result<SignedHead> {
    let (tail_seq, _) = tail(conn, &keys.key_id, user)?;
    let seq = seq.min(tail_seq);
    let chain_hash = hash_at(conn, &keys.key_id, user, seq)?.unwrap_or_default();
    Ok(Head {
        key_id: keys.key_id,
        user_id: *user,
        seq,
        chain_hash,
        issued_at_ms: now_ms,
    }
    .sign(keys.head_key()))
}

/// The user's current signed head.
///
/// # Errors
/// Any SQLite error.
pub fn signed_head(
    conn: &Connection,
    keys: &ServerKeys,
    user: &[u8; 16],
    now_ms: u64,
) -> rusqlite::Result<SignedHead> {
    let (seq, chain_hash) = tail(conn, &keys.key_id, user)?;
    Ok(Head {
        key_id: keys.key_id,
        user_id: *user,
        seq,
        chain_hash,
        issued_at_ms: now_ms,
    }
    .sign(keys.head_key()))
}

/// Recompute every user's chain from genesis and check each stored hash (startup and
/// tests). Returns the number of users checked.
///
/// # Errors
/// A conversion error naming the first broken chain.
pub fn verify_all(conn: &Connection, key_id: &[u8; 32]) -> rusqlite::Result<usize> {
    let users: Vec<[u8; 16]> = conn
        .prepare("SELECT user_id FROM users")?
        .query_map([], |r| id16(r.get(0)?))?
        .collect::<rusqlite::Result<_>>()?;
    for user in &users {
        let mut stmt = conn.prepare(
            "SELECT seq, at_ms, kind, body, prev_hash, hash FROM control_log WHERE user_id = ?1 ORDER BY seq",
        )?;
        let rows = stmt
            .query_map([user.as_slice()], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, u8>(2)?,
                    r.get::<_, Vec<u8>>(3)?,
                    h32(r.get(4)?)?,
                    h32(r.get(5)?)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut prev = genesis(key_id, user);
        for (expect_seq, (seq, at_ms, kind, body, prev_hash, hash)) in (1i64..).zip(rows) {
            let broken = || {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Blob,
                    "control log chain is broken".into(),
                )
            };
            let body = EntryBody::decode(kind, &body).map_err(|_| broken())?;
            if seq != expect_seq || prev_hash != prev {
                return Err(broken());
            }
            let entry = ControlEntry {
                user_id: *user,
                seq: u64::try_from(seq).map_err(|_| broken())?,
                at_ms: u64::try_from(at_ms).map_err(|_| broken())?,
                body,
            };
            if entry.hash(&prev) != hash {
                return Err(broken());
            }
            prev = hash;
        }
    }
    Ok(users.len())
}
