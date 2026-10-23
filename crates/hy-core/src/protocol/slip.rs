//! The pairing slip (contract 4.3):
//!
//! ```text
//! hy1-<host>-<port>-<fingerprint: 52 base32>-<secret: 16 base32>-<check: 4 base32>
//! ```
//!
//! - Crockford base32, case-insensitive; on input `O` reads as `0` and `I` or `L` as `1`.
//!   Spaces anywhere, and hyphens inside the three fixed-length groups, are ignored, so a
//!   slip typed from print in groups of four parses the same as a pasted one.
//! - Parsed **right to left**: the check, the secret and the fingerprint have fixed
//!   lengths, then comes the decimal port, and whatever is left after `hy1-` is the host.
//!   A hyphenated host name is therefore never ambiguous.
//! - The check is the first 20 bits of `SHA-256("HY-SLIPCHK1" ‖ canonical slip without
//!   check)`. It catches typos only, so a typo is reported as "mistyped", never as "wrong
//!   server". It is not a security control.
//! - The slip carries no user alias, so a leaked slip does not name its user.

use std::fmt;
use std::sync::OnceLock;

use super::{SLIP_SECRET_LEN, ds, sha256};
use crate::secret::Secret;

/// Crockford base32 symbols.
pub const ALPHABET: &str = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";

const FP_CHARS: usize = 52;
const SECRET_CHARS: usize = 16;
const CHECK_CHARS: usize = 4;
const MAX_HOST: usize = 253;
const MAX_INPUT: usize = 512;

/// The Crockford base32 encoding used for the fingerprint and the secret.
#[allow(clippy::expect_used)]
pub(crate) fn crockford() -> &'static data_encoding::Encoding {
    static ENC: OnceLock<data_encoding::Encoding> = OnceLock::new();
    ENC.get_or_init(|| {
        let mut spec = data_encoding::Specification::new();
        spec.symbols.push_str(ALPHABET);
        spec.translate.from.push_str("abcdefghjkmnpqrstvwxyzOoIiLl");
        spec.translate.to.push_str("ABCDEFGHJKMNPQRSTVWXYZ001111");
        spec.check_trailing_bits = true;
        spec.encoding()
            .expect("the Crockford specification is valid")
    })
}

/// The first `chars * 5` bits of `bytes` as Crockford symbols (check codes).
pub(crate) fn leading_bits_code(bytes: &[u8], chars: usize) -> String {
    let alphabet = ALPHABET.as_bytes();
    (0..chars)
        .map(|i| {
            let mut v = 0usize;
            for bit in i * 5..i * 5 + 5 {
                let byte = bytes.get(bit / 8).copied().unwrap_or(0);
                v = (v << 1) | usize::from((byte >> (7 - bit % 8)) & 1);
            }
            char::from(alphabet[v])
        })
        .collect()
}

/// Map one typed character to its canonical symbol, if it is one.
fn symbol(c: char) -> Option<char> {
    let up = match c {
        'o' | 'O' => '0',
        'i' | 'I' | 'l' | 'L' => '1',
        other => other.to_ascii_uppercase(),
    };
    ALPHABET.contains(up).then_some(up)
}

/// The secret as sent in `POST /v1/pair`: 16 Crockford base32 characters.
#[must_use]
pub fn encode_secret(secret: &[u8; SLIP_SECRET_LEN]) -> Secret<String> {
    Secret::new(crockford().encode(secret))
}

/// Read a secret sent in `POST /v1/pair`. `None` unless it is exactly 16 valid symbols.
#[must_use]
pub fn decode_secret(text: &str) -> Option<Secret<[u8; SLIP_SECRET_LEN]>> {
    if text.len() != SECRET_CHARS {
        return None;
    }
    let bytes = crockford().decode(text.as_bytes()).ok()?;
    let arr: [u8; SLIP_SECRET_LEN] = bytes.try_into().ok()?;
    Some(Secret::new(arr))
}

/// Why a slip could not be read. Both are local, before any network use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlipError {
    /// Not the shape of a slip (wrong lengths, characters, port or host).
    Malformed,
    /// The shape is right but the checksum does not match: a typo.
    Mistyped,
}

