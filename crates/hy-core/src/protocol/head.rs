//! The signed head (contract 7.2), kept by each device as its receipt.
//!
//! ```text
//! head    = { key_id[32], user_id[16], seq u64, chain_hash[32], issued_at_ms u64 }
//! message = "HY-HEAD1\0" ‖ key_id ‖ user_id ‖ u64be(seq) ‖ chain_hash ‖ u64be(issued_at_ms)
//! sig     = Ed25519_sign(head_key, message)               (64 bytes, deterministic)
//! wire    = base64url(head ‖ sig)
//! ```
//!
//! A head is valid only under the `key_id` the device stored at pairing; a head under any
//! other key is refused.

use aws_lc_rs::signature::{ED25519, Ed25519KeyPair, KeyPair, UnparsedPublicKey};

use super::{b64, ds, key_id, unb64_exact};

/// Bytes of an encoded head without its signature.
pub const HEAD_LEN: usize = 32 + 16 + 8 + 32 + 8;
/// Bytes of a head with its signature.
pub const SIGNED_LEN: usize = HEAD_LEN + 64;

/// A user's chain position as the server states it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Head {
    /// The head-signing key's id.
    pub key_id: [u8; 32],
    /// The user.
    pub user_id: [u8; 16],
    /// The chain's last seq.
    pub seq: u64,
    /// The chain's last entry hash.
    pub chain_hash: [u8; 32],
    /// When the server issued it.
    pub issued_at_ms: u64,
}

impl Head {
    fn bytes(&self) -> [u8; HEAD_LEN] {
        let mut out = [0u8; HEAD_LEN];
        out[..32].copy_from_slice(&self.key_id);
        out[32..48].copy_from_slice(&self.user_id);
        out[48..56].copy_from_slice(&self.seq.to_be_bytes());
        out[56..88].copy_from_slice(&self.chain_hash);
        out[88..96].copy_from_slice(&self.issued_at_ms.to_be_bytes());
        out
    }

    /// The signed message.
    #[must_use]
    pub fn message(&self) -> Vec<u8> {
        let mut m = Vec::with_capacity(ds::HEAD.len() + HEAD_LEN);
        m.extend_from_slice(ds::HEAD);
        m.extend_from_slice(&self.bytes());
        m
    }

    /// Sign with the server's head key.
    #[must_use]
    pub fn sign(self, key: &Ed25519KeyPair) -> SignedHead {
        let sig = key.sign(&self.message());
        let mut out = [0u8; 64];
        out.copy_from_slice(sig.as_ref());
        SignedHead {
            head: self,
            sig: out,
        }
    }
}

/// The 32-byte public key of a head-signing key pair.
#[must_use]
pub fn public_key(key: &Ed25519KeyPair) -> [u8; 32] {
    let mut out = [0u8; 32];
    out.copy_from_slice(key.public_key().as_ref());
    out
}

/// Why a signed head was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeadError {
    /// Not a well-formed encoded head.
    Malformed,
    /// Signed under a key other than the one stored at pairing.
    WrongKey,
    /// The head names another user.
    WrongUser,
    /// The signature does not verify.
    BadSignature,
}

/// A head and its signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignedHead {
    /// The head.
    pub head: Head,
    /// Ed25519 signature over [`Head::message`].
    pub sig: [u8; 64],
}

impl SignedHead {
    /// The wire form: base64url of the head and signature.
    #[must_use]
    pub fn to_wire(&self) -> String {
        let mut bytes = [0u8; SIGNED_LEN];
        bytes[..HEAD_LEN].copy_from_slice(&self.head.bytes());
        bytes[HEAD_LEN..].copy_from_slice(&self.sig);
        b64(&bytes)
    }

