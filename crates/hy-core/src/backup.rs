//! Full personal backup (architecture 8; wireframe W9): one `.hybackup` file, encrypted
//! with a password the user chooses.
//!
//! The file is a fixed header followed by the AES-256-GCM ciphertext of the backup's
//! JSON ([`Contents`]). The key comes from the password with Argon2id; the header holds
//! the Argon2id cost, the salt and the nonce, and is authenticated as associated data,
//! so a changed header, a changed byte of ciphertext or a wrong password all fail the
//! same way and nothing is read.
//!
//! ```text
//! "HYBACKUP" | version (1 byte) | memory KiB, passes, lanes (u32 LE each) | salt (16) | nonce (12) | ciphertext
//! ```
//!
//! What the file holds and how an import treats it is the store's part
//! ([`crate::store::Store::export_backup`] and [`crate::store::Store::import_backup`]).

use std::collections::BTreeMap;

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{Error, Result};

/// The file extension, without the dot.
pub const EXTENSION: &str = "hybackup";
/// The version of [`Contents`] this build writes and reads.
pub const FORMAT_VERSION: u32 = 1;
/// Passwords have at least this many characters.
pub const MIN_PASSWORD_CHARS: usize = 8;
/// Files larger than this are refused before anything is decrypted.
pub const MAX_FILE_BYTES: usize = 64 * 1024 * 1024;

const MAGIC: &[u8; 8] = b"HYBACKUP";
const FILE_VERSION: u8 = 1;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const HEADER_LEN: usize = MAGIC.len() + 1 + 12 + SALT_LEN + NONCE_LEN;

/// The Argon2id cost recorded in each file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KdfParams {
    /// Memory, KiB.
    pub memory_kib: u32,
    /// Passes over the memory.
    pub passes: u32,
    /// Parallel lanes.
    pub lanes: u32,
}

impl KdfParams {
    /// What exports use: 64 MiB, 3 passes, 1 lane (D).
    pub const DEFAULT: Self = Self {
        memory_kib: 64 * 1024,
        passes: 3,
        lanes: 1,
    };

    /// A file asking for more than this is refused, so opening a file cannot exhaust the
    /// PC.
    const MAX: Self = Self {
        memory_kib: 1024 * 1024,
        passes: 16,
        lanes: 8,
    };

    fn argon2(self) -> Result<Argon2<'static>> {
        if self.memory_kib > Self::MAX.memory_kib
            || self.passes > Self::MAX.passes
            || self.lanes > Self::MAX.lanes
        {
            return Err(Error::Backup(
                "the file asks for more memory or time than allowed".into(),
            ));
        }
        let params = Params::new(self.memory_kib, self.passes, self.lanes, Some(32))
            .map_err(|e| Error::Backup(format!("key settings: {e}")))?;
        Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
    }
}

/// Check a new backup password.
///
/// # Errors
/// [`Error::Invalid`] if it is shorter than [`MIN_PASSWORD_CHARS`] characters.
pub fn check_password(password: &str) -> Result<()> {
    if password.chars().count() < MIN_PASSWORD_CHARS {
        return Err(Error::invalid(format!(
            "use a password of at least {MIN_PASSWORD_CHARS} characters"
        )));
    }
    Ok(())
}

fn derive_key(password: &str, salt: &[u8], params: KdfParams) -> Result<[u8; 32]> {
    let mut key = [0u8; 32];
    params
        .argon2()?
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|e| Error::Backup(format!("key: {e}")))?;
    Ok(key)
}

/// Encrypt `plain` into a backup file.
///
/// # Errors
/// [`Error::Invalid`] for a password that is too short; [`Error::Backup`] if encryption
/// fails.
///
/// # Panics
/// If the operating system's random source fails.
pub fn seal(plain: &[u8], password: &str, params: KdfParams) -> Result<Vec<u8>> {
    check_password(password)?;
    let mut salt = [0u8; SALT_LEN];
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::fill(&mut salt).expect("operating system random source failed");
    getrandom::fill(&mut nonce).expect("operating system random source failed");
    let mut out = Vec::with_capacity(HEADER_LEN + plain.len() + 16);
    out.extend_from_slice(MAGIC);
    out.push(FILE_VERSION);
    out.extend_from_slice(&params.memory_kib.to_le_bytes());
    out.extend_from_slice(&params.passes.to_le_bytes());
    out.extend_from_slice(&params.lanes.to_le_bytes());
    out.extend_from_slice(&salt);
    out.extend_from_slice(&nonce);
    let key = derive_key(password, &salt, params)?;
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
    let sealed = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: plain,
                aad: &out,
            },
        )
        .map_err(|_| Error::Backup("encryption failed".into()))?;
    out.extend_from_slice(&sealed);
    Ok(out)
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    let mut four = [0u8; 4];
    four.copy_from_slice(&b[at..at + 4]);
    u32::from_le_bytes(four)
}

