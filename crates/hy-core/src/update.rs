//! Signed update files, rollback and the update record (architecture 8; slice V1-6).
//!
//! Every V1 update arrives as a file: the NSIS installer of a release and its `.sig`,
//! copied from a LAN share or a USB stick. Nothing runs before the signature is checked.
//!
//! - **Signature.** The `.sig` is a minisign signature, written by the Tauri CLI's
//!   `signer sign --app-version <version>` on the owner's PC (the secret key never leaves
//!   it). It is checked with [`minisign_verify`] against the public keys built into the
//!   app ([`TrustedKeys`]). The installer is hashed in pre-hashed mode (BLAKE2b-512) while
//!   it streams from disk, so a 200 MB installer is never held in memory. Legacy
//!   (whole-file) signatures are refused.
//! - **What the signature binds.** minisign's global signature covers the trusted
//!   comment, which the signer fills with `file:<installer name>` and
//!   `version:<version>`. The installer's name says which product and version it is
//!   (`HealthierYou_0.2.0_x64-setup.exe`, `HealthierYou-Server_0.2.0_x64-setup.exe`). A
//!   file is accepted only if its name, the signed name and the signed version all agree,
//!   so a renamed or re-labelled release is refused.
//! - **What may be installed** ([`plan`]): a newer release of the same product, or, as a
//!   rollback, exactly the release this PC was updated from (one release back, which the
//!   expand-then-contract migration rule keeps able to open the data). Anything else is
//!   refused.
//! - **The update record** ([`UpdateState`]) survives the restart: the attempt in
//!   progress, the version this PC came from, and the last outcome for Settings.
//! - **Keeping the previous version.** Before an installer runs, the program files are
//!   copied aside ([`backup_program_files`]); if the new version does not start, they are
//!   put back ([`restore_program_files`]).
//! - **Data rollback.** An older release that finds a newer database it cannot read
//!   offers the copy taken before the upgrade ([`usable_pre_migration_copy`],
//!   [`restore_pre_migration_copy`]); the newer database is set aside, never deleted.

use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use data_encoding::BASE64;
use minisign_verify::{PublicKey, Signature};
use serde::{Deserialize, Serialize};

use crate::migrations::{self, Migration};

/// The companion installer's name: `HealthierYou_<version>_x64-setup.exe`.
pub const COMPANION_PREFIX: &str = "HealthierYou_";
/// The server installer's name: `HealthierYou-Server_<version>_x64-setup.exe`.
pub const SERVER_PREFIX: &str = "HealthierYou-Server_";
/// What follows the version in both installer names.
pub const INSTALLER_SUFFIX: &str = "_x64-setup.exe";
/// The signature file's extension, appended to the installer's name.
pub const SIG_EXTENSION: &str = "sig";
/// Installers larger than this are refused before they are hashed.
pub const MAX_INSTALLER_BYTES: u64 = 1024 * 1024 * 1024;
/// Signature files larger than this are refused.
pub const MAX_SIG_BYTES: u64 = 16 * 1024;

/// A release version, `major.minor.patch` (no pre-release or build parts).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    /// Major.
    pub major: u32,
    /// Minor.
    pub minor: u32,
    /// Patch.
    pub patch: u32,
}

impl Version {
    /// A version from its parts.
    #[must_use]
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl FromStr for Version {
    type Err = UpdateError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bad = || UpdateError::BadName(s.to_owned());
        let mut parts = s.split('.');
        let mut next = || -> Result<u32, UpdateError> {
            let p = parts.next().ok_or_else(bad)?;
            // Digits only, no sign and no leading zero (except "0"), so each version has
            // one spelling.
            if p.is_empty()
                || p.len() > 9
                || !p.bytes().all(|b| b.is_ascii_digit())
                || (p.len() > 1 && p.starts_with('0'))
            {
                return Err(bad());
            }
            p.parse().map_err(|_| bad())
        };
        let v = Self::new(next()?, next()?, next()?);
        if parts.next().is_some() {
            return Err(bad());
        }
        Ok(v)
    }
}

impl Serialize for Version {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Version {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// Which program an installer is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Product {
    /// The per-user desktop companion.
    Companion,
    /// The office server.
    Server,
}

impl Product {
    /// The installer's file name for `version`.
    #[must_use]
    pub fn installer_name(self, version: Version) -> String {
        let prefix = match self {
            Self::Companion => COMPANION_PREFIX,
            Self::Server => SERVER_PREFIX,
        };
        format!("{prefix}{version}{INSTALLER_SUFFIX}")
    }
}

/// The product and version an installer's file name declares, if it is one of ours.
#[must_use]
pub fn parse_installer_name(name: &str) -> Option<(Product, Version)> {
    let rest = name.strip_suffix(INSTALLER_SUFFIX)?;
    // The server prefix is checked first: "HealthierYou_" is not a prefix of it, but keep
    // the order explicit.
    let (product, version) = match rest.strip_prefix(SERVER_PREFIX) {
        Some(v) => (Product::Server, v),
        None => (Product::Companion, rest.strip_prefix(COMPANION_PREFIX)?),
    };
    Some((product, version.parse().ok()?))
}

/// Why an update file was refused. Every message is worded for the person choosing the
/// file; none contains a path.
#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    /// This build has no update key (a development build).
    #[error("This build of HealthierYou has no update key, so it cannot install update files.")]
    NoKey,
    /// A built-in key could not be read (a build defect).
    #[error("The built-in update key is damaged: {0}")]
    BadKey(String),
    /// Not a HealthierYou installer name, or a malformed version.
    #[error(
        "\"{0}\" is not a HealthierYou installer. Choose the file named like HealthierYou_1.2.3_x64-setup.exe, unchanged."
    )]
    BadName(String),
    /// The `.sig` file is missing, too large or not a signature.
    #[error("The signature file is missing or damaged: {0}")]
    BadSignatureFile(String),
    /// A legacy (not pre-hashed) signature.
    #[error("The signature uses an old format that HealthierYou does not accept.")]
    LegacySignature,
    /// Signed with a key this build does not trust.
    #[error("The file is signed with a key HealthierYou does not trust.")]
    UnknownKey,
    /// The signature does not match the file: it was changed after signing.
    #[error("The signature does not match the file. It may be damaged or altered.")]
    Tampered,
    /// The signed file name differs from the file's name.
    #[error("The file was renamed after it was signed (signed as \"{signed}\").")]
    NameMismatch {
        /// The name in the signature.
        signed: String,
    },
    /// The signature carries no version.
    #[error("The signature does not say which version it is for, so it is refused.")]
    MissingVersion,
    /// The signed version differs from the name's.
    #[error("The signature is for version {signed}, but the file name says {named}.")]
    VersionMismatch {
        /// The version in the signature.
        signed: String,
        /// The version in the name.
        named: Version,
    },
    /// The installer is larger than any release could be.
    #[error("The file is too large to be a HealthierYou installer.")]
    TooLarge,
    /// An installer for the other product.
    #[error("This installer is for the {0}, not this program.")]
    WrongProduct(&'static str),
    /// The same version is already installed.
    #[error("Version {0} is already installed.")]
    AlreadyInstalled(Version),
    /// An older version that is not the one this PC came from.
    #[error(
        "Version {offered} is older than the installed {current}. Only the version this PC was updated from{previous} can be put back."
    )]
    NotRollbackCompatible {
        /// The offered version.
        offered: Version,
        /// The installed version.
        current: Version,
        /// ", version x.y.z", or empty when there is none.
        previous: String,
    },
    /// The file could not be read.
    #[error("The file could not be read: {0}")]
    Io(String),
}