    /// Decode the wire form (no signature check).
    ///
    /// # Errors
    /// [`HeadError::Malformed`] unless it is exactly one encoded head.
    pub fn from_wire(text: &str) -> Result<Self, HeadError> {
        let bytes: [u8; SIGNED_LEN] = unb64_exact(text).ok_or(HeadError::Malformed)?;
        let take32 = |r: std::ops::Range<usize>| -> [u8; 32] {
            let mut a = [0u8; 32];
            a.copy_from_slice(&bytes[r]);
            a
        };
        let mut user_id = [0u8; 16];
        user_id.copy_from_slice(&bytes[32..48]);
        let mut seq = [0u8; 8];
        seq.copy_from_slice(&bytes[48..56]);
        let mut at = [0u8; 8];
        at.copy_from_slice(&bytes[88..96]);
        let mut sig = [0u8; 64];
        sig.copy_from_slice(&bytes[HEAD_LEN..]);
        Ok(Self {
            head: Head {
                key_id: take32(0..32),
                user_id,
                seq: u64::from_be_bytes(seq),
                chain_hash: take32(56..88),
                issued_at_ms: u64::from_be_bytes(at),
            },
            sig,
        })
    }

    /// Verify under the head public key stored at pairing, for the expected user.
    ///
    /// # Errors
    /// [`HeadError`] if the key id, user or signature does not match.
    pub fn verify(&self, head_public_key: &[u8; 32], user_id: &[u8; 16]) -> Result<(), HeadError> {
        if !super::ct_eq32(&self.head.key_id, &key_id(head_public_key)) {
            return Err(HeadError::WrongKey);
        }
        if &self.head.user_id != user_id {
            return Err(HeadError::WrongUser);
        }
        UnparsedPublicKey::new(&ED25519, head_public_key)
            .verify(&self.head.message(), &self.sig)
            .map_err(|_| HeadError::BadSignature)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aws_lc_rs::rand::SystemRandom;
    use proptest::prelude::*;

    fn keypair() -> Ed25519KeyPair {
        let doc = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
        Ed25519KeyPair::from_pkcs8(doc.as_ref()).unwrap()
    }

    fn head(pk: &[u8; 32]) -> Head {
        Head {
            key_id: key_id(pk),
            user_id: [5; 16],
            seq: 42,
            chain_hash: [6; 32],
            issued_at_ms: 1_761_000_000_000,
        }
    }

    #[test]
    fn a_signed_head_round_trips_and_verifies() {
        let k = keypair();
        let pk = public_key(&k);
        let s = head(&pk).sign(&k);
        let wire = s.to_wire();
        let back = SignedHead::from_wire(&wire).unwrap();
        assert_eq!(back, s);
        back.verify(&pk, &[5; 16]).unwrap();
        // Deterministic signatures.
        assert_eq!(head(&pk).sign(&k).sig, s.sig);
    }

    #[test]
    fn wrong_key_user_or_signature_is_refused() {
        let k = keypair();
        let pk = public_key(&k);
        let other = keypair();
        let opk = public_key(&other);
        let s = head(&pk).sign(&k);
        assert_eq!(s.verify(&opk, &[5; 16]), Err(HeadError::WrongKey));
        assert_eq!(s.verify(&pk, &[4; 16]), Err(HeadError::WrongUser));
        // Signed by another key but claiming this key's id.
        let forged = head(&pk).sign(&other);
        assert_eq!(forged.verify(&pk, &[5; 16]), Err(HeadError::BadSignature));
        assert_eq!(
            SignedHead::from_wire("not-a-head"),
            Err(HeadError::Malformed)
        );
        assert_eq!(
            SignedHead::from_wire(&b64(&[0u8; SIGNED_LEN - 1])),
            Err(HeadError::Malformed)
        );
    }

    proptest! {
        // T-S25 (property part): any single-bit change to a signed head fails.
        #[test]
        fn any_single_bit_change_fails(bit in 0usize..SIGNED_LEN * 8) {
            let k = keypair();
            let pk = public_key(&k);
            let s = head(&pk).sign(&k);
            let mut bytes = unb64_exact::<SIGNED_LEN>(&s.to_wire()).unwrap();
            bytes[bit / 8] ^= 1 << (bit % 8);
            let flipped = SignedHead::from_wire(&b64(&bytes)).unwrap();
            prop_assert!(flipped.verify(&pk, &[5; 16]).is_err());
        }

        #[test]
        fn decoder_never_panics(text in "\\PC{0,300}") {
            let _ = SignedHead::from_wire(&text);
        }
    }
}
