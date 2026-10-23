//! Control-log entries and the per-user hash chain (contract 7.1).
//!
//! ```text
//! genesis(U)   = SHA-256("HY-CL1-GENESIS\0" ‖ key_id ‖ user_id)
//! entry_hash_n = SHA-256("HY-CL1\0" ‖ user_id[16] ‖ u64be(seq) ‖ u64be(at_ms) ‖ u8(kind)
//!                        ‖ u32be(len(body)) ‖ body ‖ prev_hash[32])
//! prev_hash_1  = genesis(U);  prev_hash_n = entry_hash_{n-1}
//! ```
//!
//! Bodies are fixed-width binary fields; dates are `u32be` days since 1970-01-01. No
//! entry holds a health payload, an alias, a label or an address: users and devices
//! appear only as random 128-bit ids.

use chrono::NaiveDate;

use super::{ds, sha256};
use crate::schema::Category;

/// Why a device was revoked (`revocation` body).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RevocationReason {
    /// The administrator revoked or rejected it.
    Admin,
    /// The device unpaired itself.
    SelfUnpair,
    /// The same PC paired again, or a clone was detected twice.
    Superseded,
    /// Pending for 7 days without confirmation.
    UnconfirmedExpired,
    /// The user's restored history was discarded (architecture 5.3).
    RecoveryDiscard,
}

impl RevocationReason {
    /// The wire byte.
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::Admin => 0,
            Self::SelfUnpair => 1,
            Self::Superseded => 2,
            Self::UnconfirmedExpired => 3,
            Self::RecoveryDiscard => 4,
        }
    }

    /// From the wire byte.
    #[must_use]
    pub const fn from_code(b: u8) -> Option<Self> {
        Some(match b {
            0 => Self::Admin,
            1 => Self::SelfUnpair,
            2 => Self::Superseded,
            3 => Self::UnconfirmedExpired,
            4 => Self::RecoveryDiscard,
            _ => return None,
        })
    }

    /// The stored text form.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::SelfUnpair => "self",
            Self::Superseded => "superseded",
            Self::UnconfirmedExpired => "unconfirmed_expired",
            Self::RecoveryDiscard => "recovery_discard",
        }
    }
}

/// The wire byte of a category.
#[must_use]
pub const fn category_code(c: Category) -> u8 {
    match c {
        Category::Hydration => 1,
        Category::Outcomes => 2,
        Category::WorkSession => 3,
        Category::PhoneReports => 4,
        Category::PostureSummary => 5,
    }
}

/// A category from its wire byte.
#[must_use]
pub const fn category_from_code(b: u8) -> Option<Category> {
    Some(match b {
        1 => Category::Hydration,
        2 => Category::Outcomes,
        3 => Category::WorkSession,
        4 => Category::PhoneReports,
        5 => Category::PostureSummary,
        _ => return None,
    })
}

/// Days since 1970-01-01 for a date (0 for dates before 1970, which never occur).
#[must_use]
pub fn day_number(d: NaiveDate) -> u32 {
    let days = d.signed_duration_since(NaiveDate::default()).num_days();
    u32::try_from(days).unwrap_or(0)
}

/// The date for a day number.
#[must_use]
pub fn from_day_number(n: u32) -> Option<NaiveDate> {
    NaiveDate::default().checked_add_days(chrono::Days::new(u64::from(n)))
}