/// The update public keys built into a program.
#[derive(Debug, Clone, Default)]
pub struct TrustedKeys {
    keys: Vec<PublicKey>,
}

impl TrustedKeys {
    /// Read the keys built into the program: zero or more public keys separated by
    /// whitespace or `;`. Each is either the `.pub` file the Tauri CLI writes (the
    /// base64 of a minisign public-key file) or the bare minisign key line (`RW...`).
    ///
    /// # Errors
    /// [`UpdateError::BadKey`] if any key cannot be decoded.
    pub fn parse(text: &str) -> Result<Self, UpdateError> {
        let mut keys = Vec::new();
        for item in text
            .split(|c: char| c == ';' || c.is_whitespace())
            .filter(|s| !s.is_empty())
        {
            keys.push(decode_public_key(item)?);
        }
        Ok(Self { keys })
    }

    /// Whether there is no key (a development build).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// How many keys there are.
    #[must_use]
    pub fn len(&self) -> usize {
        self.keys.len()
    }
}

fn decode_public_key(item: &str) -> Result<PublicKey, UpdateError> {
    if item.starts_with("RW") {
        return PublicKey::from_base64(item).map_err(|e| UpdateError::BadKey(e.to_string()));
    }
    let text = BASE64
        .decode(item.as_bytes())
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .ok_or_else(|| UpdateError::BadKey("not base64 text".into()))?;
    PublicKey::decode(&text).map_err(|e| UpdateError::BadKey(e.to_string()))
}

/// Read a signature: the base64-wrapped form the Tauri CLI writes, or a plain minisign
/// signature file.
///
/// # Errors
/// [`UpdateError::BadSignatureFile`] or [`UpdateError::LegacySignature`].
pub fn decode_signature(text: &str) -> Result<Signature, UpdateError> {
    let text = text.trim();
    let plain = if text.starts_with("untrusted comment:") {
        text.to_owned()
    } else {
        BASE64
            .decode(text.as_bytes())
            .ok()
            .and_then(|b| String::from_utf8(b).ok())
            .ok_or_else(|| UpdateError::BadSignatureFile("not a signature".into()))?
    };
    // The second line's first two bytes name the algorithm: "Ed" is legacy (whole file),
    // "ED" pre-hashed. Refuse legacy up front, with its own message.
    let alg = plain
        .lines()
        .nth(1)
        .and_then(|l| BASE64.decode(l.trim().as_bytes()).ok())
        .and_then(|b| b.get(..2).map(<[u8]>::to_vec));
    if alg.as_deref() == Some(b"Ed") {
        return Err(UpdateError::LegacySignature);
    }
    Signature::decode(&plain).map_err(|e| UpdateError::BadSignatureFile(e.to_string()))
}

/// The `key:value` fields of a trusted comment (tab separated).
fn comment_field<'a>(comment: &'a str, key: &str) -> Option<&'a str> {
    comment
        .split('\t')
        .find_map(|f| f.strip_prefix(key)?.strip_prefix(':'))
}

/// A release whose signature checked out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VerifiedRelease {
    /// Which program it installs.
    pub product: Product,
    /// Its version.
    pub version: Version,
    /// The installer's file name (as signed).
    pub file_name: String,
    /// The installer's size in bytes.
    pub size: u64,
}

/// The signature file's path for an installer: `<installer>.sig`.
#[must_use]
pub fn sig_path_for(installer: &Path) -> PathBuf {
    let mut name = installer.as_os_str().to_owned();
    name.push(".");
    name.push(SIG_EXTENSION);
    PathBuf::from(name)
}

/// Read a signature file, refusing one too large to be a signature.
///
/// # Errors
/// [`UpdateError::BadSignatureFile`].
pub fn read_signature_file(path: &Path) -> Result<String, UpdateError> {
    let meta = std::fs::metadata(path).map_err(|_| {
        UpdateError::BadSignatureFile(
            "keep the .sig file next to the installer, with the same name plus .sig".into(),
        )
    })?;
    if meta.len() > MAX_SIG_BYTES {
        return Err(UpdateError::BadSignatureFile("too large".into()));
    }
    std::fs::read_to_string(path).map_err(|e| UpdateError::BadSignatureFile(e.to_string()))
}

/// Check an installer against its signature and the built-in keys.
///
/// The order matters: the name is checked first (so an unrelated file is refused without
/// being read), then the signature's key and trusted comment, then the whole file is
/// hashed and the signature verified.
///
/// # Errors
/// Any [`UpdateError`] up to [`UpdateError::TooLarge`]; nothing is changed.
pub fn verify_release(
    keys: &TrustedKeys,
    installer: &Path,
    sig_text: &str,
) -> Result<VerifiedRelease, UpdateError> {
    if keys.is_empty() {
        return Err(UpdateError::NoKey);
    }
    let file_name = installer
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_owned();
    let (product, version) =
        parse_installer_name(&file_name).ok_or_else(|| UpdateError::BadName(file_name.clone()))?;
    let sig = decode_signature(sig_text)?;
    let comment = sig.trusted_comment();
    match comment_field(comment, "file") {
        Some(signed) if signed == file_name => {}
        Some(signed) => {
            return Err(UpdateError::NameMismatch {
                signed: signed.to_owned(),
            });
        }
        None => {
            return Err(UpdateError::NameMismatch {
                signed: String::new(),
            });
        }
    }
    match comment_field(comment, "version") {
        None => return Err(UpdateError::MissingVersion),
        Some(signed) if signed.parse::<Version>().ok() == Some(version) => {}
        Some(signed) => {
            return Err(UpdateError::VersionMismatch {
                signed: signed.to_owned(),
                named: version,
            });
        }
    }
    // Find the key the signature names (minisign carries an 8-byte key id).
    let mut verifier = None;
    for key in &keys.keys {
        match key.verify_stream(&sig) {
            Ok(v) => {
                verifier = Some(v);
                break;
            }
            Err(minisign_verify::Error::UnexpectedKeyId) => {}
            Err(minisign_verify::Error::UnsupportedLegacyMode) => {
                return Err(UpdateError::LegacySignature);
            }
            Err(e) => return Err(UpdateError::BadSignatureFile(e.to_string())),
        }
    }
    let mut verifier = verifier.ok_or(UpdateError::UnknownKey)?;
    let mut file = std::fs::File::open(installer).map_err(|e| UpdateError::Io(e.to_string()))?;
    let size = file
        .metadata()
        .map_err(|e| UpdateError::Io(e.to_string()))?
        .len();
    if size > MAX_INSTALLER_BYTES {
        return Err(UpdateError::TooLarge);
    }
    let mut buf = vec![0u8; 1024 * 1024];
    let mut read: u64 = 0;
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| UpdateError::Io(e.to_string()))?;
        if n == 0 {
            break;
        }
        read += n as u64;
        if read > MAX_INSTALLER_BYTES {
            return Err(UpdateError::TooLarge);
        }
        verifier.update(&buf[..n]);
    }
    verifier.finalize().map_err(|_| UpdateError::Tampered)?;
    Ok(VerifiedRelease {
        product,
        version,
        file_name,
        size: read,
    })
}

