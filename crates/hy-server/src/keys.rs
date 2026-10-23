//! The server's keys (V1-5 contract 6.1 and 6.2).
//!
//! - **TLS key:** ECDSA P-256, with a self-signed X.509 v3 certificate (SHA-256, 20 years,
//!   random serial, SAN `hy-server.invalid` plus the computer name for diagnostics only).
//!   The full SHA-256 of the certificate's DER is the server's identity (the pin).
//! - **Head-signing key:** a separate Ed25519 key, used only for `HY-HEAD1` messages
//!   (decision C5).
//!
//! Both are generated once, at the first `init`, from the OS CSPRNG through aws-lc-rs.
//! The private keys are written only through a [`KeyProtector`]: machine-scope DPAPI on
//! Windows. They are never exported, printed, logged, backed up or written to the mirror;
//! losing them means every PC re-pairs (owner decision O4). The certificate is public and
//! stored as plain DER.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use aws_lc_rs::rand::{SecureRandom, SystemRandom};
use aws_lc_rs::signature::Ed25519KeyPair;
use chrono::Datelike;
use hy_core::protocol::head::public_key;
use hy_core::protocol::{SNI, fingerprint, key_id};
use hy_core::secret::Secret;

/// Protects private key bytes at rest.
pub trait KeyProtector: Send + Sync + std::fmt::Debug {
    /// Seal key bytes.
    ///
    /// # Errors
    /// An I/O error if the platform refuses.
    fn protect(&self, plain: &[u8]) -> io::Result<Vec<u8>>;

    /// Open sealed key bytes.
    ///
    /// # Errors
    /// An I/O error if the blob cannot be opened on this machine.
    fn unprotect(&self, blob: &[u8]) -> io::Result<Secret<Vec<u8>>>;
}

/// Machine-scope DPAPI (Windows): only this machine can open the blobs. Machine scope
/// alone lets any local user decrypt, so the installer must also restrict the keys
/// folder's ACL to the service account, Administrators and SYSTEM (contract 6.1).
#[cfg(windows)]
#[derive(Debug, Default)]
pub struct DpapiMachine;

#[cfg(windows)]
impl KeyProtector for DpapiMachine {
    fn protect(&self, plain: &[u8]) -> io::Result<Vec<u8>> {
        crate::dpapi::protect_machine(plain, KEY_ENTROPY)
    }

    fn unprotect(&self, blob: &[u8]) -> io::Result<Secret<Vec<u8>>> {
        crate::dpapi::unprotect_machine(blob, KEY_ENTROPY).map(Secret::new)
    }
}

/// DPAPI optional entropy for the server's key files.
pub const KEY_ENTROPY: &[u8] = b"HealthierYou/server/keys/v1";

/// No protection at all: the key bytes are stored as they are, in files readable by the
/// owner only. **For tests and non-Windows development only**; the server product runs on
/// Windows with [`DpapiMachine`].
#[derive(Debug, Default)]
pub struct UnprotectedForTests;

impl KeyProtector for UnprotectedForTests {
    fn protect(&self, plain: &[u8]) -> io::Result<Vec<u8>> {
        Ok(plain.to_vec())
    }

    fn unprotect(&self, blob: &[u8]) -> io::Result<Secret<Vec<u8>>> {
        Ok(Secret::new(blob.to_vec()))
    }
}

/// The platform's protector: DPAPI machine scope on Windows, otherwise the unprotected
/// test protector.
#[must_use]
pub fn platform_protector() -> Box<dyn KeyProtector> {
    #[cfg(windows)]
    {
        Box::new(DpapiMachine)
    }
    #[cfg(not(windows))]
    {
        Box::new(UnprotectedForTests)
    }
}

/// The server's loaded keys.
pub struct ServerKeys {
    /// The certificate's DER (public).
    pub cert_der: Vec<u8>,
    tls_key_pkcs8: Secret<Vec<u8>>,
    head: Ed25519KeyPair,
    /// SHA-256 of the certificate's DER: the pin.
    pub fingerprint: [u8; 32],
    /// The head-signing public key.
    pub head_public: [u8; 32],
    /// `SHA-256("HY-KEYID1\0" ‖ head_public)`.
    pub key_id: [u8; 32],
}