/// The effect of one control entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryBody {
    /// kind 1: a category switched on (from `share_from`) or off.
    Policy {
        /// The category.
        category: Category,
        /// On or off.
        enabled: bool,
        /// Day number of `share_from`, 0 when off.
        share_from: u32,
        /// The operation's idempotency key.
        op_id: [u8; 16],
        /// The device that sent it.
        device_ref: [u8; 16],
    },
    /// kind 2: shared data deleted for a date range.
    Deletion {
        /// The category.
        category: Category,
        /// First day number.
        from: u32,
        /// Last day number.
        to: u32,
        /// The request's idempotency key.
        request_id: [u8; 16],
        /// The device that sent it.
        device_ref: [u8; 16],
    },
    /// kind 3: a device paired (pending until confirmed).
    Pairing {
        /// The new device.
        device_ref: [u8; 16],
        /// Its origin-device id.
        origin_device: [u8; 16],
        /// `SHA-256("HY-TOK1" ‖ token)`, so the device can authenticate after a replay.
        token_verifier: [u8; 32],
    },
    /// kind 4: the administrator confirmed a pending device.
    DeviceConfirmed {
        /// The confirmed device.
        device_ref: [u8; 16],
    },
    /// kind 5: a device revoked.
    Revocation {
        /// The revoked device.
        device_ref: [u8; 16],
        /// Why.
        reason: RevocationReason,
    },
    /// kind 6: the user's restored history was discarded.
    RecoveryDiscard {
        /// The snapshot it came from.
        snapshot_id: [u8; 16],
    },
    /// kind 7: a newly confirmed device read the user's history.
    HistoryAccess {
        /// The reading device.
        device_ref: [u8; 16],
    },
}

/// A body could not be decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodeError;

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("malformed control entry")
    }
}

impl std::error::Error for DecodeError {}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        if self.0.len() < N {
            return Err(DecodeError);
        }
        let (head, rest) = self.0.split_at(N);
        self.0 = rest;
        head.try_into().map_err(|_| DecodeError)
    }
    fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take::<1>()?[0])
    }
    fn u32(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_be_bytes(self.take::<4>()?))
    }
    fn done(&self) -> Result<(), DecodeError> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(DecodeError)
        }
    }
}

impl EntryBody {
    /// The kind byte.
    #[must_use]
    pub const fn kind(&self) -> u8 {
        match self {
            Self::Policy { .. } => 1,
            Self::Deletion { .. } => 2,
            Self::Pairing { .. } => 3,
            Self::DeviceConfirmed { .. } => 4,
            Self::Revocation { .. } => 5,
            Self::RecoveryDiscard { .. } => 6,
            Self::HistoryAccess { .. } => 7,
        }
    }

    /// The fixed-width body bytes.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(64);
        match self {
            Self::Policy {
                category,
                enabled,
                share_from,
                op_id,
                device_ref,
            } => {
                out.push(category_code(*category));
                out.push(u8::from(*enabled));
                out.extend_from_slice(&share_from.to_be_bytes());
                out.extend_from_slice(op_id);
                out.extend_from_slice(device_ref);
            }
            Self::Deletion {
                category,
                from,
                to,
                request_id,
                device_ref,
            } => {
                out.push(category_code(*category));
                out.extend_from_slice(&from.to_be_bytes());
                out.extend_from_slice(&to.to_be_bytes());
                out.extend_from_slice(request_id);
                out.extend_from_slice(device_ref);
            }
            Self::Pairing {
                device_ref,
                origin_device,
                token_verifier,
            } => {
                out.extend_from_slice(device_ref);
                out.extend_from_slice(origin_device);
                out.extend_from_slice(token_verifier);
            }
            Self::DeviceConfirmed { device_ref } | Self::HistoryAccess { device_ref } => {
                out.extend_from_slice(device_ref);
            }
            Self::Revocation { device_ref, reason } => {
                out.extend_from_slice(device_ref);
                out.push(reason.code());
            }
            Self::RecoveryDiscard { snapshot_id } => out.extend_from_slice(snapshot_id),
        }
        out
    }

    /// Decode a body of the given kind. Trailing bytes, unknown kinds, unknown
    /// categories or reasons, and out-of-range values are refused.
    ///
    /// # Errors
    /// [`DecodeError`] for anything but an exact, valid body.
    pub fn decode(kind: u8, body: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader(body);
        let out = match kind {
            1 => {
                let category = category_from_code(r.u8()?).ok_or(DecodeError)?;
                let enabled = match r.u8()? {
                    0 => false,
                    1 => true,
                    _ => return Err(DecodeError),
                };
                let share_from = r.u32()?;
                if enabled == (share_from == 0) {
                    return Err(DecodeError);
                }
                Self::Policy {
                    category,
                    enabled,
                    share_from,
                    op_id: r.take()?,
                    device_ref: r.take()?,
                }
            }
            2 => {
                let category = category_from_code(r.u8()?).ok_or(DecodeError)?;
                let from = r.u32()?;
                let to = r.u32()?;
                if from > to {
                    return Err(DecodeError);
                }
                Self::Deletion {
                    category,
                    from,
                    to,
                    request_id: r.take()?,
                    device_ref: r.take()?,
                }
            }
            3 => Self::Pairing {
                device_ref: r.take()?,
                origin_device: r.take()?,
                token_verifier: r.take()?,
            },
            4 => Self::DeviceConfirmed {
                device_ref: r.take()?,
            },
            5 => Self::Revocation {
                device_ref: r.take()?,
                reason: RevocationReason::from_code(r.u8()?).ok_or(DecodeError)?,
            },
            6 => Self::RecoveryDiscard {
                snapshot_id: r.take()?,
            },
            7 => Self::HistoryAccess {
                device_ref: r.take()?,
            },
            _ => return Err(DecodeError),
        };
        r.done()?;
        Ok(out)
    }
}