/// What installing a verified release would do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Plan {
    /// A newer release.
    Upgrade,
    /// The release this PC was updated from (one release back).
    Rollback,
}

/// Decide whether `release` may be installed over `current`.
///
/// `previous` is the version this PC was last updated from ([`UpdateState`]); only it may
/// be installed as a rollback.
///
/// # Errors
/// [`UpdateError::WrongProduct`], [`UpdateError::AlreadyInstalled`] or
/// [`UpdateError::NotRollbackCompatible`].
pub fn plan(
    product: Product,
    current: Version,
    previous: Option<Version>,
    release: &VerifiedRelease,
) -> Result<Plan, UpdateError> {
    if release.product != product {
        return Err(UpdateError::WrongProduct(match release.product {
            Product::Companion => "desktop companion",
            Product::Server => "office server",
        }));
    }
    let offered = release.version;
    if offered > current {
        Ok(Plan::Upgrade)
    } else if offered == current {
        Err(UpdateError::AlreadyInstalled(offered))
    } else if previous == Some(offered) {
        Ok(Plan::Rollback)
    } else {
        Err(UpdateError::NotRollbackCompatible {
            offered,
            current,
            previous: previous.map_or_else(String::new, |p| format!(", version {p}")),
        })
    }
}

// ------------------------------------------------------------------ update record

/// The update record's file name in the updates folder.
pub const STATE_FILE: &str = "state.json";
/// The new version's start confirmation, in the updates folder.
pub const CONFIRM_FILE: &str = "confirm.json";

/// Where an attempt is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// The installer was checked and copied into the updates folder.
    Staged,
    /// The program files were copied aside and the installer is running.
    Installing,
    /// The installer finished; waiting for the new version to confirm it started.
    AwaitingStart,
    /// The new version did not start; the previous files are being put back.
    RollingBack,
}

/// An update in progress.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attempt {
    /// The version installed before.
    pub from: Version,
    /// The version being installed.
    pub to: Version,
    /// Upgrade or rollback.
    pub plan: Plan,
    /// The staged installer's file name (in the updates folder).
    pub installer: String,
    /// When it started (Unix ms).
    pub started_at_ms: i64,
    /// Where it is.
    pub phase: Phase,
    /// A random value the new version echoes in its confirmation.
    pub token: String,
    /// Whether the chosen file carried Windows' downloaded-file mark (Mark of the Web).
    /// It is kept on the staged copy, so Windows checks the installer as it would have.
    pub marked_from_internet: bool,
}

/// How an attempt ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeKind {
    /// The new version installed and confirmed it started.
    Updated,
    /// The new version did not start; the previous version was put back.
    RolledBack,
    /// Windows or the user stopped the installer before it changed anything.
    Blocked,
    /// Something failed and could not be put right automatically.
    Failed,
}

/// The last attempt's outcome, shown in Settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outcome {
    /// What happened.
    pub kind: OutcomeKind,
    /// The version before.
    pub from: Version,
    /// The version offered.
    pub to: Version,
    /// When it ended (Unix ms).
    pub at_ms: i64,
    /// A short, path-free explanation.
    pub detail: String,
}

/// The update record (`updates\state.json`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateState {
    /// The version this PC was last updated from: the only one that may be put back.
    #[serde(default)]
    pub previous_version: Option<Version>,
    /// The attempt in progress.
    #[serde(default)]
    pub attempt: Option<Attempt>,
    /// The last outcome.
    #[serde(default)]
    pub last: Option<Outcome>,
}

impl UpdateState {
    /// Read the record; a missing file is an empty record.
    ///
    /// # Errors
    /// An unreadable or malformed file.
    pub fn load(dir: &Path) -> std::io::Result<Self> {
        match std::fs::read(dir.join(STATE_FILE)) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(std::io::Error::other),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e),
        }
    }

    /// Write the record atomically (a temporary file, then a rename).
    ///
    /// # Errors
    /// A write failure; the previous record is unchanged.
    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        let bytes = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        write_atomic(&dir.join(STATE_FILE), &bytes)
    }

    /// End the attempt with `kind`, recording it as the last outcome. A completed
    /// upgrade or rollback updates [`UpdateState::previous_version`]: after an upgrade
    /// the old version may be put back; after a rollback nothing older may.
    pub fn finish(&mut self, kind: OutcomeKind, detail: impl Into<String>, at_ms: i64) {
        let Some(a) = self.attempt.take() else {
            return;
        };
        if kind == OutcomeKind::Updated {
            self.previous_version = match a.plan {
                Plan::Upgrade => Some(a.from),
                Plan::Rollback => None,
            };
        }
        self.last = Some(Outcome {
            kind,
            from: a.from,
            to: a.to,
            at_ms,
            detail: detail.into(),
        });
    }
}

/// The new version's confirmation that it started (`updates\confirm.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Confirmation {
    /// The version that started.
    pub version: Version,
    /// The attempt's token.
    pub token: String,
}