impl std::fmt::Debug for ServerKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerKeys")
            .field(
                "fingerprint",
                &hy_core::protocol::prefix8(&self.fingerprint),
            )
            .field("key_id", &hy_core::protocol::prefix8(&self.key_id))
            .finish_non_exhaustive()
    }
}

fn other(msg: &str) -> io::Error {
    io::Error::other(msg.to_owned())
}

const CERT_FILE: &str = "tls.cert.der";
const TLS_KEY_FILE: &str = "tls.key.dpapi";
const HEAD_KEY_FILE: &str = "head.key.dpapi";

impl ServerKeys {
    fn from_parts(
        cert_der: Vec<u8>,
        tls_key_pkcs8: Secret<Vec<u8>>,
        head_pkcs8: &[u8],
    ) -> io::Result<Self> {
        let head = Ed25519KeyPair::from_pkcs8(head_pkcs8)
            .map_err(|_| other("the head-signing key file is damaged"))?;
        let head_public = public_key(&head);
        Ok(Self {
            fingerprint: fingerprint(&cert_der),
            key_id: key_id(&head_public),
            head_public,
            cert_der,
            tls_key_pkcs8,
            head,
        })
    }

    /// New keys, in memory only (tests).
    ///
    /// # Errors
    /// An I/O error if key generation fails.
    pub fn generate(host_name: &str, year: i32) -> io::Result<Self> {
        let (cert, tls_key, head) = generate_parts(host_name, year)?;
        Self::from_parts(cert, tls_key, head.expose())
    }

    /// The TLS private key (PKCS#8 DER), for the rustls server only.
    #[must_use]
    pub fn tls_key_pkcs8(&self) -> &Secret<Vec<u8>> {
        &self.tls_key_pkcs8
    }

    /// The head-signing key pair.
    #[must_use]
    pub fn head_key(&self) -> &Ed25519KeyPair {
        &self.head
    }

    /// The key folder under a data directory.
    #[must_use]
    pub fn dir(data_dir: &Path) -> PathBuf {
        data_dir.join("keys")
    }

    /// True if keys exist under `data_dir`.
    #[must_use]
    pub fn exist(data_dir: &Path) -> bool {
        Self::dir(data_dir).join(CERT_FILE).exists()
    }

    /// Generate keys and write them under `data_dir/keys` (the first `init`). Refuses to
    /// overwrite existing keys: replacing them means every PC re-pairs.
    ///
    /// # Errors
    /// An I/O error, or `AlreadyExists` if keys are present.
    pub fn create(
        data_dir: &Path,
        protector: &dyn KeyProtector,
        host_name: &str,
        year: i32,
    ) -> io::Result<Self> {
        let dir = Self::dir(data_dir);
        if dir.join(CERT_FILE).exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "server keys already exist; replacing them means every PC must pair again",
            ));
        }
        fs::create_dir_all(&dir)?;
        let (cert, tls_key, head) = generate_parts(host_name, year)?;
        write_private(
            &dir.join(TLS_KEY_FILE),
            &protector.protect(tls_key.expose())?,
        )?;
        write_private(&dir.join(HEAD_KEY_FILE), &protector.protect(head.expose())?)?;
        write_private(&dir.join(CERT_FILE), &cert)?;
        Self::from_parts(cert, tls_key, head.expose())
    }

    /// Load the keys under `data_dir/keys`.
    ///
    /// # Errors
    /// An I/O error if a file is missing or cannot be opened on this machine.
    pub fn load(data_dir: &Path, protector: &dyn KeyProtector) -> io::Result<Self> {
        let dir = Self::dir(data_dir);
        let cert = fs::read(dir.join(CERT_FILE))?;
        let tls_key = protector.unprotect(&fs::read(dir.join(TLS_KEY_FILE))?)?;
        let head = protector.unprotect(&fs::read(dir.join(HEAD_KEY_FILE))?)?;
        Self::from_parts(cert, tls_key, head.expose())
    }
}