/// One entry of a user's chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlEntry {
    /// The user whose chain this is.
    pub user_id: [u8; 16],
    /// 1, 2, 3, ... per user.
    pub seq: u64,
    /// Server wall time of the entry (display and dates only; never ordering).
    pub at_ms: u64,
    /// The effect.
    pub body: EntryBody,
}

impl ControlEntry {
    /// `entry_hash` given the previous entry's hash (or the genesis).
    #[must_use]
    pub fn hash(&self, prev: &[u8; 32]) -> [u8; 32] {
        let body = self.body.encode();
        let len = u32::try_from(body.len()).unwrap_or(u32::MAX);
        sha256(&[
            ds::ENTRY,
            &self.user_id,
            &self.seq.to_be_bytes(),
            &self.at_ms.to_be_bytes(),
            &[self.body.kind()],
            &len.to_be_bytes(),
            &body,
            prev,
        ])
    }
}

/// `genesis(U)`, the `prev_hash` of a user's first entry.
#[must_use]
pub fn genesis(key_id: &[u8; 32], user_id: &[u8; 16]) -> [u8; 32] {
    sha256(&[ds::GENESIS, key_id, user_id])
}

/// Why a chain did not verify.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainError {
    /// An entry belongs to another user.
    WrongUser,
    /// Sequence numbers are not consecutive from the start point.
    Gap,
    /// The recomputed hash does not reach the expected end.
    Mismatch,
}

/// Recompute a run of entries from `(start_seq, start_hash)` and return the final
/// `(seq, hash)`. `start_seq` is 0 with the genesis hash for a full chain.
///
/// # Errors
/// [`ChainError`] if an entry is for another user or the seqs are not consecutive.
pub fn extend(
    user_id: &[u8; 16],
    start_seq: u64,
    start_hash: [u8; 32],
    entries: &[ControlEntry],
) -> Result<(u64, [u8; 32]), ChainError> {
    let mut seq = start_seq;
    let mut hash = start_hash;
    for e in entries {
        if &e.user_id != user_id {
            return Err(ChainError::WrongUser);
        }
        if e.seq != seq.checked_add(1).ok_or(ChainError::Gap)? {
            return Err(ChainError::Gap);
        }
        hash = e.hash(&hash);
        seq = e.seq;
    }
    Ok((seq, hash))
}