impl Confirmation {
    /// Write the confirmation atomically.
    ///
    /// # Errors
    /// A write failure.
    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        let bytes = serde_json::to_vec(self).map_err(std::io::Error::other)?;
        write_atomic(&dir.join(CONFIRM_FILE), &bytes)
    }

    /// Read the confirmation, if there is a readable one.
    #[must_use]
    pub fn load(dir: &Path) -> Option<Self> {
        let bytes = std::fs::read(dir.join(CONFIRM_FILE)).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// Whether this confirms `attempt`: the expected version, with its token.
    #[must_use]
    pub fn confirms(&self, attempt: &Attempt) -> bool {
        self.version == attempt.to && self.token == attempt.token
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

// ----------------------------------------------------------- previous program files

/// Copy the named program entries (files or folders) of `install_dir` into `keep_dir`,
/// replacing what was there. Entries that do not exist are skipped. Returns the entries
/// copied.
///
/// # Errors
/// A copy failure; `keep_dir` may then be incomplete and must not be restored from.
pub fn backup_program_files(
    install_dir: &Path,
    keep_dir: &Path,
    entries: &[&str],
) -> std::io::Result<Vec<String>> {
    if keep_dir.exists() {
        std::fs::remove_dir_all(keep_dir)?;
    }
    std::fs::create_dir_all(keep_dir)?;
    let mut copied = Vec::new();
    for e in entries {
        let src = install_dir.join(e);
        if !src.exists() {
            continue;
        }
        copy_entry(&src, &keep_dir.join(e))?;
        copied.push((*e).to_owned());
    }
    // A marker written last: a copy interrupted half-way has none and is never restored.
    std::fs::write(keep_dir.join(KEEP_COMPLETE), copied.join("\n"))?;
    Ok(copied)
}

/// The marker [`backup_program_files`] writes when its copy is complete.
pub const KEEP_COMPLETE: &str = "complete.txt";

/// Put the program entries kept by [`backup_program_files`] back into `install_dir`:
/// every entry in `entries` is removed from `install_dir` first (so files a newer version
/// added go too), then the kept copies are copied back.
///
/// # Errors
/// No complete kept copy, or a file-system failure.
pub fn restore_program_files(
    keep_dir: &Path,
    install_dir: &Path,
    entries: &[&str],
) -> std::io::Result<Vec<String>> {
    let list = std::fs::read_to_string(keep_dir.join(KEEP_COMPLETE)).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no complete copy of the previous version",
        )
    })?;
    let kept: Vec<&str> = list.lines().filter(|l| !l.is_empty()).collect();
    for e in entries {
        let dst = install_dir.join(e);
        if dst.is_dir() {
            std::fs::remove_dir_all(&dst)?;
        } else if dst.exists() {
            std::fs::remove_file(&dst)?;
        }
    }
    let mut restored = Vec::new();
    for e in kept {
        if !entries.contains(&e) {
            continue;
        }
        copy_entry(&keep_dir.join(e), &install_dir.join(e))?;
        restored.push(e.to_owned());
    }
    Ok(restored)
}

fn copy_entry(src: &Path, dst: &Path) -> std::io::Result<()> {
    if src.is_dir() {
        std::fs::create_dir_all(dst)?;
        for item in std::fs::read_dir(src)? {
            let item = item?;
            copy_entry(&item.path(), &dst.join(item.file_name()))?;
        }
        Ok(())
    } else {
        if let Some(dir) = dst.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::copy(src, dst).map(|_| ())
    }
}

// ------------------------------------------------------------------- data rollback

/// The pre-migration copies next to `db` (`<db>.pre-v<N>.bak`), highest version first.
#[must_use]
pub fn pre_migration_copies(db: &Path) -> Vec<(u32, PathBuf)> {
    let (Some(dir), Some(name)) = (db.parent(), db.file_name().and_then(|n| n.to_str())) else {
        return Vec::new();
    };
    let prefix = format!("{name}.pre-v");
    let mut found: Vec<(u32, PathBuf)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().into_string().ok()?;
            let v = n
                .strip_prefix(&prefix)?
                .strip_suffix(".bak")?
                .parse()
                .ok()?;
            Some((v, e.path()))
        })
        .collect();
    found.sort_by_key(|f| std::cmp::Reverse(f.0));
    found
}

/// The newest pre-migration copy that a build with `migrations` can open, if any.
#[must_use]
pub fn usable_pre_migration_copy(db: &Path, migrations: &[Migration]) -> Option<(u32, PathBuf)> {
    let supported = migrations.last().map_or(0, |m| m.version);
    pre_migration_copies(db).into_iter().find(|(_, path)| {
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .ok()
            .and_then(|c| migrations::schema_status(&c).ok())
            .is_some_and(|s| s.version >= 1 && s.required_reader <= supported)
    })
}

/// Where the data stands after a failed [`restore_pre_migration_copy`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestoreState {
    /// The database is in place as it was (perhaps folded into one file); the copy was
    /// not put back.
    Unchanged,
    /// The database is no longer at its name: it was set aside at this path and could not
    /// be moved back. A marker beside it ([`incomplete_restore`]) stops the next start
    /// from creating an empty database in its place.
    SetAside(PathBuf),
}

/// A pre-migration restore that did not happen, and where the data stands.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{reason}")]
pub struct RestoreError {
    /// What failed, without paths.
    pub reason: String,
    /// Where the data stands.
    pub state: RestoreState,
}

impl RestoreError {
    fn unchanged(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
            state: RestoreState::Unchanged,
        }
    }
}

/// The file operations a restore uses (failures are injected in tests).
#[doc(hidden)]
pub trait RestoreFs {
    /// Copy `from` to `to` and flush it to disk.
    ///
    /// # Errors
    /// An I/O error.
    fn copy(&self, from: &Path, to: &Path) -> std::io::Result<()>;
    /// Rename `from` to `to` (same folder).
    ///
    /// # Errors
    /// An I/O error.
    fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()>;
}

/// The real file system.
#[doc(hidden)]
#[derive(Debug, Clone, Copy)]
pub struct RealFs;

impl RestoreFs for RealFs {
    fn copy(&self, from: &Path, to: &Path) -> std::io::Result<()> {
        std::fs::copy(from, to)?;
        std::fs::OpenOptions::new().write(true).open(to)?.sync_all()
    }
    fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()> {
        std::fs::rename(from, to)
    }
}

fn side_path(db: &Path, suffix: &str) -> PathBuf {
    PathBuf::from(format!("{}{suffix}", db.display()))
}

/// The marker left when a restore set the database aside and could not move it back.
fn marker_path(db: &Path) -> PathBuf {
    side_path(db, ".restore-incomplete")
}

/// If a restore set the database aside and could not move it back, where it is. The
/// program must not open (and so create) a database at `db` while this is `Some`.
#[must_use]
pub fn incomplete_restore(db: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(marker_path(db)).ok()?;
    let name = text.trim();
    // Only a file name in the same folder is accepted.
    if name.is_empty() || name.contains(['/', '\\']) {
        return None;
    }
    db.parent().map(|d| d.join(name))
}

/// Move a database a failed restore set aside back to its name and clear the marker.
///
/// # Errors
/// The set-aside file is missing, a database is already at `db`, or the rename failed.
pub fn recover_incomplete_restore(db: &Path) -> std::io::Result<()> {
    let aside = incomplete_restore(db)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no restore marker"))?;
    if db.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "a database is already in place",
        ));
    }
    std::fs::rename(&aside, db)?;
    std::fs::remove_file(marker_path(db))
}

/// Check that `path` is a whole SQLite database with a HealthierYou schema.
fn validate_database(path: &Path) -> Result<u32, String> {
    let conn =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| format!("the copy could not be opened: {e}"))?;
    let check: String = conn
        .query_row("PRAGMA quick_check", [], |r| r.get(0))
        .map_err(|e| format!("the copy could not be checked: {e}"))?;
    if check != "ok" {
        return Err("the copy is damaged".into());
    }
    let status = migrations::schema_status(&conn)
        .map_err(|e| format!("the copy has no readable schema: {e}"))?;
    if status.version < 1 {
        return Err("the copy has no HealthierYou schema".into());
    }
    Ok(status.version)
}

