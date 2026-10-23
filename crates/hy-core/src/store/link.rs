//! The device side of the office-server link (V1-5 contract 4.2 steps 7 and 10, 6.3, 7.4
//! and 13).
//!
//! - [`Store::save_pairing`] stores the link pending, keeps every category off and writes
//!   the `paired` consent entry, in one transaction. No receipt is stored yet.
//! - [`Store::apply_confirmed_state`] runs only on a `confirmed` state response. It
//!   verifies that the chain entries extend this PC's receipt (or the genesis, the first
//!   time) exactly to the signed head, and that the head is signed by the head key stored
//!   at pairing for this user. Only then, in one transaction, does it record the
//!   confirmation, replace the receipt and cache the device list. Any failure applies
//!   nothing ("server history mismatch").
//! - [`Store::unpair_locally`] removes the link and, through the schema's triggers, the
//!   receipt, the device cache and the whole upload queue, switches every category off
//!   and writes `unpaired`, in one transaction. It runs on unpair, on `device_revoked`
//!   and when the token can no longer be opened (re-pair needed).
//!
//! The token itself is kept as an opaque blob sealed by a [`crate::secret::TokenVault`]
//! (DPAPI user scope on Windows). The store never sees it in the clear.

use chrono::NaiveDate;
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use uuid::Uuid;

use super::{Store, date_text, from_ms, insert_consent, parse_date, parse_uuid, to_ms};
use crate::error::{Error, Result};
use crate::protocol::chain::{ControlEntry, EntryBody, genesis, verify_extension};
use crate::protocol::head::SignedHead;
use crate::protocol::wire::{DeviceInfo, MAX_LABEL, StateResponse};
use crate::protocol::{key_id, unb64};
use crate::schema::ConsentKind;

/// What [`Store::save_pairing`] records (contract 4.2 step 7). Its `Debug` leaves out
/// the alias and the sealed token (contract 11).
#[derive(Clone)]
pub struct NewLink {
    /// SHA-256 of the server certificate's DER.
    pub pin: [u8; 32],
    /// The server's Ed25519 head-signing public key.
    pub head_key: [u8; 32],
    /// The slip's host.
    pub host: String,
    /// The slip's port.
    pub port: u16,
    /// The address that passed the pin.
    pub last_ip: Option<String>,
    /// The user's id on the server.
    pub user_id: Uuid,
    /// This device's reference on the server.
    pub device_ref: Uuid,
    /// The user's alias, for "Paired as ...".
    pub alias: String,
    /// The token, sealed by the vault.
    pub token_blob: Vec<u8>,
}

/// The stored link. Its `Debug` leaves out the alias and the sealed token.
#[derive(Clone, PartialEq, Eq)]
pub struct ServerLink {
    /// `SHA-256("HY-KEYID1\0" ‖ head_key)`.
    pub server_key_id: [u8; 32],
    /// The pin.
    pub pin: [u8; 32],
    /// The head-signing public key.
    pub head_key: [u8; 32],
    /// The slip's host.
    pub host: String,
    /// The slip's port.
    pub port: u16,
    /// The last address that passed the pin.
    pub last_ip: Option<String>,
    /// An address the user typed in settings.
    pub manual_address: Option<String>,
    /// The user's id.
    pub user_id: Uuid,
    /// This device's reference.
    pub device_ref: Uuid,
    /// The user's alias.
    pub alias: String,
    /// The sealed token.
    pub token_blob: Vec<u8>,
    /// Confirmed by the administrator and verified against a signed head.
    pub confirmed: bool,
    /// This PC's confirmation date (server date in the user's zone).
    pub confirmed_on: Option<NaiveDate>,
    /// When it paired.
    pub paired_at: chrono::DateTime<chrono::Utc>,
}

impl std::fmt::Debug for NewLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NewLink")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user_id", &self.user_id)
            .field("device_ref", &self.device_ref)
            .field("alias", &crate::secret::REDACTED)
            .field("token_blob", &crate::secret::REDACTED)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for ServerLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerLink")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user_id", &self.user_id)
            .field("device_ref", &self.device_ref)
            .field("alias", &crate::secret::REDACTED)
            .field("token_blob", &crate::secret::REDACTED)
            .field("confirmed", &self.confirmed)
            .field("confirmed_on", &self.confirmed_on)
            .finish_non_exhaustive()
    }
}

