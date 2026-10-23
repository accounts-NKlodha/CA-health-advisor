//! The off-volume mirror of every user's control log (V1-5 contract 7.3; architecture
//! 5.3, "Atomic ordering" and "Mirror location").
//!
//! ```text
//! HYM1-HEADER <mirror_id hex> <key_id hex> <created_at ms>
//! HYM1 <user_id hex> <seq> <at_ms> <kind> <body b64url> <prev hex> <hash hex>
//! ```
//!
//! One line per entry, written whole and then fsynced, after the database commit and
//! before the client is acknowledged. The file holds no payloads, aliases, labels or
//! addresses: only random ids, seq numbers, times and fixed-width entry bodies. It is not
//! encrypted, so the recovery path stays inspectable.
//!
//! [`scan`] reads the whole file and checks it: the header, every line's syntax, and each
//! user's chain from its genesis (seq, previous hash and recomputed hash). A final line
//! with no newline is a torn write (power loss during the fsync, T-J4) and is reported so
//! [`Mirror::open`] can truncate it; any other damage is [`MirrorError::Corrupt`].

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use hy_core::protocol::chain::{ControlEntry, EntryBody, genesis};
use hy_core::protocol::{b64, hex, unb64};

use crate::chain::StoredEntry;

const HEADER: &str = "HYM1-HEADER";
const RECORD: &str = "HYM1";

/// Why a mirror cannot be used. Each leads to the global discard path at startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirrorError {
    /// The file does not exist.
    Missing,
    /// The file or its folder cannot be read (permissions, an offline share). Nothing is
    /// proven lost: the server stays stopped with its data intact until it can be read.
    Unreadable,
    /// A line is malformed, or a chain does not verify (flipped byte, cut in the middle,
    /// reordered lines).
    Corrupt,
    /// The header names another install or another head key.
    Foreign,
}

impl std::fmt::Display for MirrorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Missing => "mirror_missing",
            Self::Unreadable => "mirror_unreadable",
            Self::Corrupt => "mirror_corrupt",
            Self::Foreign => "mirror_foreign",
        })
    }
}

/// The parsed header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    /// Random, created at install, also stored in the database.
    pub mirror_id: [u8; 16],
    /// The head-signing key's id.
    pub key_id: [u8; 32],
    /// When the file was created (ms).
    pub created_at: i64,
}

/// What [`scan`] found.
#[derive(Debug, Clone)]
pub struct Scan {
    /// The header.
    pub header: Header,
    /// Every complete entry, in file order.
    pub entries: Vec<StoredEntry>,
    /// Each user's tail: `(seq, hash)`.
    pub tails: HashMap<[u8; 16], (u64, [u8; 32])>,
    /// Byte length of the complete lines; a longer file ends in a torn line.
    pub good_len: u64,
    /// The file ends in a torn line.
    pub torn: bool,
}

impl Scan {
    /// The user's mirrored tail, `(0, genesis)` when absent.
    #[must_use]
    pub fn tail(&self, user: &[u8; 16]) -> (u64, [u8; 32]) {
        self.tails
            .get(user)
            .copied()
            .unwrap_or_else(|| (0, genesis(&self.header.key_id, user)))
    }

    /// The hash of the user's entry at `seq`, if mirrored.
    #[must_use]
    pub fn hash_at(&self, user: &[u8; 16], seq: u64) -> Option<[u8; 32]> {
        if seq == 0 {
            return Some(genesis(&self.header.key_id, user));
        }
        self.entries
            .iter()
            .find(|e| &e.user_id == user && e.seq == seq)
            .map(|e| e.hash)
    }
}

fn parse_hex<const N: usize>(s: &str) -> Option<[u8; N]> {
    if s.len() != 2 * N {
        return None;
    }
    let v = data_encoding::HEXLOWER.decode(s.as_bytes()).ok()?;
    v.try_into().ok()
}

