//! The human checks used in the mandatory administrator confirmation (contract 4.2 steps
//! 8 and 9).
//!
//! - **Six fingerprint words:** the first 66 bits of the pinned fingerprint, as six 11-bit
//!   indexes into the BIP-39 English list (2048 words). The companion shows the words of
//!   the pin it actually holds and the admin page shows the server's own, so a swapped
//!   fingerprint on a tampered slip gives different words. They are an auxiliary display:
//!   the full fingerprint is the only authoritative pin check.
//! - **Device check code:** four Crockford base32 characters, the first 20 bits of
//!   `SHA-256("HY-DEVCHK1\0" ‖ device_ref)`. It tells pending rows apart. It is not a
//!   secret and not a security control by itself.
//!
//! The word list is the BIP-39 English list (`data/bip39-english.txt`, from
//! bitcoin/bips, MIT licence; SHA-256 `2f5eed53…24dbda`, checked by a test).

use std::sync::OnceLock;

use super::slip::leading_bits_code;
use super::{ds, sha256};

const WORDLIST: &str = include_str!("../../data/bip39-english.txt");

fn words() -> &'static [&'static str] {
    static LIST: OnceLock<Vec<&'static str>> = OnceLock::new();
    LIST.get_or_init(|| {
        WORDLIST
            .lines()
            .map(str::trim)
            .filter(|w| !w.is_empty())
            .collect()
    })
}

/// The six words for a fingerprint.
#[must_use]
pub fn fingerprint_words(fingerprint: &[u8; 32]) -> [&'static str; 6] {
    let list = words();
    let mut out = [""; 6];
    for (i, slot) in out.iter_mut().enumerate() {
        let mut v = 0usize;
        for bit in i * 11..i * 11 + 11 {
            v = (v << 1) | usize::from((fingerprint[bit / 8] >> (7 - bit % 8)) & 1);
        }
        *slot = list.get(v).copied().unwrap_or("?");
    }
    out
}

/// The six words joined with spaces.
#[must_use]
pub fn fingerprint_phrase(fingerprint: &[u8; 32]) -> String {
    fingerprint_words(fingerprint).join(" ")
}

/// The device check code for a device reference.
#[must_use]
pub fn device_check_code(device_ref: &[u8; 16]) -> String {
    leading_bits_code(&sha256(&[ds::DEVICE_CHECK, device_ref]), 4)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::hex;

    #[test]
    fn the_bundled_list_is_the_bip39_english_list() {
        let normalised: String = WORDLIST.replace("\r\n", "\n");
        assert_eq!(
            hex(&sha256(&[normalised.as_bytes()])),
            "2f5eed53a4727b4bf8880d8f3f199efc90e58503646d9ff8eff3a2ed3b24dbda"
        );
        assert_eq!(words().len(), 2048);
        assert_eq!(words()[0], "abandon");
        assert_eq!(words()[2047], "zoo");
    }

    #[test]
    fn words_take_the_first_66_bits() {
        assert_eq!(fingerprint_words(&[0u8; 32]), ["abandon"; 6]);
        assert_eq!(fingerprint_words(&[0xFF; 32]), ["zoo"; 6]);
        // Only the first 66 bits matter.
        let mut a = [0u8; 32];
        a[9] = 0x3F;
        a[31] = 0xFF;
        assert_eq!(fingerprint_words(&a), ["abandon"; 6]);
        // Bit 65 (the last one used) changes the sixth word.
        a[8] = 0x40;
        assert_eq!(fingerprint_words(&a)[5], "ability");
        assert_eq!(fingerprint_phrase(&[0u8; 32]).split(' ').count(), 6);
    }

    #[test]
    fn different_fingerprints_give_different_words() {
        let a = sha256(&[b"server a"]);
        let b = sha256(&[b"server b"]);
        assert_ne!(fingerprint_words(&a), fingerprint_words(&b));
    }

    #[test]
    fn check_codes_are_four_symbols_and_tell_devices_apart() {
        let a = device_check_code(&[1; 16]);
        let b = device_check_code(&[2; 16]);
        assert_eq!(a.len(), 4);
        assert!(a.chars().all(|c| super::super::slip::ALPHABET.contains(c)));
        assert_ne!(a, b);
        assert_eq!(a, device_check_code(&[1; 16]));
    }
}