/// Put a pre-migration copy back as the database, failure-safe:
///
/// 1. the copy is copied beside the database (`<db>.restore-<millis>.tmp`), flushed and
///    checked (whole, readable schema) before anything else changes;
/// 2. the newer database is folded into one file (its write-ahead log checkpointed);
/// 3. it is renamed to `<db>.newer-<millis>.bak` (never deleted);
/// 4. the checked copy is renamed into place.
///
/// If step 4 fails, the newer database is moved back. Only if that fails too is the
/// database left aside, with a marker so the next start does not create an empty one
/// ([`incomplete_restore`]). The pre-migration copy itself is never changed. Returns
/// where the newer database was set aside.
///
/// # Errors
/// [`RestoreError`], which says where the data stands.
pub fn restore_pre_migration_copy(
    db: &Path,
    copy: &Path,
    now_ms: i64,
) -> Result<PathBuf, RestoreError> {
    restore_pre_migration_copy_with(&RealFs, db, copy, now_ms)
}

/// As [`restore_pre_migration_copy`] with the file operations supplied (tests).
///
/// # Errors
/// [`RestoreError`].
#[doc(hidden)]
pub fn restore_pre_migration_copy_with(
    fs: &dyn RestoreFs,
    db: &Path,
    copy: &Path,
    now_ms: i64,
) -> Result<PathBuf, RestoreError> {
    // 1. Stage and check the replacement; nothing has changed yet.
    let staged = side_path(db, &format!(".restore-{now_ms}.tmp"));
    let _ = std::fs::remove_file(&staged);
    if let Err(e) = fs.copy(copy, &staged) {
        let _ = std::fs::remove_file(&staged);
        return Err(RestoreError::unchanged(format!(
            "the earlier copy could not be copied ({e})"
        )));
    }
    if let Err(why) = validate_database(&staged) {
        let _ = std::fs::remove_file(&staged);
        return Err(RestoreError::unchanged(why));
    }
    // 2. Fold the newer database into one file. Any schema can be checkpointed.
    let folded = (|| -> rusqlite::Result<()> {
        let conn =
            rusqlite::Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        let _: (i64, i64, i64) = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?;
        conn.pragma_update(None, "journal_mode", "DELETE")?;
        Ok(())
    })();
    let wal_left = ["-wal", "-shm"].iter().any(|ext| {
        side_path(db, ext).exists()
            && std::fs::metadata(side_path(db, ext)).is_ok_and(|m| m.len() > 0)
    });
    if let Err(e) = folded {
        let _ = std::fs::remove_file(&staged);
        return Err(RestoreError::unchanged(format!(
            "the current data could not be prepared ({e})"
        )));
    }
    if wal_left {
        let _ = std::fs::remove_file(&staged);
        return Err(RestoreError::unchanged(
            "the current data is still in use by another program",
        ));
    }
    // 3. Set the newer database aside.
    let aside = side_path(db, &format!(".newer-{now_ms}.bak"));
    if let Err(e) = fs.rename(db, &aside) {
        let _ = std::fs::remove_file(&staged);
        return Err(RestoreError::unchanged(format!(
            "the current data could not be set aside ({e})"
        )));
    }
    for ext in ["-wal", "-shm"] {
        let _ = std::fs::remove_file(side_path(db, ext));
    }
    // 4. Swap the checked copy in.
    match fs.rename(&staged, db) {
        Ok(()) => Ok(aside),
        Err(e) => {
            let _ = std::fs::remove_file(&staged);
            match fs.rename(&aside, db) {
                Ok(()) => Err(RestoreError::unchanged(format!(
                    "the earlier copy could not be put in place ({e}); the current data was put back"
                ))),
                Err(back) => {
                    let name = aside
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let marked = std::fs::write(marker_path(db), &name).is_ok();
                    Err(RestoreError {
                        reason: format!(
                            "the earlier copy could not be put in place ({e}) and the current data could not be moved back ({back}){}",
                            if marked {
                                ""
                            } else {
                                "; the safety marker could not be written either"
                            }
                        ),
                        state: RestoreState::SetAside(aside),
                    })
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/update");

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(Path::new(FIXTURES).join(name)).unwrap()
    }

    fn keys() -> TrustedKeys {
        TrustedKeys::parse(&fixture("test.pub")).unwrap()
    }

    /// Put a fixture installer into `dir` under `as_name` and return its path and
    /// signature text.
    fn staged(dir: &Path, fixture_name: &str, as_name: &str) -> (PathBuf, String) {
        let path = dir.join(as_name);
        std::fs::copy(
            Path::new(FIXTURES).join(format!("{fixture_name}.payload")),
            &path,
        )
        .unwrap();
        (path, fixture(&format!("{fixture_name}.sig")))
    }

    const V011: &str = "HealthierYou_0.1.1_x64-setup.exe";

    #[test]
    fn versions_parse_strictly_and_order() {
        assert_eq!("0.1.10".parse::<Version>().unwrap(), Version::new(0, 1, 10));
        for bad in [
            "",
            "1",
            "1.2",
            "1.2.3.4",
            "01.2.3",
            "1.2.-3",
            "1.2.3-rc1",
            "a.b.c",
            "1..3",
        ] {
            assert!(bad.parse::<Version>().is_err(), "{bad}");
        }
        assert!(Version::new(0, 1, 10) > Version::new(0, 1, 9));
        assert!(Version::new(1, 0, 0) > Version::new(0, 99, 99));
        let json = serde_json::to_string(&Version::new(1, 2, 3)).unwrap();
        assert_eq!(json, "\"1.2.3\"");
        assert_eq!(
            serde_json::from_str::<Version>(&json).unwrap(),
            Version::new(1, 2, 3)
        );
    }

    #[test]
    fn installer_names_say_product_and_version() {
        assert_eq!(
            parse_installer_name(V011),
            Some((Product::Companion, Version::new(0, 1, 1)))
        );
        assert_eq!(
            parse_installer_name("HealthierYou-Server_2.0.0_x64-setup.exe"),
            Some((Product::Server, Version::new(2, 0, 0)))
        );
        for bad in [
            "HealthierYou_0.1.1_x64-setup (1).exe",
            "HealthierYou_0.1_x64-setup.exe",
            "healthieryou_0.1.1_x64-setup.exe",
            "HealthierYou_0.1.1_x86-setup.exe",
            "Other_0.1.1_x64-setup.exe",
        ] {
            assert_eq!(parse_installer_name(bad), None, "{bad}");
        }
        assert_eq!(
            Product::Server.installer_name(Version::new(0, 2, 0)),
            "HealthierYou-Server_0.2.0_x64-setup.exe"
        );
    }

    #[test]
    fn keys_parse_in_both_forms() {
        assert!(TrustedKeys::parse("").unwrap().is_empty());
        let two = format!("{};{}", fixture("test.pub"), fixture("other.pub"));
        assert_eq!(TrustedKeys::parse(&two).unwrap().len(), 2);
        // The bare minisign line is the second line of the decoded .pub file.
        let decoded = String::from_utf8(
            BASE64
                .decode(fixture("test.pub").trim().as_bytes())
                .unwrap(),
        )
        .unwrap();
        let bare = decoded.lines().nth(1).unwrap();
        assert_eq!(TrustedKeys::parse(bare).unwrap().len(), 1);
        assert!(matches!(
            TrustedKeys::parse("not-a-key"),
            Err(UpdateError::BadKey(_))
        ));
    }

    #[test]
    fn a_signed_release_verifies() {
        let dir = tempfile::tempdir().unwrap();
        let (path, sig) = staged(dir.path(), V011, V011);
        let r = verify_release(&keys(), &path, &sig).unwrap();
        assert_eq!(r.product, Product::Companion);
        assert_eq!(r.version, Version::new(0, 1, 1));
        assert_eq!(r.file_name, V011);
        assert_eq!(r.size, std::fs::metadata(&path).unwrap().len());
        // The plain minisign form of the same signature verifies too.
        let plain = String::from_utf8(BASE64.decode(sig.trim().as_bytes()).unwrap()).unwrap();
        verify_release(&keys(), &path, &plain).unwrap();
        // With a second, unrelated key also built in.
        let both = TrustedKeys::parse(&format!("{} {}", fixture("other.pub"), fixture("test.pub")))
            .unwrap();
        verify_release(&both, &path, &sig).unwrap();
    }

    #[test]
    fn a_build_without_a_key_refuses_every_file() {
        let dir = tempfile::tempdir().unwrap();
        let (path, sig) = staged(dir.path(), V011, V011);
        assert!(matches!(
            verify_release(&TrustedKeys::default(), &path, &sig),
            Err(UpdateError::NoKey)
        ));
    }

    #[test]
    fn a_changed_byte_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (path, sig) = staged(dir.path(), V011, V011);
        let mut bytes = std::fs::read(&path).unwrap();
        bytes[3] ^= 0x01;
        std::fs::write(&path, &bytes).unwrap();
        assert!(matches!(
            verify_release(&keys(), &path, &sig),
            Err(UpdateError::Tampered)
        ));
        // An appended byte too.
        bytes[3] ^= 0x01;
        bytes.push(b'x');
        std::fs::write(&path, &bytes).unwrap();
        assert!(matches!(
            verify_release(&keys(), &path, &sig),
            Err(UpdateError::Tampered)
        ));
    }

    #[test]
    fn another_key_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (path, sig) = staged(dir.path(), V011, V011);
        let other = TrustedKeys::parse(&fixture("other.pub")).unwrap();
        assert!(matches!(
            verify_release(&other, &path, &sig),
            Err(UpdateError::UnknownKey)
        ));
    }

    #[test]
    fn a_renamed_file_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        // The 0.1.1 release renamed to claim 0.1.3.
        let (path, sig) = staged(dir.path(), V011, "HealthierYou_0.1.3_x64-setup.exe");
        assert!(matches!(
            verify_release(&keys(), &path, &sig),
            Err(UpdateError::NameMismatch { signed }) if signed == V011
        ));
        // A name that is not an installer at all is refused before it is read.
        let (path, sig) = staged(dir.path(), V011, "setup.exe");
        assert!(matches!(
            verify_release(&keys(), &path, &sig),
            Err(UpdateError::BadName(_))
        ));
        // The server's signature on a companion name.
        let (path, sig) = staged(dir.path(), "HealthierYou-Server_0.1.1_x64-setup.exe", V011);
        assert!(matches!(
            verify_release(&keys(), &path, &sig),
            Err(UpdateError::NameMismatch { .. })
        ));
    }

    #[test]
    fn the_signed_version_must_match_the_name() {
        let dir = tempfile::tempdir().unwrap();
        let n = "HealthierYou_0.1.2_x64-setup.exe";
        let (path, sig) = staged(dir.path(), n, n);
        assert!(matches!(
            verify_release(&keys(), &path, &sig),
            Err(UpdateError::MissingVersion)
        ));
        let n = "HealthierYou_0.1.4_x64-setup.exe";
        let (path, sig) = staged(dir.path(), n, n);
        assert!(matches!(
            verify_release(&keys(), &path, &sig),
            Err(UpdateError::VersionMismatch { signed, named })
                if signed == "0.1.5" && named == Version::new(0, 1, 4)
        ));
    }

    #[test]
    fn damaged_and_legacy_signatures_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (path, sig) = staged(dir.path(), V011, V011);
        for bad in ["", "garbage", "dW50cnVzdGVkIGNvbW1lbnQ6"] {
            assert!(
                matches!(
                    verify_release(&keys(), &path, bad),
                    Err(UpdateError::BadSignatureFile(_))
                ),
                "{bad:?}"
            );
        }
        // Rewrite the algorithm bytes to legacy "Ed".
        let plain = String::from_utf8(BASE64.decode(sig.trim().as_bytes()).unwrap()).unwrap();
        let mut lines: Vec<String> = plain.lines().map(str::to_owned).collect();
        let mut bin = BASE64.decode(lines[1].as_bytes()).unwrap();
        bin[1] = b'd';
        lines[1] = BASE64.encode(&bin);
        assert!(matches!(
            verify_release(&keys(), &path, &lines.join("\n")),
            Err(UpdateError::LegacySignature)
        ));
        // A changed trusted comment breaks the global signature.
        let tampered = plain.replace("version:0.1.1", "version:0.1.1\tx:y");
        assert!(matches!(
            verify_release(&keys(), &path, &tampered),
            Err(UpdateError::Tampered)
        ));
    }

    #[test]
    fn the_signature_file_sits_next_to_the_installer() {
        let p = Path::new("dir").join(V011);
        assert_eq!(
            sig_path_for(&p),
            Path::new("dir").join(format!("{V011}.sig"))
        );
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            read_signature_file(&dir.path().join("missing.sig")),
            Err(UpdateError::BadSignatureFile(_))
        ));
        let big = dir.path().join("big.sig");
        std::fs::write(&big, vec![b'a'; 20_000]).unwrap();
        assert!(matches!(
            read_signature_file(&big),
            Err(UpdateError::BadSignatureFile(_))
        ));
    }

    fn release(product: Product, v: Version) -> VerifiedRelease {
        VerifiedRelease {
            product,
            version: v,
            file_name: product.installer_name(v),
            size: 1,
        }
    }

    #[test]
    fn only_newer_or_the_previous_version_may_be_installed() {
        let cur = Version::new(0, 2, 0);
        let prev = Some(Version::new(0, 1, 0));
        let p = |v, prev| {
            plan(
                Product::Companion,
                cur,
                prev,
                &release(Product::Companion, v),
            )
        };
        assert_eq!(p(Version::new(0, 3, 0), prev).unwrap(), Plan::Upgrade);
        assert_eq!(p(Version::new(0, 2, 1), None).unwrap(), Plan::Upgrade);
        assert_eq!(p(Version::new(0, 1, 0), prev).unwrap(), Plan::Rollback);
        assert!(matches!(
            p(Version::new(0, 2, 0), prev),
            Err(UpdateError::AlreadyInstalled(_))
        ));
        // Two releases back, or no record of where this PC came from.
        assert!(matches!(
            p(Version::new(0, 0, 9), prev),
            Err(UpdateError::NotRollbackCompatible { .. })
        ));
        let e = p(Version::new(0, 1, 0), None).unwrap_err();
        assert!(matches!(e, UpdateError::NotRollbackCompatible { .. }));
        assert!(!e.to_string().contains(", version"));
        // The other product's installer.
        assert!(matches!(
            plan(
                Product::Companion,
                cur,
                prev,
                &release(Product::Server, Version::new(9, 0, 0))
            ),
            Err(UpdateError::WrongProduct("office server"))
        ));
    }

    fn attempt(plan: Plan) -> Attempt {
        Attempt {
            from: Version::new(0, 1, 0),
            to: Version::new(0, 1, 1),
            plan,
            installer: V011.into(),
            started_at_ms: 1,
            phase: Phase::AwaitingStart,
            token: "t0k".into(),
            marked_from_internet: true,
        }
    }

    #[test]
    fn the_record_round_trips_and_tracks_the_previous_version() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            UpdateState::load(dir.path()).unwrap(),
            UpdateState::default()
        );
        let mut s = UpdateState {
            attempt: Some(attempt(Plan::Upgrade)),
            ..UpdateState::default()
        };
        s.save(dir.path()).unwrap();
        assert_eq!(UpdateState::load(dir.path()).unwrap(), s);
        // A confirmed upgrade: the old version may be put back.
        s.finish(OutcomeKind::Updated, "ok", 5);
        assert_eq!(s.previous_version, Some(Version::new(0, 1, 0)));
        assert!(s.attempt.is_none());
        assert_eq!(s.last.as_ref().unwrap().kind, OutcomeKind::Updated);
        // A rolled-back upgrade leaves the record of where this PC came from alone.
        s.attempt = Some(Attempt {
            from: Version::new(0, 1, 1),
            to: Version::new(0, 1, 2),
            ..attempt(Plan::Upgrade)
        });
        s.finish(OutcomeKind::RolledBack, "did not start", 6);
        assert_eq!(s.previous_version, Some(Version::new(0, 1, 0)));
        assert_eq!(s.last.as_ref().unwrap().to, Version::new(0, 1, 2));
        // A completed rollback: nothing older may follow it.
        s.attempt = Some(attempt(Plan::Rollback));
        s.finish(OutcomeKind::Updated, "ok", 7);
        assert_eq!(s.previous_version, None);
        // Finishing with no attempt changes nothing.
        let before = s.clone();
        s.finish(OutcomeKind::Failed, "x", 8);
        assert_eq!(s, before);
        // A damaged record is an error, not an empty one.
        std::fs::write(dir.path().join(STATE_FILE), b"{").unwrap();
        assert!(UpdateState::load(dir.path()).is_err());
    }

    #[test]
    fn a_confirmation_counts_only_for_its_attempt() {
        let dir = tempfile::tempdir().unwrap();
        let a = attempt(Plan::Upgrade);
        assert!(Confirmation::load(dir.path()).is_none());
        let c = Confirmation {
            version: a.to,
            token: a.token.clone(),
        };
        c.save(dir.path()).unwrap();
        assert!(Confirmation::load(dir.path()).unwrap().confirms(&a));
        // The old version starting (rolled back by hand), or a stale token.
        assert!(
            !Confirmation {
                version: a.from,
                ..c.clone()
            }
            .confirms(&a)
        );
        assert!(
            !Confirmation {
                token: "old".into(),
                ..c
            }
            .confirms(&a)
        );
    }

    #[test]
    fn program_files_are_kept_and_put_back() {
        let root = tempfile::tempdir().unwrap();
        let install = root.path().join("HealthierYou");
        let keep = root.path().join("keep");
        let entries = ["hy-companion.exe", "uninstall.exe", "webview2-fixed"];
        std::fs::create_dir_all(install.join("webview2-fixed/sub")).unwrap();
        std::fs::create_dir_all(install.join("data")).unwrap();
        std::fs::write(install.join("hy-companion.exe"), b"old exe").unwrap();
        std::fs::write(install.join("uninstall.exe"), b"old uninstaller").unwrap();
        std::fs::write(install.join("webview2-fixed/a.dll"), b"old runtime").unwrap();
        std::fs::write(install.join("webview2-fixed/sub/b.dat"), b"old data").unwrap();
        std::fs::write(install.join("data/hy.db"), b"personal").unwrap();
        let copied = backup_program_files(&install, &keep, &entries).unwrap();
        assert_eq!(copied, entries);
        // The personal data is never part of the program copy.
        assert!(!keep.join("data").exists());
        // The new version replaces files and adds one to the runtime.
        std::fs::write(install.join("hy-companion.exe"), b"new exe").unwrap();
        std::fs::write(install.join("webview2-fixed/a.dll"), b"new runtime").unwrap();
        std::fs::write(install.join("webview2-fixed/new.dll"), b"added").unwrap();
        std::fs::write(install.join("data/hy.db"), b"personal, newer").unwrap();
        let restored = restore_program_files(&keep, &install, &entries).unwrap();
        assert_eq!(restored, entries);
        assert_eq!(
            std::fs::read(install.join("hy-companion.exe")).unwrap(),
            b"old exe"
        );
        assert_eq!(
            std::fs::read(install.join("webview2-fixed/a.dll")).unwrap(),
            b"old runtime"
        );
        assert_eq!(
            std::fs::read(install.join("webview2-fixed/sub/b.dat")).unwrap(),
            b"old data"
        );
        assert!(!install.join("webview2-fixed/new.dll").exists());
        // The data folder is left exactly as the user last had it.
        assert_eq!(
            std::fs::read(install.join("data/hy.db")).unwrap(),
            b"personal, newer"
        );
    }

    #[test]
    fn an_incomplete_copy_is_never_restored() {
        let root = tempfile::tempdir().unwrap();
        let install = root.path().join("i");
        let keep = root.path().join("k");
        std::fs::create_dir_all(&install).unwrap();
        std::fs::create_dir_all(&keep).unwrap();
        std::fs::write(install.join("hy-companion.exe"), b"new").unwrap();
        std::fs::write(keep.join("hy-companion.exe"), b"half").unwrap();
        assert!(restore_program_files(&keep, &install, &["hy-companion.exe"]).is_err());
        assert_eq!(
            std::fs::read(install.join("hy-companion.exe")).unwrap(),
            b"new"
        );
    }

    fn db_at(path: &Path, migrations: &[Migration]) {
        let mut c = rusqlite::Connection::open(path).unwrap();
        migrations::migrate_with(
            &mut c,
            migrations,
            migrations::MigrateOptions {
                copy_before_upgrade: true,
            },
        )
        .unwrap();
    }

    #[test]
    fn the_newest_readable_pre_migration_copy_is_offered_and_restored() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("hy.db");
        let all = migrations::MIGRATIONS;
        assert!(all.len() >= 2);
        // An older build's database, then this build's upgrade (which writes the copy).
        db_at(&db, &all[..all.len() - 1]);
        db_at(&db, all);
        let copies = pre_migration_copies(&db);
        let from = u32::try_from(all.len() - 1).unwrap();
        assert_eq!(copies.len(), 1);
        assert_eq!(copies[0].0, from);
        // The older build can read the copy; a build two contracts older cannot read a
        // copy whose reader requirement is above it.
        let older = &all[..all.len() - 1];
        let (v, copy) = usable_pre_migration_copy(&db, older).unwrap();
        assert_eq!(v, from);
        assert!(usable_pre_migration_copy(&db, &all[..0]).is_none());
        // Write something only the newer database has, then restore.
        rusqlite::Connection::open(&db)
            .unwrap()
            .execute_batch("CREATE TABLE newer_only (x INTEGER)")
            .unwrap();
        let aside = restore_pre_migration_copy(&db, &copy, 42).unwrap();
        assert!(aside.ends_with("hy.db.newer-42.bak"));
        let restored = rusqlite::Connection::open(&db).unwrap();
        assert_eq!(migrations::schema_status(&restored).unwrap().version, from);
        let has_newer: bool = restored
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE name = 'newer_only')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!has_newer);
        // The newer database is kept, whole, beside it; the copy is kept too.
        let set_aside = rusqlite::Connection::open(&aside).unwrap();
        let kept: bool = set_aside
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE name = 'newer_only')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(kept);
        assert!(copy.exists());
    }

    /// A newer database (with a table only it has) and a pre-migration copy beside it.
    fn newer_db_with_copy(dir: &Path) -> (PathBuf, PathBuf) {
        let db = dir.join("hy.db");
        let all = migrations::MIGRATIONS;
        db_at(&db, &all[..all.len() - 1]);
        db_at(&db, all);
        rusqlite::Connection::open(&db)
            .unwrap()
            .execute_batch(
                "CREATE TABLE newer_only (x INTEGER); INSERT INTO newer_only VALUES (7);",
            )
            .unwrap();
        let copy = pre_migration_copies(&db).remove(0).1;
        (db, copy)
    }

    fn has_newer(db: &Path) -> bool {
        rusqlite::Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap()
            .query_row("SELECT x FROM newer_only", [], |r| r.get::<_, i64>(0))
            .is_ok_and(|x| x == 7)
    }

    fn leftovers(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".restore-") || n.contains(".newer-"))
            .collect()
    }

    type RenameRule = Box<dyn Fn(&Path, &Path) -> bool>;

    /// File operations that fail on request.
    struct Faulty {
        partial_copy: bool,
        fail_rename: RenameRule,
    }

    impl RestoreFs for Faulty {
        fn copy(&self, from: &Path, to: &Path) -> std::io::Result<()> {
            if self.partial_copy {
                // Half the file lands, then the disk is full.
                let bytes = std::fs::read(from)?;
                std::fs::write(to, &bytes[..bytes.len() / 2])?;
                return Err(std::io::Error::other("disk full"));
            }
            RealFs.copy(from, to)
        }
        fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()> {
            if (self.fail_rename)(from, to) {
                return Err(std::io::Error::other("access denied"));
            }
            RealFs.rename(from, to)
        }
    }

    fn renames_fail_when(f: impl Fn(&Path, &Path) -> bool + 'static) -> Faulty {
        Faulty {
            partial_copy: false,
            fail_rename: Box::new(f),
        }
    }

    #[test]
    fn a_restore_whose_source_cannot_be_read_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (db, _) = newer_db_with_copy(dir.path());
        let missing = dir.path().join("hy.db.pre-v1.bak.gone");
        let err = restore_pre_migration_copy(&db, &missing, 1).unwrap_err();
        assert_eq!(err.state, RestoreState::Unchanged);
        assert!(has_newer(&db));
        assert!(
            leftovers(dir.path()).is_empty(),
            "{:?}",
            leftovers(dir.path())
        );
        assert!(incomplete_restore(&db).is_none());
    }

    #[test]
    fn a_restore_whose_copy_fails_part_way_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (db, copy) = newer_db_with_copy(dir.path());
        let fs = Faulty {
            partial_copy: true,
            fail_rename: Box::new(|_, _| false),
        };
        let err = restore_pre_migration_copy_with(&fs, &db, &copy, 2).unwrap_err();
        assert_eq!(err.state, RestoreState::Unchanged);
        assert!(err.reason.contains("disk full"));
        assert!(has_newer(&db));
        assert!(leftovers(dir.path()).is_empty());
        assert!(copy.exists());
    }

    #[test]
    fn a_damaged_copy_is_refused_before_anything_changes() {
        let dir = tempfile::tempdir().unwrap();
        let (db, copy) = newer_db_with_copy(dir.path());
        let bytes = std::fs::read(&copy).unwrap();
        std::fs::write(&copy, &bytes[..bytes.len() / 3]).unwrap();
        let err = restore_pre_migration_copy(&db, &copy, 3).unwrap_err();
        assert_eq!(err.state, RestoreState::Unchanged);
        assert!(has_newer(&db));
        assert!(leftovers(dir.path()).is_empty());
    }

    #[test]
    fn a_restore_that_cannot_set_the_data_aside_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (db, copy) = newer_db_with_copy(dir.path());
        let live = db.clone();
        let fs = renames_fail_when(move |from, _| from == live);
        let err = restore_pre_migration_copy_with(&fs, &db, &copy, 4).unwrap_err();
        assert_eq!(err.state, RestoreState::Unchanged);
        assert!(has_newer(&db));
        assert!(leftovers(dir.path()).is_empty());
    }

    #[test]
    fn a_failed_swap_puts_the_current_data_back() {
        let dir = tempfile::tempdir().unwrap();
        let (db, copy) = newer_db_with_copy(dir.path());
        let fs = renames_fail_when(|from, _| from.to_string_lossy().contains(".restore-"));
        let err = restore_pre_migration_copy_with(&fs, &db, &copy, 5).unwrap_err();
        assert_eq!(err.state, RestoreState::Unchanged);
        assert!(err.reason.contains("put back"));
        assert!(has_newer(&db));
        assert!(leftovers(dir.path()).is_empty());
        assert!(incomplete_restore(&db).is_none());
    }

    #[test]
    fn a_swap_that_cannot_be_undone_leaves_a_marker_and_never_an_empty_database() {
        let dir = tempfile::tempdir().unwrap();
        let (db, copy) = newer_db_with_copy(dir.path());
        let live = db.clone();
        // Renaming anything onto the live name fails.
        let fs = renames_fail_when(move |_, to| to == live);
        let err = restore_pre_migration_copy_with(&fs, &db, &copy, 6).unwrap_err();
        let RestoreState::SetAside(aside) = err.state.clone() else {
            panic!("expected the data set aside: {err:?}");
        };
        assert!(!db.exists());
        assert!(
            has_newer(&aside),
            "the current data is whole where it was set aside"
        );
        // The next start sees the marker (and must not create a database), and can
        // move the data back.
        assert_eq!(incomplete_restore(&db), Some(aside.clone()));
        recover_incomplete_restore(&db).unwrap();
        assert!(has_newer(&db));
        assert!(incomplete_restore(&db).is_none());
        assert!(!aside.exists());
    }
}