fn parse_header(line: &str) -> Option<Header> {
    let mut f = line.split(' ');
    let (Some(HEADER), Some(id), Some(key), Some(at), None) =
        (f.next(), f.next(), f.next(), f.next(), f.next())
    else {
        return None;
    };
    Some(Header {
        mirror_id: parse_hex(id)?,
        key_id: parse_hex(key)?,
        created_at: at.parse().ok()?,
    })
}

fn parse_record(line: &str) -> Option<StoredEntry> {
    let f: Vec<&str> = line.split(' ').collect();
    let [RECORD, user, seq, at, kind, body, prev, hash] = f.as_slice() else {
        return None;
    };
    // Canonical decimal only, so a re-encoded line is byte-identical.
    let num = |s: &str| -> Option<u64> {
        let n: u64 = s.parse().ok()?;
        (n.to_string() == s).then_some(n)
    };
    let body = unb64(body, 256)?;
    let e = StoredEntry {
        user_id: parse_hex(user)?,
        seq: num(seq)?,
        at_ms: num(at)?,
        kind: u8::try_from(num(kind)?).ok()?,
        body,
        prev: parse_hex(prev)?,
        hash: parse_hex(hash)?,
    };
    (format_record(&e) == format!("{line}\n")).then_some(e)
}

/// One mirror line for an entry, with its newline.
#[must_use]
pub fn format_record(e: &StoredEntry) -> String {
    format!(
        "{RECORD} {} {} {} {} {} {} {}\n",
        hex(&e.user_id),
        e.seq,
        e.at_ms,
        e.kind,
        b64(&e.body),
        hex(&e.prev),
        hex(&e.hash)
    )
}

fn format_header(h: &Header) -> String {
    format!(
        "{HEADER} {} {} {}\n",
        hex(&h.mirror_id),
        hex(&h.key_id),
        h.created_at
    )
}

/// Read and check the mirror at `path` against the server's `key_id`.
///
/// # Errors
/// [`MirrorError`] as documented on each variant.
pub fn scan(path: &Path, key_id: &[u8; 32]) -> Result<Scan, MirrorError> {
    let mut bytes = Vec::new();
    match File::open(path) {
        Ok(mut f) => f
            .read_to_end(&mut bytes)
            .map_err(|_| MirrorError::Unreadable)?,
        // Missing only when its folder can be listed and the file is not there; a folder
        // that cannot be reached (an offline share) is unreadable, never "missing".
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let folder = path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."));
            return Err(if std::fs::read_dir(folder).is_ok() {
                MirrorError::Missing
            } else {
                MirrorError::Unreadable
            });
        }
        Err(_) => return Err(MirrorError::Unreadable),
    };
    let text = std::str::from_utf8(&bytes).map_err(|_| MirrorError::Corrupt)?;
    let good_len = text.rfind('\n').map_or(0, |i| i + 1);
    let torn = good_len < text.len();
    let mut lines = text[..good_len].lines();
    let header = lines
        .next()
        .and_then(parse_header)
        .ok_or(MirrorError::Corrupt)?;
    if &header.key_id != key_id {
        return Err(MirrorError::Foreign);
    }
    let mut tails: HashMap<[u8; 16], (u64, [u8; 32])> = HashMap::new();
    let mut entries = Vec::new();
    for line in lines {
        let e = parse_record(line).ok_or(MirrorError::Corrupt)?;
        let (seq, prev) = tails
            .get(&e.user_id)
            .copied()
            .unwrap_or_else(|| (0, genesis(key_id, &e.user_id)));
        let body = EntryBody::decode(e.kind, &e.body).map_err(|_| MirrorError::Corrupt)?;
        let entry = ControlEntry {
            user_id: e.user_id,
            seq: e.seq,
            at_ms: e.at_ms,
            body,
        };
        if e.seq != seq + 1 || e.prev != prev || entry.hash(&prev) != e.hash {
            return Err(MirrorError::Corrupt);
        }
        tails.insert(e.user_id, (e.seq, e.hash));
        entries.push(e);
    }
    Ok(Scan {
        header,
        entries,
        tails,
        good_len: good_len as u64,
        torn,
    })
}

