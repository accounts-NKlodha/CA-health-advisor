//! The office-server protocol shared by the server, the companion and the simulator
//! (V1-5 contract, `docs/V1-5_Pairing_Security.md`).
//!
//! Everything here is pure data and hashing: no sockets, no storage. It defines
//!
//! - [`slip`]: the pairing slip `hy1-<host>-<port>-<fp>-<secret>-<check>` (4.3), parsed
//!   right to left with a typo checksum;
//! - [`words`]: the six fingerprint words and the device check code shown during the
//!   mandatory administrator confirmation (4.2 steps 8 and 9);
//! - [`chain`]: the fixed binary encoding of control-log entries and the per-user hash
//!   chain (7.1);
//! - [`head`]: the Ed25519-signed head that devices keep as their receipt (7.2);
//! - [`share`]: the per-category payload schemas (8.2);
//! - [`wire`]: the JSON bodies of `/v1` and the error enum (appendix A).
//!
//! Every hash or signature input starts with a versioned domain-separation string
//! (appendix B), so a later format can never be mistaken for this one. JSON is never
//! hashed or signed.

pub mod chain;
pub mod head;
pub mod share;
pub mod slip;
pub mod wire;
pub mod words;

use aws_lc_rs::digest;
use subtle::ConstantTimeEq;

/// ALPN protocol id, on both sides of every connection (contract 5).
pub const ALPN: &[u8] = b"hy/1";
/// The constant SNI the companion sends. Names are never checked: the pin is the identity.
pub const SNI: &str = "hy-server.invalid";
/// The `/v1` protocol number sent at pairing.
pub const PROTOCOL: u32 = 1;
/// The LAN API port.
pub const DEFAULT_PORT: u16 = 8443;
/// The admin page port, on 127.0.0.1 only.
pub const ADMIN_PORT: u16 = 8444;
/// DPAPI optional entropy for the companion's token blob (contract 6.1).
pub const DPAPI_ENTROPY: &[u8] = b"HealthierYou/server_link/v1";

/// Domain-separation strings (contract appendix B).
pub mod ds {
    /// Slip secret hash.
    pub const SLIP: &[u8] = b"HY-SLIP1";
    /// Slip typo checksum.
    pub const SLIP_CHECK: &[u8] = b"HY-SLIPCHK1";
    /// Device check code.
    pub const DEVICE_CHECK: &[u8] = b"HY-DEVCHK1\0";
    /// Device token verifier.
    pub const TOKEN: &[u8] = b"HY-TOK1";
    /// Head-signing key id.
    pub const KEY_ID: &[u8] = b"HY-KEYID1\0";
    /// Control-log entry hash.
    pub const ENTRY: &[u8] = b"HY-CL1\0";
    /// Control-log genesis.
    pub const GENESIS: &[u8] = b"HY-CL1-GENESIS\0";
    /// Signed head message.
    pub const HEAD: &[u8] = b"HY-HEAD1\0";
}

/// Length of the slip secret in bytes (80 bits).
pub const SLIP_SECRET_LEN: usize = 10;
/// Length of a device token in bytes (256 bits).
pub const TOKEN_LEN: usize = 32;

/// SHA-256 over the concatenation of `parts`.
#[must_use]
pub fn sha256(parts: &[&[u8]]) -> [u8; 32] {
    let mut ctx = digest::Context::new(&digest::SHA256);
    for p in parts {
        ctx.update(p);
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(ctx.finish().as_ref());
    out
}

/// The server's identity: SHA-256 of its certificate's DER (contract 5).
#[must_use]
pub fn fingerprint(cert_der: &[u8]) -> [u8; 32] {
    sha256(&[cert_der])
}

/// What the server stores for a slip: `SHA-256("HY-SLIP1" ‖ secret)`.
#[must_use]
pub fn slip_hash(secret: &[u8; SLIP_SECRET_LEN]) -> [u8; 32] {
    sha256(&[ds::SLIP, secret])
}

/// What the server stores for a device token: `SHA-256("HY-TOK1" ‖ token)`.
#[must_use]
pub fn token_verifier(token: &[u8; TOKEN_LEN]) -> [u8; 32] {
    sha256(&[ds::TOKEN, token])
}

/// The id of a head-signing public key: `SHA-256("HY-KEYID1\0" ‖ public key)`.
#[must_use]
pub fn key_id(head_public_key: &[u8; 32]) -> [u8; 32] {
    sha256(&[ds::KEY_ID, head_public_key])
}

/// Constant-time equality of two 32-byte values (pins and verifiers).
#[must_use]
pub fn ct_eq32(a: &[u8; 32], b: &[u8; 32]) -> bool {
    a.ct_eq(b).into()
}

/// The first 8 hex characters of a fingerprint or key: the only form that may be logged
/// (contract 6.1 and 11).
#[must_use]
pub fn prefix8(value: &[u8; 32]) -> String {
    hex(&value[..4])
}

/// Lower-case hex.
#[must_use]
pub fn hex(bytes: &[u8]) -> String {
    data_encoding::HEXLOWER.encode(bytes)
}

/// Unpadded base64url, as used for tokens, keys and heads in JSON.
#[must_use]
pub fn b64(bytes: &[u8]) -> String {
    data_encoding::BASE64URL_NOPAD.encode(bytes)
}

/// Decode unpadded base64url of exactly `N` bytes.
#[must_use]
pub fn unb64_exact<const N: usize>(text: &str) -> Option<[u8; N]> {
    let bytes = data_encoding::BASE64URL_NOPAD
        .decode(text.as_bytes())
        .ok()?;
    bytes.try_into().ok()
}

/// Decode unpadded base64url of any length up to `max` bytes.
#[must_use]
pub fn unb64(text: &str, max: usize) -> Option<Vec<u8>> {
    if text.len() > max.saturating_mul(4) / 3 + 4 {
        return None;
    }
    let bytes = data_encoding::BASE64URL_NOPAD
        .decode(text.as_bytes())
        .ok()?;
    (bytes.len() <= max).then_some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_the_standard_vector() {
        assert_eq!(
            hex(&sha256(&[b"ab", b"c"])),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn hashes_are_domain_separated() {
        let secret = [1u8; SLIP_SECRET_LEN];
        let mut token = [0u8; TOKEN_LEN];
        token[..SLIP_SECRET_LEN].copy_from_slice(&secret);
        assert_ne!(slip_hash(&secret), sha256(&[&secret]));
        assert_ne!(token_verifier(&token), sha256(&[&token]));
        assert_ne!(key_id(&token), token_verifier(&token));
    }

    #[test]
    fn prefixes_and_base64_round_trip() {
        let v = sha256(&[b"x"]);
        assert_eq!(prefix8(&v).len(), 8);
        assert_eq!(unb64_exact::<32>(&b64(&v)), Some(v));
        assert_eq!(unb64_exact::<31>(&b64(&v)), None);
        assert_eq!(unb64(&b64(&v), 32).as_deref(), Some(&v[..]));
        assert_eq!(unb64(&b64(&v), 31), None);
        assert!(ct_eq32(&v, &v));
        assert!(!ct_eq32(&v, &sha256(&[b"y"])));
    }
}