/// A cached entry of "PCs paired to you".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairedDevice {
    /// The device reference.
    pub device_ref: Uuid,
    /// Its label.
    pub label: String,
    /// When it paired (ms).
    pub paired_at_ms: i64,
    /// Last seen (ms).
    pub last_seen_ms: Option<i64>,
    /// Confirmed.
    pub confirmed: bool,
    /// The new-device notice was shown for it.
    pub notice_shown: bool,
}

/// What [`Store::apply_confirmed_state`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateOutcome {
    /// The extension and the head verified; the receipt is now at `seq`.
    Applied {
        /// The receipt's seq.
        seq: u64,
        /// True if this response confirmed the PC.
        newly_confirmed: bool,
        /// Devices of the user that appeared since the last response (for the notice).
        new_devices: Vec<Uuid>,
    },
    /// The chain or head did not verify. Nothing was applied: sharing stays paused and
    /// the receipt stays as it was (contract 7.4, "server history mismatch").
    HistoryMismatch,
    /// The server is in recovery hold for this user (contract 7.3): no entries and no
    /// head were sent. The receipt stays as it was and sharing is paused until release.
    Hold,
}

fn blob32(v: Vec<u8>) -> rusqlite::Result<[u8; 32]> {
    v.try_into().map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Blob,
            "expected 32 bytes".into(),
        )
    })
}