/// A fault the tests inject into mirror writes (T-J crash points, contract 12 "mirror
/// unavailable"). No setting, flag or API of the product reaches it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirrorFault {
    /// Every write fails (the share is offline).
    Unavailable,
    /// The next write stops half way through a line (power loss during the fsync).
    TornWrite,
}

/// The open mirror.
#[derive(Debug)]
pub struct Mirror {
    path: PathBuf,
    file: File,
    header: Header,
    tails: HashMap<[u8; 16], (u64, [u8; 32])>,
    /// Set after a failed write: the next append first re-reads and repairs the file.
    needs_repair: bool,
    /// Injected by tests only.
    pub fault: Option<MirrorFault>,
}

fn sync_dir(path: &Path) {
    // Make the new file's directory entry durable too (best effort; not on Windows).
    #[cfg(unix)]
    if let Some(dir) = path.parent()
        && let Ok(d) = File::open(dir)
    {
        let _ = d.sync_all();
    }
    #[cfg(not(unix))]
    let _ = path;
}

impl Mirror {
    /// Create a new mirror file holding only its header. An existing file is refused.
    ///
    /// # Errors
    /// Any I/O error.
    pub fn create(
        path: &Path,
        mirror_id: [u8; 16],
        key_id: [u8; 32],
        created_at: i64,
    ) -> io::Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let header = Header {
            mirror_id,
            key_id,
            created_at,
        };
        let mut file = OpenOptions::new()
            .create_new(true)
            .append(true)
            .open(path)?;
        file.write_all(format_header(&header).as_bytes())?;
        file.sync_all()?;
        sync_dir(path);
        Ok(Self {
            path: path.to_path_buf(),
            file,
            header,
            tails: HashMap::new(),
            needs_repair: false,
            fault: None,
        })
    }

    /// Open a scanned mirror for appending, truncating a torn final line first.
    ///
    /// # Errors
    /// Any I/O error.
    pub fn open(path: &Path, scan: &Scan) -> io::Result<Self> {
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        if scan.torn {
            file.set_len(scan.good_len)?;
            file.sync_all()?;
        }
        drop(file);
        let file = OpenOptions::new().append(true).open(path)?;
        Ok(Self {
            path: path.to_path_buf(),
            file,
            header: scan.header,
            tails: scan.tails.clone(),
            needs_repair: false,
            fault: None,
        })
    }

    /// The header.
    #[must_use]
    pub fn header(&self) -> Header {
        self.header
    }

    /// The file's path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The user's mirrored seq (0 when none).
    #[must_use]
    pub fn tail_seq(&self, user: &[u8; 16]) -> u64 {
        self.tails.get(user).map_or(0, |t| t.0)
    }

    fn repair(&mut self) -> io::Result<()> {
        let s =
            scan(&self.path, &self.header.key_id).map_err(|e| io::Error::other(e.to_string()))?;
        if s.header != self.header {
            return Err(io::Error::other("mirror header changed"));
        }
        *self = Self {
            fault: self.fault,
            ..Self::open(&self.path, &s)?
        };
        Ok(())
    }

    /// Append entries (each continuing its user's mirrored chain), then fsync. Entries at
    /// or below a user's mirrored seq are skipped, so a caller can resend a batch after a
    /// failed write.
    ///
    /// With no entries it still checks that the mirror can be written (an fsync), so a
    /// caller can refuse to record anything while the share is offline (contract 12).
    ///
    /// # Errors
    /// An I/O error, or entries that do not continue the mirrored chains. After an error
    /// the next call re-reads the file and truncates any partial line first.
    pub fn append(&mut self, entries: &[StoredEntry]) -> io::Result<()> {
        if self.fault == Some(MirrorFault::Unavailable) {
            self.needs_repair = true;
            return Err(io::Error::other("mirror unavailable (injected)"));
        }
        if entries.is_empty() {
            return self.file.sync_data();
        }
        if self.needs_repair {
            self.repair()?;
        }
        let mut tails = self.tails.clone();
        let mut text = String::new();
        for e in entries {
            let (seq, prev) = tails
                .get(&e.user_id)
                .copied()
                .unwrap_or_else(|| (0, genesis(&self.header.key_id, &e.user_id)));
            // Already there (a repaired torn write had completed this line): skip it;
            // the next entry must still continue the mirrored hash.
            if e.seq <= seq {
                continue;
            }
            if e.seq != seq + 1 || e.prev != prev {
                return Err(io::Error::other("entry does not continue the mirror"));
            }
            tails.insert(e.user_id, (e.seq, e.hash));
            text.push_str(&format_record(e));
        }
        if self.fault == Some(MirrorFault::TornWrite) {
            self.fault = None;
            self.needs_repair = true;
            // Stop inside the last line (never on a line boundary).
            let cut = text.len().saturating_sub(5);
            let _ = self.file.write_all(&text.as_bytes()[..cut]);
            let _ = self.file.sync_data();
            return Err(io::Error::other("torn write (injected)"));
        }
        let res = self
            .file
            .write_all(text.as_bytes())
            .and_then(|()| self.file.sync_data());
        if let Err(e) = res {
            self.needs_repair = true;
            return Err(e);
        }
        self.tails = tails;
        Ok(())
    }
}