/// Check that `entries` extend `(start_seq, start_hash)` exactly to `(end_seq, end_hash)`.
///
/// # Errors
/// [`ChainError`] on any difference.
pub fn verify_extension(
    user_id: &[u8; 16],
    start_seq: u64,
    start_hash: [u8; 32],
    entries: &[ControlEntry],
    end_seq: u64,
    end_hash: &[u8; 32],
) -> Result<(), ChainError> {
    let (seq, hash) = extend(user_id, start_seq, start_hash, entries)?;
    if seq != end_seq {
        return Err(ChainError::Gap);
    }
    if !super::ct_eq32(&hash, end_hash) {
        return Err(ChainError::Mismatch);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const U: [u8; 16] = [9; 16];

    fn bodies() -> Vec<EntryBody> {
        vec![
            EntryBody::Pairing {
                device_ref: [1; 16],
                origin_device: [2; 16],
                token_verifier: [3; 32],
            },
            EntryBody::DeviceConfirmed {
                device_ref: [1; 16],
            },
            EntryBody::Policy {
                category: Category::Hydration,
                enabled: true,
                share_from: 20_384,
                op_id: [4; 16],
                device_ref: [1; 16],
            },
            EntryBody::Policy {
                category: Category::Hydration,
                enabled: false,
                share_from: 0,
                op_id: [5; 16],
                device_ref: [1; 16],
            },
            EntryBody::Deletion {
                category: Category::PhoneReports,
                from: 20_000,
                to: 20_010,
                request_id: [6; 16],
                device_ref: [1; 16],
            },
            EntryBody::HistoryAccess {
                device_ref: [1; 16],
            },
            EntryBody::Revocation {
                device_ref: [1; 16],
                reason: RevocationReason::SelfUnpair,
            },
            EntryBody::RecoveryDiscard {
                snapshot_id: [7; 16],
            },
        ]
    }

    fn chain() -> Vec<ControlEntry> {
        bodies()
            .into_iter()
            .enumerate()
            .map(|(i, body)| ControlEntry {
                user_id: U,
                seq: i as u64 + 1,
                at_ms: 1_700_000_000_000 + i as u64,
                body,
            })
            .collect()
    }

    #[test]
    fn every_body_round_trips_with_its_documented_width() {
        let widths = [64, 16, 38, 38, 41, 16, 17, 16];
        for (b, w) in bodies().iter().zip(widths) {
            let bytes = b.encode();
            assert_eq!(bytes.len(), w, "{b:?}");
            assert_eq!(EntryBody::decode(b.kind(), &bytes), Ok(*b));
            // Trailing or missing bytes are refused.
            let mut longer = bytes.clone();
            longer.push(0);
            assert!(EntryBody::decode(b.kind(), &longer).is_err());
            assert!(EntryBody::decode(b.kind(), &bytes[..bytes.len() - 1]).is_err());
        }
        assert!(EntryBody::decode(0, &[]).is_err());
        assert!(EntryBody::decode(8, &[0; 16]).is_err());
    }

    #[test]
    fn invalid_field_values_are_refused() {
        // Unknown category, bad flag, on without share_from, off with one, reversed range,
        // unknown reason.
        let mut p = bodies()[2].encode();
        p[0] = 9;
        assert!(EntryBody::decode(1, &p).is_err());
        let mut p = bodies()[2].encode();
        p[1] = 2;
        assert!(EntryBody::decode(1, &p).is_err());
        let mut p = bodies()[2].encode();
        p[2..6].copy_from_slice(&0u32.to_be_bytes());
        assert!(EntryBody::decode(1, &p).is_err());
        let mut p = bodies()[3].encode();
        p[2..6].copy_from_slice(&5u32.to_be_bytes());
        assert!(EntryBody::decode(1, &p).is_err());
        let mut d = bodies()[4].encode();
        d[1..5].copy_from_slice(&30_000u32.to_be_bytes());
        assert!(EntryBody::decode(2, &d).is_err());
        let mut r = bodies()[6].encode();
        r[16] = 99;
        assert!(EntryBody::decode(5, &r).is_err());
    }

    #[test]
    fn a_full_chain_verifies_from_genesis_and_a_suffix_from_its_receipt() {
        let key_id = [8u8; 32];
        let g = genesis(&key_id, &U);
        let entries = chain();
        let (seq, tail) = extend(&U, 0, g, &entries).unwrap();
        assert_eq!(seq, entries.len() as u64);
        verify_extension(&U, 0, g, &entries, seq, &tail).unwrap();
        // A receipt at seq 3 is extended by entries 4.. to the same tail.
        let (s3, h3) = extend(&U, 0, g, &entries[..3]).unwrap();
        assert_eq!(s3, 3);
        verify_extension(&U, s3, h3, &entries[3..], seq, &tail).unwrap();
        // Another key or user has another genesis.
        assert_ne!(genesis(&[0; 32], &U), g);
        assert_ne!(genesis(&key_id, &[0; 16]), g);
    }

    #[test]
    fn gaps_reordering_other_users_and_forks_fail() {
        let g = genesis(&[8; 32], &U);
        let entries = chain();
        let (seq, tail) = extend(&U, 0, g, &entries).unwrap();
        let mut gap = entries.clone();
        gap.remove(2);
        assert_eq!(
            verify_extension(&U, 0, g, &gap, seq, &tail),
            Err(ChainError::Gap)
        );
        let mut swapped = entries.clone();
        swapped.swap(1, 2);
        assert!(verify_extension(&U, 0, g, &swapped, seq, &tail).is_err());
        let mut other = entries.clone();
        other[4].user_id = [1; 16];
        assert_eq!(
            verify_extension(&U, 0, g, &other, seq, &tail),
            Err(ChainError::WrongUser)
        );
        // A fork: same seqs, one different entry.
        let mut fork = entries.clone();
        fork[5].at_ms += 1;
        assert_eq!(
            verify_extension(&U, 0, g, &fork, seq, &tail),
            Err(ChainError::Mismatch)
        );
    }

    #[test]
    fn day_numbers_round_trip() {
        let d = NaiveDate::from_ymd_opt(2026, 10, 23).unwrap();
        assert_eq!(from_day_number(day_number(d)), Some(d));
        assert_eq!(day_number(NaiveDate::from_ymd_opt(1970, 1, 2).unwrap()), 1);
    }

    proptest! {
        // T-S25 (property part): any single-bit change to an entry fails verification.
        #[test]
        fn any_single_bit_change_to_an_entry_fails(idx in 0usize..8, bit in 0usize..1024) {
            let g = genesis(&[8; 32], &U);
            let entries = chain();
            let (seq, tail) = extend(&U, 0, g, &entries).unwrap();
            let e = entries[idx];
            // The hashed bytes of the entry: user, seq, at, kind, len, body.
            let mut bytes = Vec::new();
            bytes.extend_from_slice(&e.user_id);
            bytes.extend_from_slice(&e.seq.to_be_bytes());
            bytes.extend_from_slice(&e.at_ms.to_be_bytes());
            bytes.push(e.body.kind());
            let body = e.body.encode();
            bytes.extend_from_slice(&(body.len() as u32).to_be_bytes());
            bytes.extend_from_slice(&body);
            let bit = bit % (bytes.len() * 8);
            bytes[bit / 8] ^= 1 << (bit % 8);
            // Re-read the flipped entry; a flip that makes it undecodable also fails.
            let user_id: [u8; 16] = bytes[..16].try_into().unwrap();
            let seq_f = u64::from_be_bytes(bytes[16..24].try_into().unwrap());
            let at_ms = u64::from_be_bytes(bytes[24..32].try_into().unwrap());
            let kind = bytes[32];
            let len = u32::from_be_bytes(bytes[33..37].try_into().unwrap()) as usize;
            let decoded = (len == bytes.len() - 37)
                .then(|| EntryBody::decode(kind, &bytes[37..]).ok())
                .flatten();
            if let Some(body) = decoded {
                let mut changed = entries.clone();
                changed[idx] = ControlEntry { user_id, seq: seq_f, at_ms, body };
                prop_assert!(verify_extension(&U, 0, g, &changed, seq, &tail).is_err());
            }
        }

        // T-S25: the body decoder never panics on arbitrary input.
        #[test]
        fn decode_never_panics(kind in any::<u8>(), body in proptest::collection::vec(any::<u8>(), 0..80)) {
            if let Ok(b) = EntryBody::decode(kind, &body) {
                prop_assert_eq!(b.encode(), body);
            }
        }
    }
}