/// Decrypt a backup file.
///
/// # Errors
/// [`Error::Backup`] if the file is too large, is not a HealthierYou backup, was made by
/// a newer version, or the password is wrong or the file was changed (these last two
/// cannot be told apart).
pub fn open(file: &[u8], password: &str) -> Result<Vec<u8>> {
    if file.len() > MAX_FILE_BYTES {
        return Err(Error::Backup("the file is too large to be a backup".into()));
    }
    if file.len() < HEADER_LEN + 16 || &file[..MAGIC.len()] != MAGIC {
        return Err(Error::Backup(
            "this is not a HealthierYou backup file".into(),
        ));
    }
    if file[MAGIC.len()] != FILE_VERSION {
        return Err(Error::Backup(
            "the backup was made by a newer version of HealthierYou".into(),
        ));
    }
    let p = MAGIC.len() + 1;
    let params = KdfParams {
        memory_kib: u32_at(file, p),
        passes: u32_at(file, p + 4),
        lanes: u32_at(file, p + 8),
    };
    let salt = &file[p + 12..p + 12 + SALT_LEN];
    let nonce = &file[p + 12 + SALT_LEN..HEADER_LEN];
    let key = derive_key(password, salt, params)?;
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
    cipher
        .decrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: &file[HEADER_LEN..],
                aad: &file[..HEADER_LEN],
            },
        )
        .map_err(|_| {
            Error::Backup("the password is wrong, or the file was damaged or changed".into())
        })
}

/// One table's rows, as columns and values (`null`, a number or text).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Table {
    /// Column names.
    pub columns: Vec<String>,
    /// One array of values per row, in column order.
    pub rows: Vec<Vec<serde_json::Value>>,
}

/// What a backup file holds once decrypted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Contents {
    /// [`FORMAT_VERSION`].
    pub format: u32,
    /// The schema version of the database it came from.
    pub schema_version: u32,
    /// The app version that wrote it.
    pub app_version: String,
    /// When it was written.
    pub exported_at: DateTime<Utc>,
    /// The PC it was written on. Every row keeps its own origin device as well.
    pub origin_device: Uuid,
    /// When that PC's store was created (Unix ms), for the first day of use.
    pub store_created_at: Option<i64>,
    /// The tables, by name.
    pub tables: BTreeMap<String, Table>,
}

impl Contents {
    /// Encode and encrypt.
    ///
    /// # Errors
    /// See [`seal`].
    pub fn seal(&self, password: &str, params: KdfParams) -> Result<Vec<u8>> {
        seal(&serde_json::to_vec(self)?, password, params)
    }

    /// Decrypt and decode.
    ///
    /// # Errors
    /// See [`open`]; [`Error::Backup`] if the contents are not a backup this build reads.
    pub fn open(file: &[u8], password: &str) -> Result<Self> {
        let plain = open(file, password)?;
        let c: Self = serde_json::from_slice(&plain)
            .map_err(|e| Error::Backup(format!("the backup's contents cannot be read: {e}")))?;
        if c.format != FORMAT_VERSION {
            return Err(Error::Backup(
                "the backup was made by a newer version of HealthierYou".into(),
            ));
        }
        Ok(c)
    }

    /// The number of rows of `table`.
    #[must_use]
    pub fn rows(&self, table: &str) -> usize {
        self.tables.get(table).map_or(0, |t| t.rows.len())
    }
}

/// What a backup holds, in words the screens show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Summary {
    /// When it was written.
    pub exported_at: DateTime<Utc>,
    /// Events (water, routines, reminders, notes).
    pub events: usize,
    /// Days with tracker tallies.
    pub days: usize,
    /// The earliest and latest dates with data, if any.
    pub from: Option<NaiveDate>,
    /// See `from`.
    pub to: Option<NaiveDate>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAST: KdfParams = KdfParams {
        memory_kib: 64,
        passes: 1,
        lanes: 1,
    };

    #[test]
    fn a_sealed_file_opens_only_with_its_password_and_unchanged() {
        let file = seal(b"{\"hello\":1}", "correct horse", FAST).unwrap();
        assert_eq!(&file[..8], b"HYBACKUP");
        assert_eq!(open(&file, "correct horse").unwrap(), b"{\"hello\":1}");
        assert!(matches!(open(&file, "wrong horse!"), Err(Error::Backup(_))));
        // Any changed byte, in the header or the ciphertext, is refused.
        for at in [9, 20, 30, file.len() - 1] {
            let mut bad = file.clone();
            bad[at] ^= 1;
            assert!(matches!(open(&bad, "correct horse"), Err(Error::Backup(_))));
        }
        assert!(matches!(open(b"not a backup", "x"), Err(Error::Backup(_))));
        // Two files of the same contents differ (fresh salt and nonce).
        assert_ne!(file, seal(b"{\"hello\":1}", "correct horse", FAST).unwrap());
    }

    #[test]
    fn passwords_and_costs_are_bounded() {
        assert!(matches!(seal(b"x", "short", FAST), Err(Error::Invalid(_))));
        let mut file = seal(b"x", "long enough", FAST).unwrap();
        // A file asking for 4 GiB of memory is refused before any work.
        file[9..13].copy_from_slice(&(4 * 1024 * 1024u32).to_le_bytes());
        assert!(matches!(open(&file, "long enough"), Err(Error::Backup(_))));
        let mut newer = seal(b"x", "long enough", FAST).unwrap();
        newer[8] = 2;
        assert!(matches!(open(&newer, "long enough"), Err(Error::Backup(_))));
    }
}