/// Why a mirror location was refused at service start (contract 7.3: the mirror lives
/// on another volume than the data folder). Anything that cannot be verified is refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VolumeError {
    /// The mirror's folder does not exist or cannot be reached (an offline share, or a
    /// folder never created): create it on the other volume first.
    #[error(
        "the mirror folder does not exist or cannot be reached; create it on another volume first"
    )]
    Unreachable,
    /// The mirror is inside the data folder.
    #[error("the mirror must not be inside the data folder")]
    InsideData,
    /// The mirror is on the data folder's volume.
    #[error("the mirror must be on another volume than the data folder")]
    SameVolume,
    /// The volumes could not be compared.
    #[error("the mirror's volume could not be verified")]
    Unverifiable,
}

/// An opaque volume identity: the mount device on Unix, the drive or share prefix on
/// Windows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeId(String);

impl VolumeId {
    /// A volume identity with this name, for tests of [`check_volume_with`].
    #[doc(hidden)]
    #[must_use]
    pub fn for_tests(name: &str) -> Self {
        Self(name.to_owned())
    }
}

/// The volume of an existing, canonical path.
///
/// # Errors
/// The I/O error if it cannot be read.
pub fn volume_of(path: &Path) -> io::Result<VolumeId> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(VolumeId(std::fs::metadata(path)?.dev().to_string()))
    }
    #[cfg(windows)]
    {
        use std::path::Component;
        std::fs::metadata(path)?;
        match path.components().next() {
            Some(Component::Prefix(x)) => Ok(VolumeId(
                x.as_os_str().to_string_lossy().to_ascii_lowercase(),
            )),
            _ => Err(io::Error::other("no volume prefix")),
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        Err(io::Error::other("unsupported platform"))
    }
}

/// Check the mirror location before the service starts: its folder must already exist
/// (it is never created here, so a mistyped or offline share is not silently replaced
/// by a new folder on the data drive), lie outside the data folder, and be on another
/// volume. Fails closed when anything cannot be verified.
///
/// # Errors
/// [`VolumeError`].
pub fn check_volume(data: &Path, mirror: &Path) -> Result<(), VolumeError> {
    check_volume_with(data, mirror, volume_of)
}