#[allow(clippy::type_complexity)]
fn generate_parts(
    host_name: &str,
    year: i32,
) -> io::Result<(Vec<u8>, Secret<Vec<u8>>, Secret<Vec<u8>>)> {
    let rng = SystemRandom::new();
    let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
        .map_err(|_| other("TLS key generation failed"))?;
    let mut names = vec![SNI.to_owned()];
    let host = host_name.trim().to_ascii_lowercase();
    if !host.is_empty()
        && host.len() <= 253
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        && host != SNI
    {
        names.push(host);
    }
    let mut params =
        rcgen::CertificateParams::new(names).map_err(|_| other("bad certificate names"))?;
    params.not_before = rcgen::date_time_ymd(year, 1, 1);
    params.not_after = rcgen::date_time_ymd(year + 20, 1, 1);
    let mut serial = [0u8; 16];
    rng.fill(&mut serial)
        .map_err(|_| other("random source failed"))?;
    serial[0] &= 0x7F;
    params.serial_number = Some(rcgen::SerialNumber::from_slice(&serial));
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, SNI);
    let cert = params
        .self_signed(&key)
        .map_err(|_| other("certificate generation failed"))?;
    let head =
        Ed25519KeyPair::generate_pkcs8(&rng).map_err(|_| other("head key generation failed"))?;
    Ok((
        cert.der().to_vec(),
        Secret::new(key.serialize_der()),
        Secret::new(head.as_ref().to_vec()),
    ))
}

/// The current year, for certificate validity.
#[must_use]
pub fn year_of(now: chrono::DateTime<chrono::Utc>) -> i32 {
    now.year()
}

pub(crate) fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path)?;
    io::Write::write_all(&mut f, bytes)?;
    f.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_created_once_and_load_back_identically() {
        let dir = tempfile::tempdir().unwrap();
        let p = UnprotectedForTests;
        let a = ServerKeys::create(dir.path(), &p, "OFFICE-PC", 2026).unwrap();
        let b = ServerKeys::load(dir.path(), &p).unwrap();
        assert_eq!(a.fingerprint, b.fingerprint);
        assert_eq!(a.key_id, b.key_id);
        assert_eq!(a.fingerprint, fingerprint(&b.cert_der));
        assert_eq!(a.key_id, key_id(&b.head_public));
        let again = ServerKeys::create(dir.path(), &p, "OFFICE-PC", 2026).unwrap_err();
        assert_eq!(again.kind(), io::ErrorKind::AlreadyExists);
        // Debug output never shows key material.
        assert!(!format!("{a:?}").contains(&hy_core::protocol::hex(&a.fingerprint)));
    }

    #[test]
    fn two_installs_have_different_identities() {
        let a = ServerKeys::generate("pc", 2026).unwrap();
        let b = ServerKeys::generate("pc", 2026).unwrap();
        assert_ne!(a.fingerprint, b.fingerprint);
        assert_ne!(a.key_id, b.key_id);
        assert_ne!(a.tls_key_pkcs8().expose(), b.tls_key_pkcs8().expose());
    }

    #[cfg(windows)]
    #[test]
    fn dpapi_machine_scope_round_trips_and_rejects_tampering() {
        // T-S11 (c) needs a standard user on the owner's server PC; this checks only that
        // the blobs are sealed and open on this machine.
        let p = DpapiMachine;
        let blob = p.protect(b"key material").unwrap();
        assert!(!blob.windows(12).any(|w| w == b"key material"));
        assert_eq!(p.unprotect(&blob).unwrap().expose(), b"key material");
        let mut bad = blob.clone();
        let last = bad.len() - 1;
        bad[last] ^= 1;
        assert!(p.unprotect(&bad).is_err());
    }
}