impl fmt::Display for SlipError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Malformed => "That doesn't look like a pairing slip.",
            Self::Mistyped => "The slip looks mistyped. Check it and try again.",
        })
    }
}

impl std::error::Error for SlipError {}

/// A parsed pairing slip.
#[derive(Debug, Clone)]
pub struct Slip {
    /// The server PC's computer name, or an IP address (lower case).
    pub host: String,
    /// The API port.
    pub port: u16,
    /// SHA-256 of the server certificate's DER: the pin.
    pub fingerprint: [u8; 32],
    /// The single-use secret.
    pub secret: Secret<[u8; SLIP_SECRET_LEN]>,
}

fn host_ok(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= MAX_HOST
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        && !host.starts_with(['-', '.'])
        && !host.ends_with(['-', '.'])
}

impl Slip {
    /// A new slip; the host is lower-cased.
    ///
    /// # Errors
    /// [`SlipError::Malformed`] if the host is not 1 to 253 characters of
    /// `[A-Za-z0-9.-]` or the port is 0.
    pub fn new(
        host: &str,
        port: u16,
        fingerprint: [u8; 32],
        secret: [u8; SLIP_SECRET_LEN],
    ) -> Result<Self, SlipError> {
        if !host_ok(host) || port == 0 {
            return Err(SlipError::Malformed);
        }
        Ok(Self {
            host: host.to_ascii_lowercase(),
            port,
            fingerprint,
            secret: Secret::new(secret),
        })
    }

    fn body(&self) -> String {
        format!(
            "hy1-{}-{}-{}-{}",
            self.host,
            self.port,
            crockford().encode(&self.fingerprint),
            crockford().encode(self.secret.expose())
        )
    }

    fn check_for(body: &str) -> String {
        leading_bits_code(&sha256(&[ds::SLIP_CHECK, body.as_bytes()]), CHECK_CHARS)
    }

    /// The canonical slip, to copy into internal chat. It contains the secret.
    #[must_use]
    pub fn canonical(&self) -> Secret<String> {
        let body = self.body();
        let check = Self::check_for(&body);
        Secret::new(format!("{body}-{check}"))
    }

    /// The printable slip: the fingerprint and the secret in groups of four.
    #[must_use]
    pub fn printable(&self) -> Secret<String> {
        let group = |s: String| {
            s.as_bytes()
                .chunks(4)
                .map(|c| String::from_utf8_lossy(c).into_owned())
                .collect::<Vec<_>>()
                .join(" ")
        };
        let body = self.body();
        let check = Self::check_for(&body);
        Secret::new(format!(
            "hy1-{}-{}-{}-{}-{check}",
            self.host,
            self.port,
            group(crockford().encode(&self.fingerprint)),
            group(crockford().encode(self.secret.expose()))
        ))
    }