impl Store {
    /// The link, if this PC is paired.
    ///
    /// # Errors
    /// [`Error::Sqlite`] on a read failure.
    pub fn server_link(&self) -> Result<Option<ServerLink>> {
        let raw = self
            .conn
            .query_row(
                "SELECT server_key_id, pin, head_key, host, port, last_ip, manual_address, user_id,
                        device_ref, alias, token_blob, confirmed, confirmed_on, paired_at
                 FROM server_link WHERE id = 1",
                [],
                |r| {
                    Ok((
                        blob32(r.get(0)?)?,
                        blob32(r.get(1)?)?,
                        blob32(r.get(2)?)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, u16>(4)?,
                        r.get::<_, Option<String>>(5)?,
                        r.get::<_, Option<String>>(6)?,
                        r.get::<_, String>(7)?,
                        r.get::<_, String>(8)?,
                        r.get::<_, String>(9)?,
                        r.get::<_, Vec<u8>>(10)?,
                        r.get::<_, bool>(11)?,
                        r.get::<_, Option<String>>(12)?,
                        r.get::<_, i64>(13)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            server_key_id,
            pin,
            head_key,
            host,
            port,
            last_ip,
            manual_address,
            user,
            device,
            alias,
            token_blob,
            confirmed,
            confirmed_on,
            paired_at,
        )) = raw
        else {
            return Ok(None);
        };
        Ok(Some(ServerLink {
            server_key_id,
            pin,
            head_key,
            host,
            port,
            last_ip,
            manual_address,
            user_id: parse_uuid(&user)?,
            device_ref: parse_uuid(&device)?,
            alias,
            token_blob,
            confirmed,
            confirmed_on: confirmed_on.as_deref().map(parse_date).transpose()?,
            paired_at: from_ms(paired_at)?,
        }))
    }

    /// Store a new pairing, pending (contract 4.2 step 7). In one transaction: the link is
    /// stored unconfirmed, every category is switched off (purging the queue), and the
    /// pairing notice and a `paired` entry go in the consent log.
    ///
    /// # Errors
    /// [`Error::Invalid`] if this PC is already paired (unpair first) or a field is out
    /// of range; [`Error::Sqlite`] otherwise. Nothing is stored on any error.
    pub fn save_pairing(&mut self, link: &NewLink, notice_version: &str) -> Result<()> {
        if link.alias.is_empty() || link.alias.chars().count() > MAX_LABEL {
            return Err(Error::invalid("alias must be 1 to 40 characters"));
        }
        let now = self.clock.now();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let paired: bool = tx.query_row("SELECT EXISTS (SELECT 1 FROM server_link)", [], |r| {
            r.get(0)
        })?;
        if paired {
            return Err(Error::invalid("already paired; unpair first"));
        }
        tx.execute(
            "UPDATE sharing_policy SET enabled = 0, updated_at = ?1 WHERE enabled = 1",
            [to_ms(now)],
        )?;
        // A new link starts from nothing another link owned (also after a backup import
        // that brought such state without a link): no policy sequence or share dates,
        // no unsent policy operation, no sync status, notices or upload revisions.
        tx.execute("UPDATE sharing_policy SET share_from = NULL, seq = 0", [])?;
        tx.execute(
            "DELETE FROM policy_ops WHERE state IN ('pending', 'pending_recovery')",
            [],
        )?;
        tx.execute(
            "DELETE FROM settings WHERE key IN ('server_status', 'server_notices', 'share_revisions', 'server_regression_suppress')",
            [],
        )?;
        tx.execute(
            "INSERT INTO server_link (id, server_key_id, pin, head_key, host, port, last_ip,
                 manual_address, user_id, device_ref, alias, token_blob, confirmed,
                 confirmed_on, paired_at)
             VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, ?8, ?9, ?10, 0, NULL, ?11)",
            params![
                key_id(&link.head_key).to_vec(),
                link.pin.to_vec(),
                link.head_key.to_vec(),
                link.host,
                link.port,
                link.last_ip,
                link.user_id.to_string(),
                link.device_ref.to_string(),
                link.alias,
                link.token_blob,
                to_ms(now)
            ],
        )?;
        insert_consent(
            &tx,
            now,
            ConsentKind::NoticeShown,
            None,
            Some(notice_version),
            None,
        )?;
        insert_consent(
            &tx,
            now,
            ConsentKind::Paired,
            None,
            Some(notice_version),
            None,
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Remember the last address that passed the pin (contract 5, "Address").
    ///
    /// # Errors
    /// [`Error::Sqlite`] on a write failure.
    pub fn set_link_last_ip(&self, ip: &str) -> Result<()> {
        self.conn
            .execute("UPDATE server_link SET last_ip = ?1 WHERE id = 1", [ip])?;
        Ok(())
    }

    /// Set or clear the manual server address the user typed in settings.
    ///
    /// # Errors
    /// [`Error::Sqlite`] on a write failure.
    pub fn set_link_manual_address(&self, address: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE server_link SET manual_address = ?1 WHERE id = 1",
            [address],
        )?;
        Ok(())
    }

    /// The receipt: the highest valid signed head received.
    ///
    /// # Errors
    /// [`Error::Sqlite`] on a read failure.
    pub fn head_receipt(&self) -> Result<Option<SignedHead>> {
        let row = self
            .conn
            .query_row(
                "SELECT key_id, user_id, seq, chain_hash, issued_at, sig FROM head_receipt WHERE id = 1",
                [],
                |r| {
                    Ok((
                        blob32(r.get(0)?)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                        blob32(r.get(3)?)?,
                        r.get::<_, i64>(4)?,
                        r.get::<_, Vec<u8>>(5)?,
                    ))
                },
            )
            .optional()?;
        let Some((key_id, user, seq, chain_hash, issued_at, sig)) = row else {
            return Ok(None);
        };
        let sig: [u8; 64] = sig
            .try_into()
            .map_err(|_| Error::invalid("stored receipt signature length"))?;
        Ok(Some(SignedHead {
            head: crate::protocol::head::Head {
                key_id,
                user_id: *parse_uuid(&user)?.as_bytes(),
                seq: u64::try_from(seq).map_err(|_| Error::invalid("stored receipt seq"))?,
                chain_hash,
                issued_at_ms: u64::try_from(issued_at)
                    .map_err(|_| Error::invalid("stored receipt time"))?,
            },
            sig,
        }))
    }

    /// The cached device list, oldest pairing first.
    ///
    /// # Errors
    /// [`Error::Sqlite`] on a read failure.
    pub fn paired_devices(&self) -> Result<Vec<PairedDevice>> {
        let mut stmt = self.conn.prepare(
            "SELECT device_ref, label, paired_at, last_seen, confirmed, notice_shown
             FROM paired_devices ORDER BY paired_at, device_ref",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                    r.get::<_, bool>(4)?,
                    r.get::<_, bool>(5)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(
                |(id, label, paired_at_ms, last_seen_ms, confirmed, notice_shown)| {
                    Ok(PairedDevice {
                        device_ref: parse_uuid(&id)?,
                        label,
                        paired_at_ms,
                        last_seen_ms,
                        confirmed,
                        notice_shown,
                    })
                },
            )
            .collect()
    }

    /// Mark the new-device notice as shown for a device.
    ///
    /// # Errors
    /// [`Error::Sqlite`] on a write failure.
    pub fn mark_device_notice_shown(&self, device_ref: Uuid) -> Result<()> {
        self.conn.execute(
            "UPDATE paired_devices SET notice_shown = 1 WHERE device_ref = ?1",
            [device_ref.to_string()],
        )?;
        Ok(())
    }

    /// Apply a `confirmed` state response (contract 4.2 step 10 and 7.4). See the module
    /// documentation. The entries must run from this PC's receipt (or the genesis) to the
    /// head: a caller that received several pages joins them first. A `pending` response
    /// changes nothing and returns `Ok(None)`.
    ///
    /// # Errors
    /// [`Error::NotFound`] if this PC is not paired; [`Error::Sqlite`] on a write failure.
    pub fn apply_confirmed_state(&mut self, state: &StateResponse) -> Result<Option<StateOutcome>> {
        let out = self.apply_confirmed_checked(state)?;
        match out {
            Some(StateOutcome::HistoryMismatch) => {
                super::sync::record_mismatch(&self.conn, self.clock.now())?;
            }
            Some(StateOutcome::Hold) => super::sync::record_hold(&self.conn, self.clock.now())?,
            _ => {}
        }
        Ok(out)
    }

    fn apply_confirmed_checked(&mut self, state: &StateResponse) -> Result<Option<StateOutcome>> {
        let StateResponse::Confirmed {
            confirmed_on,
            devices,
            entries,
            head,
            hold,
            ..
        } = state
        else {
            return Ok(None);
        };
        if *hold && head.is_none() {
            return Ok(self.server_link()?.map(|_| StateOutcome::Hold));
        }
        let link = self
            .server_link()?
            .ok_or_else(|| Error::NotFound("server link".into()))?;
        let user = *link.user_id.as_bytes();
        let Ok(confirmed_on) = parse_date(confirmed_on) else {
            return Ok(Some(StateOutcome::HistoryMismatch));
        };
        // Outside a hold a confirmed response must carry a head.
        let Some(head) = head.as_deref().map(SignedHead::from_wire) else {
            return Ok(Some(StateOutcome::HistoryMismatch));
        };
        let Ok(head) = head else {
            return Ok(Some(StateOutcome::HistoryMismatch));
        };
        if head.verify(&link.head_key, &user).is_err() {
            return Ok(Some(StateOutcome::HistoryMismatch));
        }
        let mut decoded = Vec::with_capacity(entries.len());
        for e in entries {
            let Some(body) = unb64(&e.body, 256) else {
                return Ok(Some(StateOutcome::HistoryMismatch));
            };
            let Ok(body) = EntryBody::decode(e.kind, &body) else {
                return Ok(Some(StateOutcome::HistoryMismatch));
            };
            decoded.push(ControlEntry {
                user_id: user,
                seq: e.seq,
                at_ms: e.at_ms,
                body,
            });
        }
        let receipt = self.head_receipt()?;
        let (start_seq, start_hash) = match &receipt {
            Some(r) => (r.head.seq, r.head.chain_hash),
            None => (0, genesis(&link.server_key_id, &user)),
        };
        if head.head.seq < start_seq {
            return Ok(Some(StateOutcome::HistoryMismatch));
        }
        if verify_extension(
            &user,
            start_seq,
            start_hash,
            &decoded,
            head.head.seq,
            &head.head.chain_hash,
        )
        .is_err()
        {
            return Ok(Some(StateOutcome::HistoryMismatch));
        }
        // This PC's own confirmation must be in the chain.
        let me = *link.device_ref.as_bytes();
        let confirmed_in_chain = link.confirmed
            || decoded
                .iter()
                .any(|e| e.body == EntryBody::DeviceConfirmed { device_ref: me });
        if !confirmed_in_chain {
            return Ok(Some(StateOutcome::HistoryMismatch));
        }
        if devices
            .iter()
            .any(|d| d.label.is_empty() || d.label.chars().count() > MAX_LABEL)
        {
            return Ok(Some(StateOutcome::HistoryMismatch));
        }

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let newly_confirmed = !link.confirmed;
        if newly_confirmed {
            tx.execute(
                "UPDATE server_link SET confirmed = 1, confirmed_on = ?1 WHERE id = 1",
                [date_text(confirmed_on)],
            )?;
        }
        let h = &head.head;
        let seq = i64::try_from(h.seq).map_err(|_| Error::invalid("seq too large"))?;
        let issued = i64::try_from(h.issued_at_ms).map_err(|_| Error::invalid("time too large"))?;
        if receipt.is_some() {
            tx.execute(
                "UPDATE head_receipt SET seq = ?1, chain_hash = ?2, issued_at = ?3, sig = ?4 WHERE id = 1",
                params![seq, h.chain_hash.to_vec(), issued, head.sig.to_vec()],
            )?;
        } else {
            tx.execute(
                "INSERT INTO head_receipt (id, key_id, user_id, seq, chain_hash, issued_at, sig)
                 VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    h.key_id.to_vec(),
                    link.user_id.to_string(),
                    seq,
                    h.chain_hash.to_vec(),
                    issued,
                    head.sig.to_vec()
                ],
            )?;
        }
        let known: Vec<String> = {
            let mut stmt = tx.prepare("SELECT device_ref FROM paired_devices")?;
            stmt.query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?
        };
        let mut new_devices = Vec::new();
        let listed: Vec<String> = devices.iter().map(|d| d.device_ref.to_string()).collect();
        for k in &known {
            if !listed.contains(k) {
                tx.execute("DELETE FROM paired_devices WHERE device_ref = ?1", [k])?;
            }
        }
        for d in devices {
            let id = d.device_ref.to_string();
            let is_new = !known.contains(&id);
            // Devices already present when this PC was first confirmed are not "new".
            let notice_shown = !is_new || newly_confirmed || d.device_ref == link.device_ref;
            if is_new && !notice_shown {
                new_devices.push(d.device_ref);
            }
            upsert_device(&tx, d, notice_shown)?;
        }
        super::sync::apply_entries(
            &tx,
            &self.ids,
            self.device_id,
            self.clock.now(),
            &decoded,
            devices,
        )?;
        tx.commit()?;
        Ok(Some(StateOutcome::Applied {
            seq: h.seq,
            newly_confirmed,
            new_devices,
        }))
    }

    /// Remove the link and everything that belongs to it (contract 6.3, self-unpair and
    /// revoked device). In one transaction every category is switched off (the queue is
    /// purged), the link, receipt and device cache are deleted and `unpaired` is written.
    /// Returns false if this PC was not paired.
    ///
    /// # Errors
    /// [`Error::Sqlite`] on a write failure, in which case nothing changed.
    pub fn unpair_locally(&mut self) -> Result<bool> {
        let now = self.clock.now();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let paired: bool = tx.query_row("SELECT EXISTS (SELECT 1 FROM server_link)", [], |r| {
            r.get(0)
        })?;
        if !paired {
            return Ok(false);
        }
        tx.execute(
            "UPDATE sharing_policy SET enabled = 0, updated_at = ?1 WHERE enabled = 1",
            [to_ms(now)],
        )?;
        tx.execute("DELETE FROM server_link", [])?;
        tx.execute("DELETE FROM outbox", [])?;
        insert_consent(&tx, now, ConsentKind::Unpaired, None, None, None)?;
        tx.commit()?;
        Ok(true)
    }
}

fn upsert_device(tx: &rusqlite::Transaction<'_>, d: &DeviceInfo, notice_shown: bool) -> Result<()> {
    tx.execute(
        "INSERT INTO paired_devices (device_ref, label, paired_at, last_seen, confirmed, notice_shown)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (device_ref) DO UPDATE SET label = excluded.label,
             paired_at = excluded.paired_at, last_seen = excluded.last_seen,
             confirmed = excluded.confirmed",
        params![
            d.device_ref.to_string(),
            d.label,
            d.paired_at_ms,
            d.last_seen_ms,
            d.confirmed,
            notice_shown
        ],
    )?;
    Ok(())
}
