//! Secret values that must never reach a log, a JSON body by accident, or freed memory
//! (V1-5 contract section 11, "Types").
//!
//! [`Secret`] wraps a device token, a slip secret, a passphrase or a key. Its `Debug` and
//! `Display` print `[redacted]`, it does not implement `Serialize`, and it zeroes its
//! contents when dropped. Code that must send or hash the value calls
//! [`Secret::expose`], so every place a secret leaves its wrapper is easy to find.

use std::fmt;

use zeroize::{Zeroize, ZeroizeOnDrop};

/// What a [`Secret`] prints instead of its value.
pub const REDACTED: &str = "[redacted]";

/// A value that is redacted when printed and zeroed when dropped.
pub struct Secret<T: Zeroize>(T);

impl<T: Zeroize> Secret<T> {
    /// Wrap `value`.
    pub fn new(value: T) -> Self {
        Self(value)
    }

    /// The wrapped value, for the one place that has to send or hash it.
    pub fn expose(&self) -> &T {
        &self.0
    }
}

impl<T: Zeroize> Drop for Secret<T> {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl<T: Zeroize> ZeroizeOnDrop for Secret<T> {}

impl<T: Zeroize + Clone> Clone for Secret<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T: Zeroize> fmt::Debug for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED)
    }
}

impl<T: Zeroize> fmt::Display for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED)
    }
}

impl<T: Zeroize> From<T> for Secret<T> {
    fn from(value: T) -> Self {
        Self(value)
    }
}

/// Seals the device token for storage (contract 6.1).
///
/// On Windows the companion seals with user-scope DPAPI and the app entropy
/// [`crate::protocol::DPAPI_ENTROPY`], so only the same Windows user on the same PC can
/// open the blob; a copied database shows "re-pair needed". The trait keeps the store and
/// the link logic testable on Linux.
pub trait TokenVault: Send + Sync + fmt::Debug {
    /// Seal a token.
    ///
    /// # Errors
    /// [`crate::Error::Invalid`] if the platform refuses.
    fn seal(&self, token: &Secret<[u8; 32]>) -> crate::Result<Vec<u8>>;

    /// Open a sealed token.
    ///
    /// # Errors
    /// [`crate::Error::Invalid`] if the blob cannot be opened here (another user or PC, a
    /// damaged blob): the caller treats that as "re-pair needed".
    fn open(&self, blob: &[u8]) -> crate::Result<Secret<[u8; 32]>>;
}

/// A vault that only masks the token with a per-instance key held in memory.
///
/// **For tests and the load simulator only.** It gives no protection at rest; the
/// companion never uses it (it uses DPAPI on Windows and refuses to pair elsewhere).
#[derive(Debug)]
pub struct EphemeralVault {
    key: Secret<[u8; 32]>,
}

impl EphemeralVault {
    /// A vault with a fresh random key: blobs from another instance do not open.
    ///
    /// # Panics
    /// Only if the operating system's random source fails.
    #[must_use]
    pub fn new() -> Self {
        let mut key = [0u8; 32];
        #[allow(clippy::expect_used)]
        getrandom::fill(&mut key).expect("operating system random source failed");
        Self {
            key: Secret::new(key),
        }
    }

    fn tag(&self, masked: &[u8; 32]) -> [u8; 32] {
        crate::protocol::sha256(&[b"HY-TESTVAULT1", self.key.expose(), masked])
    }
}

impl Default for EphemeralVault {
    fn default() -> Self {
        Self::new()
    }
}

impl TokenVault for EphemeralVault {
    fn seal(&self, token: &Secret<[u8; 32]>) -> crate::Result<Vec<u8>> {
        let mut masked = [0u8; 32];
        for (i, b) in masked.iter_mut().enumerate() {
            *b = token.expose()[i] ^ self.key.expose()[i];
        }
        let mut out = masked.to_vec();
        out.extend_from_slice(&self.tag(&masked));
        masked.zeroize();
        Ok(out)
    }

    fn open(&self, blob: &[u8]) -> crate::Result<Secret<[u8; 32]>> {
        let (masked, tag) = blob
            .split_at_checked(32)
            .ok_or_else(|| crate::Error::invalid("token blob cannot be opened here"))?;
        let masked: [u8; 32] = masked
            .try_into()
            .map_err(|_| crate::Error::invalid("token blob cannot be opened here"))?;
        if !crate::protocol::ct_eq32(
            &self.tag(&masked),
            &tag.try_into()
                .map_err(|_| crate::Error::invalid("token blob cannot be opened here"))?,
        ) {
            return Err(crate::Error::invalid("token blob cannot be opened here"));
        }
        let mut token = [0u8; 32];
        for (i, b) in token.iter_mut().enumerate() {
            *b = masked[i] ^ self.key.expose()[i];
        }
        Ok(Secret::new(token))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn printing_never_shows_the_value() {
        let s = Secret::new(String::from("CANARY_TOKEN_51c2"));
        assert_eq!(format!("{s}"), REDACTED);
        assert_eq!(format!("{s:?}"), REDACTED);
        assert_eq!(format!("{:?}", Some(&s)), format!("Some({REDACTED})"));
        assert_eq!(s.expose(), "CANARY_TOKEN_51c2");
    }

    #[test]
    fn a_vault_opens_only_its_own_blobs() {
        let a = EphemeralVault::new();
        let b = EphemeralVault::new();
        let t = Secret::new([3u8; 32]);
        let blob = a.seal(&t).unwrap();
        assert_ne!(&blob[..32], t.expose());
        assert_eq!(a.open(&blob).unwrap().expose(), t.expose());
        assert!(b.open(&blob).is_err());
        assert!(a.open(&blob[..40]).is_err());
    }

    #[test]
    fn bytes_are_zeroed_in_place() {
        let mut s = Secret::new([7u8; 32]);
        s.0.zeroize();
        assert_eq!(s.expose(), &[0u8; 32]);
    }
}