/// As [`check_volume`] with the volume lookup supplied (tests).
///
/// # Errors
/// [`VolumeError`].
#[doc(hidden)]
pub fn check_volume_with(
    data: &Path,
    mirror: &Path,
    volume: impl Fn(&Path) -> io::Result<VolumeId>,
) -> Result<(), VolumeError> {
    let parent = mirror
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or(VolumeError::Unreachable)?;
    let m = parent
        .canonicalize()
        .map_err(|_| VolumeError::Unreachable)?;
    if !m.is_dir() {
        return Err(VolumeError::Unreachable);
    }
    let d = data.canonicalize().map_err(|_| VolumeError::Unverifiable)?;
    if m.starts_with(&d) {
        return Err(VolumeError::InsideData);
    }
    match (volume(&d), volume(&m)) {
        (Ok(a), Ok(b)) if a != b => Ok(()),
        (Ok(_), Ok(_)) => Err(VolumeError::SameVolume),
        _ => Err(VolumeError::Unverifiable),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hy_core::protocol::chain::EntryBody;

    fn chain(key: &[u8; 32], user: [u8; 16], n: u64) -> Vec<StoredEntry> {
        let mut prev = genesis(key, &user);
        (1..=n)
            .map(|seq| {
                let body = EntryBody::DeviceConfirmed {
                    device_ref: [seq as u8; 16],
                };
                let e = ControlEntry {
                    user_id: user,
                    seq,
                    at_ms: 1000 + seq,
                    body,
                };
                let hash = e.hash(&prev);
                let s = StoredEntry {
                    user_id: user,
                    seq,
                    at_ms: e.at_ms,
                    kind: body.kind(),
                    body: body.encode(),
                    prev,
                    hash,
                };
                prev = hash;
                s
            })
            .collect()
    }

    #[test]
    fn appended_entries_scan_back_and_tampering_is_caught() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("control.mirror");
        let key = [9u8; 32];
        let mut m = Mirror::create(&path, [1; 16], key, 5).unwrap();
        let a = chain(&key, [2; 16], 3);
        let b = chain(&key, [3; 16], 2);
        m.append(&a[..2]).unwrap();
        m.append(&b).unwrap();
        m.append(&a[2..]).unwrap();
        // Already-mirrored entries are skipped; a gap is refused before anything is
        // written.
        m.append(&a[..1]).unwrap();
        let c = chain(&key, [5; 16], 3);
        assert!(m.append(&c[1..]).is_err());
        let s = scan(&path, &key).unwrap();
        assert_eq!(s.entries.len(), 5);
        assert_eq!(s.tail(&[2; 16]).0, 3);
        assert_eq!(s.tail(&[3; 16]).0, 2);
        assert!(!s.torn);
        assert_eq!(scan(&path, &[8; 32]).unwrap_err(), MirrorError::Foreign);

        let original = std::fs::read(&path).unwrap();
        // One flipped byte anywhere in a record line.
        let first_record = original.iter().position(|&c| c == b'\n').unwrap() + 1;
        for i in (first_record..original.len() - 1).step_by(7) {
            if original[i] == b'\n' {
                continue;
            }
            let mut bad = original.clone();
            bad[i] ^= 0x01;
            std::fs::write(&path, &bad).unwrap();
            assert!(scan(&path, &key).is_err(), "byte {i}");
        }
        // Two lines swapped.
        let text = String::from_utf8(original.clone()).unwrap();
        let mut lines: Vec<&str> = text.lines().collect();
        lines.swap(1, 2);
        std::fs::write(&path, lines.join("\n") + "\n").unwrap();
        assert_eq!(scan(&path, &key).unwrap_err(), MirrorError::Corrupt);
        // A torn final line is reported, not an error, and open() truncates it.
        let mut torn = original.clone();
        torn.extend_from_slice(b"HYM1 0303");
        std::fs::write(&path, &torn).unwrap();
        let s = scan(&path, &key).unwrap();
        assert!(s.torn);
        let mut m = Mirror::open(&path, &s).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert_eq!(m.tail_seq(&[2; 16]), 3);
        // An injected torn write is repaired by the next append.
        let more = chain(&key, [4; 16], 2);
        m.fault = Some(MirrorFault::TornWrite);
        assert!(m.append(&more).is_err());
        assert!(scan(&path, &key).unwrap().torn);
        m.append(&more).unwrap();
        let s = scan(&path, &key).unwrap();
        assert!(!s.torn);
        assert_eq!(s.tail(&[4; 16]).0, 2);
        assert_eq!(
            scan(&dir.path().join("none"), &key).unwrap_err(),
            MirrorError::Missing
        );
    }
}
