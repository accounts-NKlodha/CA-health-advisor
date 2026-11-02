//! Companion updates from the office server (architecture 8; W1).
//!
//! **The drop folder.** The administrator copies the two files of a companion release,
//! `HealthierYou_<version>_x64-setup.exe` and its `.sig`, exactly as the owner signed
//! them, into `<data folder>\updates` (made by the service at start, so it has the data
//! folder's rights: SYSTEM, Administrators and the service account only). A drop folder
//! rather than an upload on the admin page: the page is a small form-only page on the
//! server PC itself (loopback, urlencoded forms, no file inputs), so an administrator who
//! can open it can already copy files on that PC, and taking a 200 MB upload would need
//! a much larger request limit and a streaming upload path on the admin listener. The
//! admin page shows what is offered ([`current`]).
//!
//! **What is offered.** The companion installer with the highest version in the folder
//! (a server installer, other files, links and anything too large are ignored), with its
//! `.sig` if there is one. The server checks names and sizes only: it holds no secret,
//! vouches for nothing, and is never trusted for authenticity. Every PC checks the
//! signature with the release key built into it, and the version rule, before anything
//! runs (`hy_core::update::check_update_file`), exactly as for a file chosen from a
//! share. A missing `.sig` is still offered so that PCs refuse it with the file route's
//! message; the admin page warns about it.
//!
//! **Served** by `GET /v1/update/manifest`, `/v1/update/signature?file=` and
//! `/v1/update/installer?file=&offset=` ([`crate::api`]): pinned TLS, a confirmed PC's
//! token, the connection limits of every other endpoint, and the installer in pieces of
//! at most [`UPDATE_CHUNK`] so each request fits the connection cap.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use hy_core::protocol::wire::UPDATE_CHUNK;
use hy_core::update::{
    self, MAX_INSTALLER_BYTES, MAX_SIG_BYTES, Product, Version, parse_installer_name,
};

/// The drop folder's name inside the data folder.
pub const DIR_NAME: &str = "updates";

/// The offered companion installer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    /// Its file name.
    pub file_name: String,
    /// The version its name says (not verified here).
    pub version: Version,
    /// Its size in bytes.
    pub size: u64,
    /// Where it is.
    pub installer: PathBuf,
    /// Whether a `.sig` of a plausible size sits beside it.
    pub has_signature: bool,
}

/// A regular file (not a link) no larger than `max`: its size.
fn plain_file(path: &Path, max: u64) -> Option<u64> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    (meta.file_type().is_file() && meta.len() <= max).then_some(meta.len())
}

/// The installer offered from `dir`, if any (see the module documentation).
#[must_use]
pub fn current(dir: &Path) -> Option<Offer> {
    let mut best: Option<Offer> = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let Ok(file_name) = entry.file_name().into_string() else {
            continue;
        };
        let Some((Product::Companion, version)) = parse_installer_name(&file_name) else {
            continue;
        };
        if best.as_ref().is_some_and(|b| b.version >= version) {
            continue;
        }
        let installer = dir.join(&file_name);
        let Some(size) = plain_file(&installer, MAX_INSTALLER_BYTES) else {
            continue;
        };
        if size == 0 {
            continue;
        }
        let has_signature = plain_file(&update::sig_path_for(&installer), MAX_SIG_BYTES).is_some();
        best = Some(Offer {
            file_name,
            version,
            size,
            installer,
            has_signature,
        });
    }
    best
}

/// The offer's `.sig` text, if it has one.
#[must_use]
pub fn signature(offer: &Offer) -> Option<String> {
    let path = update::sig_path_for(&offer.installer);
    plain_file(&path, MAX_SIG_BYTES)?;
    std::fs::read_to_string(path).ok()
}

/// At most [`UPDATE_CHUNK`] bytes of the offer's installer from `offset` (`offset` must
/// be below its size).
///
/// # Errors
/// The file could not be read.
pub fn chunk(offer: &Offer, offset: u64) -> std::io::Result<Vec<u8>> {
    let mut file = std::fs::File::open(&offer.installer)?;
    file.seek(SeekFrom::Start(offset))?;
    let mut out = Vec::new();
    file.take(UPDATE_CHUNK as u64).read_to_end(&mut out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn the_highest_companion_installer_is_offered_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        assert_eq!(current(d), None, "an empty folder offers nothing");
        assert_eq!(current(&d.join("missing")), None);
        std::fs::write(d.join("HealthierYou_0.2.0_x64-setup.exe"), b"two").unwrap();
        std::fs::write(d.join("HealthierYou_0.10.0_x64-setup.exe"), b"ten").unwrap();
        std::fs::write(d.join("HealthierYou_0.10.0_x64-setup.exe.sig"), b"sig").unwrap();
        std::fs::write(d.join("HealthierYou-Server_9.0.0_x64-setup.exe"), b"srv").unwrap();
        std::fs::write(d.join("HealthierYou_9.0.0_x64-setup (1).exe"), b"copy").unwrap();
        std::fs::write(d.join("HealthierYou_8.0.0_x64-setup.exe"), b"").unwrap();
        std::fs::create_dir(d.join("HealthierYou_7.0.0_x64-setup.exe")).unwrap();
        let o = current(d).unwrap();
        assert_eq!(o.file_name, "HealthierYou_0.10.0_x64-setup.exe");
        assert_eq!(o.version, Version::new(0, 10, 0));
        assert_eq!(o.size, 3);
        assert!(o.has_signature);
        assert_eq!(signature(&o).as_deref(), Some("sig"));
        assert_eq!(chunk(&o, 1).unwrap(), b"en");
        // Without its signature it is still offered (the PCs refuse it), and says so.
        std::fs::remove_file(d.join("HealthierYou_0.10.0_x64-setup.exe.sig")).unwrap();
        let o = current(d).unwrap();
        assert!(!o.has_signature);
        assert_eq!(signature(&o), None);
        // A signature too large to be one is not served.
        std::fs::write(
            d.join("HealthierYou_0.10.0_x64-setup.exe.sig"),
            vec![b'a'; usize::try_from(MAX_SIG_BYTES).unwrap() + 1],
        )
        .unwrap();
        let o = current(d).unwrap();
        assert!(!o.has_signature);
        assert_eq!(signature(&o), None);
    }

    #[test]
    fn the_installer_is_read_in_pieces() {
        let dir = tempfile::tempdir().unwrap();
        let name = "HealthierYou_1.0.0_x64-setup.exe";
        let bytes: Vec<u8> = (0..UPDATE_CHUNK + 10).map(|i| (i % 251) as u8).collect();
        std::fs::write(dir.path().join(name), &bytes).unwrap();
        let o = current(dir.path()).unwrap();
        let first = chunk(&o, 0).unwrap();
        assert_eq!(first.len(), UPDATE_CHUNK);
        let rest = chunk(&o, UPDATE_CHUNK as u64).unwrap();
        assert_eq!(rest.len(), 10);
        assert_eq!([first, rest].concat(), bytes);
    }
}
