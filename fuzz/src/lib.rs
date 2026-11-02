//! Shared pieces of the T-S25 fuzz targets: the fixed head key and the binary input
//! format of `chain_verify`.

use aws_lc_rs::signature::Ed25519KeyPair;
use hy_core::protocol::chain::{ControlEntry, EntryBody};
use hy_core::protocol::head;

/// The seed of the head-signing key the seed corpora are signed with (the same bytes as
/// `crates/hy-core/tests/fuzz_seeds.rs`), so a seed's head verifies and the fuzzer starts
/// on the accepted path.
pub const HEAD_KEY_SEED: [u8; 32] = [0x42; 32];

/// The fixed head key pair.
///
/// # Panics
/// Never: the seed is a valid Ed25519 seed.
#[must_use]
pub fn head_key() -> Ed25519KeyPair {
    Ed25519KeyPair::from_seed_unchecked(&HEAD_KEY_SEED).expect("a 32-byte seed")
}

/// The fixed head public key.
#[must_use]
pub fn head_public() -> [u8; 32] {
    head::public_key(&head_key())
}

/// A chain run to verify, decoded from the fuzzer's bytes.
#[derive(Debug)]
pub struct ChainInput {
    /// The chain's user.
    pub user_id: [u8; 16],
    /// Where the run starts (0 with the genesis for a full chain).
    pub start_seq: u64,
    /// The hash at `start_seq`.
    pub start_hash: [u8; 32],
    /// The claimed end.
    pub end_seq: u64,
    /// The claimed end hash.
    pub end_hash: [u8; 32],
    /// Every entry, with its body decoded. Entries whose body does not decode are
    /// counted in `refused`.
    pub entries: Vec<ControlEntry>,
    /// How many entries were refused by the body decoder.
    pub refused: usize,
}

/// Most entries read from one input (a state page holds at most 500; the hashing cost,
/// not the format, is what this bounds).
pub const MAX_ENTRIES: usize = 600;

fn take<const N: usize>(data: &mut &[u8]) -> Option<[u8; N]> {
    if data.len() < N {
        return None;
    }
    let (head, rest) = data.split_at(N);
    *data = rest;
    head.try_into().ok()
}

/// Decode the `chain_verify` input:
///
/// ```text
/// user_id[16] start_seq u64be start_hash[32] end_seq u64be end_hash[32]
/// then entries: seq u64be  at_ms u64be  kind u8  len u8  body[len]
/// ```
///
/// Each entry's entry user is the chain's user unless its kind byte has bit 7 set (then
/// it is a different user, to reach the wrong-user check). A short header gives `None`;
/// a short trailing entry ends the list.
#[must_use]
pub fn parse_chain(mut data: &[u8]) -> Option<ChainInput> {
    let d = &mut data;
    let user_id = take::<16>(d)?;
    let start_seq = u64::from_be_bytes(take::<8>(d)?);
    let start_hash = take::<32>(d)?;
    let end_seq = u64::from_be_bytes(take::<8>(d)?);
    let end_hash = take::<32>(d)?;
    let mut entries = Vec::new();
    let mut refused = 0;
    while entries.len() + refused < MAX_ENTRIES {
        let Some(seq) = take::<8>(d) else { break };
        let Some(at) = take::<8>(d) else { break };
        let Some([kind, len]) = take::<2>(d) else {
            break;
        };
        let len = usize::from(len);
        if d.len() < len {
            break;
        }
        let (body, rest) = d.split_at(len);
        *d = rest;
        let mut entry_user = user_id;
        if kind & 0x80 != 0 {
            entry_user[0] ^= 0xFF;
        }
        match EntryBody::decode(kind & 0x7F, body) {
            Ok(b) => entries.push(ControlEntry {
                user_id: entry_user,
                seq: u64::from_be_bytes(seq),
                at_ms: u64::from_be_bytes(at),
                body: b,
            }),
            Err(_) => refused += 1,
        }
    }
    Some(ChainInput {
        user_id,
        start_seq,
        start_hash,
        end_seq,
        end_hash,
        entries,
        refused,
    })
}