    /// Read a pasted or typed slip.
    ///
    /// # Errors
    /// [`SlipError::Mistyped`] if only the checksum fails; [`SlipError::Malformed`] for
    /// anything else.
    pub fn parse(text: &str) -> Result<Self, SlipError> {
        let compact: Vec<char> = text.chars().filter(|c| !c.is_whitespace()).collect();
        if compact.len() > MAX_INPUT {
            return Err(SlipError::Malformed);
        }
        let mut end = compact.len();
        // Collect `n` symbols from the right, skipping hyphens inside the group, then
        // require one hyphen before the group.
        let mut take = |n: usize| -> Result<String, SlipError> {
            while end > 0 && compact[end - 1] == '-' {
                end -= 1;
            }
            let mut out = Vec::with_capacity(n);
            while out.len() < n {
                if end == 0 {
                    return Err(SlipError::Malformed);
                }
                end -= 1;
                match compact[end] {
                    '-' => {}
                    c => out.push(symbol(c).ok_or(SlipError::Malformed)?),
                }
            }
            if end == 0 || compact[end - 1] != '-' {
                return Err(SlipError::Malformed);
            }
            end -= 1;
            Ok(out.into_iter().rev().collect())
        };
        let check = take(CHECK_CHARS)?;
        let secret = take(SECRET_CHARS)?;
        let fp = take(FP_CHARS)?;
        let rest: String = compact[..end].iter().collect();
        let (prefix_host, port) = rest.rsplit_once('-').ok_or(SlipError::Malformed)?;
        if port.is_empty() || port.len() > 5 || !port.bytes().all(|b| b.is_ascii_digit()) {
            return Err(SlipError::Malformed);
        }
        let port: u16 = port.parse().map_err(|_| SlipError::Malformed)?;
        let host = match prefix_host.get(..4) {
            Some(p) if p.eq_ignore_ascii_case("hy1-") => &prefix_host[4..],
            _ => return Err(SlipError::Malformed),
        };
        let fingerprint: [u8; 32] = crockford()
            .decode(fp.as_bytes())
            .ok()
            .and_then(|v| v.try_into().ok())
            .ok_or(SlipError::Malformed)?;
        let secret_bytes: [u8; SLIP_SECRET_LEN] = crockford()
            .decode(secret.as_bytes())
            .ok()
            .and_then(|v| v.try_into().ok())
            .ok_or(SlipError::Malformed)?;
        let slip = Self::new(host, port, fingerprint, secret_bytes)?;
        if Self::check_for(&slip.body()) != check {
            return Err(SlipError::Mistyped);
        }
        Ok(slip)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn sample() -> Slip {
        let mut fp = [0u8; 32];
        for (i, b) in fp.iter_mut().enumerate() {
            *b = u8::try_from(i * 7 % 256).unwrap();
        }
        Slip::new("office-pc-01", 8443, fp, [0xA5; SLIP_SECRET_LEN]).unwrap()
    }

    #[test]
    fn canonical_round_trips_and_has_the_documented_shape() {
        let s = sample();
        let text = s.canonical().expose().clone();
        let parts: Vec<&str> = text.rsplitn(4, '-').collect();
        assert_eq!(parts[0].len(), 4);
        assert_eq!(parts[1].len(), 16);
        assert_eq!(parts[2].len(), 52);
        assert!(parts[3].starts_with("hy1-office-pc-01-8443"));
        let back = Slip::parse(&text).unwrap();
        assert_eq!(back.host, "office-pc-01");
        assert_eq!(back.port, 8443);
        assert_eq!(back.fingerprint, s.fingerprint);
        assert_eq!(back.secret.expose(), s.secret.expose());
        // About 90 characters with a 12-character host.
        assert!((80..=100).contains(&text.len()), "{}", text.len());
    }

    #[test]
    fn printed_lower_case_and_look_alike_input_parses() {
        let s = sample();
        let printed = s.printable().expose().clone();
        assert!(printed.contains(' '));
        assert_eq!(
            Slip::parse(&printed).unwrap().fingerprint,
            s.fingerprint,
            "groups of four with spaces"
        );
        let lower = s.canonical().expose().to_lowercase();
        assert!(Slip::parse(&lower).is_ok());
        // Look-alikes in the base32 groups (the host and port stay as typed).
        let canon = s.canonical().expose().clone();
        let (head, tail) = canon.split_at(canon.find("-8443-").unwrap() + 6);
        let tail_looks = tail.replace('0', "o").replace('1', "I");
        assert!(Slip::parse(&format!("{head}{tail_looks}")).is_ok());
        // Hyphens inside the fixed groups are ignored.
        let tail_hyph: String = tail
            .chars()
            .enumerate()
            .flat_map(|(i, c)| {
                if i > 0 && i % 4 == 0 && c != '-' {
                    vec!['-', c]
                } else {
                    vec![c]
                }
            })
            .collect();
        assert!(Slip::parse(&format!("{head}{tail_hyph}")).is_ok());
    }

    #[test]
    fn hyphenated_and_ip_hosts_parse_right_to_left() {
        let mut s = sample();
        for host in ["a-b-c-d", "192.168.1.20", "srv.local", "x"] {
            s.host = host.into();
            let back = Slip::parse(s.canonical().expose()).unwrap();
            assert_eq!(back.host, host);
        }
    }

    #[test]
    fn a_typo_is_mistyped_not_a_wrong_server() {
        let text = sample().canonical().expose().clone();
        // Change one symbol of the secret.
        let pos = text.len() - 6;
        let c = text.as_bytes()[pos] as char;
        let other = if c == 'Z' { 'Y' } else { 'Z' };
        let bad = format!("{}{other}{}", &text[..pos], &text[pos + 1..]);
        assert_eq!(Slip::parse(&bad).unwrap_err(), SlipError::Mistyped);
        // A changed host is also only a typo.
        let bad_host = text.replacen("office", "offize", 1);
        assert_eq!(Slip::parse(&bad_host).unwrap_err(), SlipError::Mistyped);
    }

    #[test]
    fn malformed_slips_are_refused() {
        let text = sample().canonical().expose().clone();
        for bad in [
            String::new(),
            "hello".into(),
            text.replacen("hy1-", "hy2-", 1),
            text.replacen("-8443-", "-0-", 1),
            text.replacen("-8443-", "-99999-", 1),
            text.replacen("-8443-", "-84a3-", 1),
            text.replacen("office-pc-01", "off_ice", 1),
            text.replacen("office-pc-01", "", 1),
            text[..text.len() - 1].to_string(),
            format!("{text}0"),
            format!("{}U", &text[..text.len() - 1]),
            "x".repeat(600),
        ] {
            assert!(Slip::parse(&bad).is_err(), "accepted {bad:?}");
        }
        assert!(Slip::new("bad host", 1, [0; 32], [0; 10]).is_err());
        assert!(Slip::new("ok", 0, [0; 32], [0; 10]).is_err());
    }

    #[test]
    fn every_single_symbol_change_is_caught() {
        // Exhaustive over every position of the three groups and every other symbol.
        let text = sample().canonical().expose().clone();
        let groups_start = text.find("-8443-").unwrap() + 6;
        let chars: Vec<char> = text.chars().collect();
        for p in (groups_start..chars.len()).filter(|&i| chars[i] != '-') {
            for new in ALPHABET.chars().filter(|&c| c != chars[p]) {
                let mut changed = chars.clone();
                changed[p] = new;
                let bad: String = changed.into_iter().collect();
                assert!(Slip::parse(&bad).is_err(), "accepted a change at {p}");
            }
        }
    }

    #[test]
    fn secrets_round_trip_on_the_wire() {
        let s = [0x5Au8; SLIP_SECRET_LEN];
        let text = encode_secret(&s);
        assert_eq!(text.expose().len(), 16);
        assert_eq!(decode_secret(text.expose()).unwrap().expose(), &s);
        assert!(decode_secret("short").is_none());
        assert!(decode_secret(&"U".repeat(16)).is_none());
    }

    #[test]
    fn leading_bits_code_reads_bits_in_order() {
        assert_eq!(leading_bits_code(&[0, 0, 0], 4), "0000");
        assert_eq!(leading_bits_code(&[0xFF, 0xFF, 0xF0], 4), "ZZZZ");
        // 00001 00010 00011 00100 = 0x08 0x86 0x40
        assert_eq!(leading_bits_code(&[0x08, 0x86, 0x40], 4), "1234");
    }

    proptest! {
        // T-S25 (property part): the parser never panics, and anything it accepts
        // re-encodes to an equivalent slip.
        #[test]
        fn parse_never_panics(text in "\\PC{0,200}") {
            if let Ok(s) = Slip::parse(&text) {
                let again = Slip::parse(s.canonical().expose()).unwrap();
                prop_assert_eq!(again.fingerprint, s.fingerprint);
            }
        }

        #[test]
        fn every_slip_round_trips(
            host in "[a-z0-9]([a-z0-9.-]{0,30}[a-z0-9])?",
            port in 1u16..,
            fp in any::<[u8; 32]>(),
            secret in any::<[u8; 10]>(),
        ) {
            let s = Slip::new(&host, port, fp, secret).unwrap();
            for text in [s.canonical().expose().clone(), s.printable().expose().clone()] {
                let back = Slip::parse(&text).unwrap();
                prop_assert_eq!(&back.host, &s.host);
                prop_assert_eq!(back.port, port);
                prop_assert_eq!(back.fingerprint, fp);
                prop_assert_eq!(back.secret.expose(), &secret);
            }
        }
    }
}
