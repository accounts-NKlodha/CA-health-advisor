//! Checking the server data folder's rights (slice V1-6b; V1-5 contract 6.2: the data
//! folder is open to SYSTEM, Administrators and the service's own account only).
//!
//! The installer sets the owner and the access list of the whole tree, then runs
//! `hy-server check-acl`, which reads every file's and folder's owner and access list
//! back and fails if anything else could open it: an owner outside the allowed accounts,
//! an access entry for any other account (explicit or inherited), a folder whose list is
//! still inherited from above, or no access list at all (which means everyone). The rules
//! are in [`evaluate`] (tested on every platform); reading them from Windows is in
//! [`read_tree`], the one other place in `hy-server` allowed `unsafe` code, with a SAFETY
//! note for each call.

use std::path::{Path, PathBuf};

/// SYSTEM.
pub const SYSTEM: &str = "S-1-5-18";
/// The local Administrators group.
pub const ADMINISTRATORS: &str = "S-1-5-32-544";

/// One access entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ace {
    /// The entry type (0 allow, 1 deny, others are object or callback entries).
    pub kind: u8,
    /// The account, as a string SID.
    pub sid: String,
}

/// What Windows says about one file or folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Where it is.
    pub path: PathBuf,
    /// Its owner, as a string SID.
    pub owner: String,
    /// Whether its access list is protected from inheritance.
    pub protected: bool,
    /// Its access list; `None` is no list at all (full access for everyone).
    pub aces: Option<Vec<Ace>>,
}

/// Every way the tree breaks the rule, worded for an administrator (SIDs and paths
/// inside the data folder only).
#[must_use]
pub fn evaluate(root: &Path, entries: &[Entry], allowed: &[&str]) -> Vec<String> {
    let ok = |sid: &str| allowed.contains(&sid);
    let mut problems = Vec::new();
    for e in entries {
        let name = e.path.strip_prefix(root).map_or_else(
            |_| e.path.display().to_string(),
            |p| {
                if p.as_os_str().is_empty() {
                    ".".to_owned()
                } else {
                    p.display().to_string()
                }
            },
        );
        if !ok(&e.owner) {
            problems.push(format!("{name}: owned by {}", e.owner));
        }
        if e.path == root && !e.protected {
            problems.push(format!(
                "{name}: still inherits rights from the folder above"
            ));
        }
        match &e.aces {
            None => problems.push(format!("{name}: no access list (everyone has access)")),
            Some(aces) => {
                for a in aces {
                    match a.kind {
                        // Deny entries take rights away; they cannot open anything.
                        1 => {}
                        0 if ok(&a.sid) => {}
                        0 => problems.push(format!("{name}: access for {}", a.sid)),
                        k => problems.push(format!("{name}: access entry of type {k}")),
                    }
                }
            }
        }
    }
    problems
}

/// Read the owner and access list of `root` and everything under it.
///
/// # Errors
/// A file or folder whose security cannot be read (that is itself a failure: the check
/// fails closed).
#[cfg(windows)]
pub fn read_tree(root: &Path) -> Result<Vec<Entry>, String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(p) = stack.pop() {
        out.push(win::read(&p).map_err(|e| format!("{}: {e}", p.display()))?);
        let meta = std::fs::symlink_metadata(&p).map_err(|e| format!("{}: {e}", p.display()))?;
        if meta.is_dir() {
            for item in std::fs::read_dir(&p).map_err(|e| format!("{}: {e}", p.display()))? {
                let item = item.map_err(|e| format!("{}: {e}", p.display()))?;
                stack.push(item.path());
            }
        }
    }
    Ok(out)
}

/// The string SID of an account name (for the service's virtual account).
///
/// # Errors
/// The name does not resolve.
#[cfg(windows)]
pub fn sid_of(name: &str) -> Result<String, String> {
    win::sid_of(name)
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod win {
    use super::{Ace, Entry};
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use windows_sys::Win32::Foundation::{ERROR_SUCCESS, LocalFree};
    use windows_sys::Win32::Security::Authorization::{
        ConvertSidToStringSidW, GetNamedSecurityInfoW, SE_FILE_OBJECT,
    };
    use windows_sys::Win32::Security::{
        ACE_HEADER, ACL, ACL_SIZE_INFORMATION, AclSizeInformation, DACL_SECURITY_INFORMATION,
        GetAce, GetAclInformation, GetSecurityDescriptorControl, LookupAccountNameW,
        OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SE_DACL_PROTECTED, SID_NAME_USE,
    };

    fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
        s.encode_wide().chain(std::iter::once(0)).collect()
    }

    fn sid_string(sid: PSID) -> Result<String, String> {
        let mut out: *mut u16 = std::ptr::null_mut();
        // SAFETY: `sid` points into a live security descriptor (or buffer); on success
        // Windows allocates `out` with LocalAlloc, which is read up to its NUL and freed
        // once.
        unsafe {
            if ConvertSidToStringSidW(sid, &raw mut out) == 0 || out.is_null() {
                return Err("an account could not be read".into());
            }
            let mut len = 0;
            while *out.add(len) != 0 {
                len += 1;
            }
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(out, len));
            LocalFree(out.cast());
            Ok(s)
        }
    }

    pub fn read(path: &Path) -> Result<Entry, String> {
        let name = wide(path.as_os_str());
        let mut owner: PSID = std::ptr::null_mut();
        let mut dacl: *mut ACL = std::ptr::null_mut();
        let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        // SAFETY: a live NUL-terminated name; the out pointers are valid. On success
        // `owner` and `dacl` point into `sd`, which is freed once with LocalFree after the
        // last use of either.
        unsafe {
            let rc = GetNamedSecurityInfoW(
                name.as_ptr(),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                &raw mut owner,
                std::ptr::null_mut(),
                &raw mut dacl,
                std::ptr::null_mut(),
                &raw mut sd,
            );
            if rc != ERROR_SUCCESS {
                return Err(format!("its rights could not be read (error {rc})"));
            }
            let result = (|| {
                let owner = sid_string(owner)?;
                let mut control = 0u16;
                let mut revision = 0u32;
                if GetSecurityDescriptorControl(sd, &raw mut control, &raw mut revision) == 0 {
                    return Err("its rights could not be read".to_owned());
                }
                let aces = if dacl.is_null() {
                    None
                } else {
                    let mut info = ACL_SIZE_INFORMATION::default();
                    if GetAclInformation(
                        dacl,
                        (&raw mut info).cast(),
                        u32::try_from(std::mem::size_of::<ACL_SIZE_INFORMATION>())
                            .unwrap_or(u32::MAX),
                        AclSizeInformation,
                    ) == 0
                    {
                        return Err("its access list could not be read".to_owned());
                    }
                    let mut v = Vec::new();
                    for i in 0..info.AceCount {
                        let mut ace: *mut core::ffi::c_void = std::ptr::null_mut();
                        if GetAce(dacl, i, &raw mut ace) == 0 || ace.is_null() {
                            return Err("an access entry could not be read".to_owned());
                        }
                        let header = *ace.cast::<ACE_HEADER>();
                        // Allow and deny entries: header, mask, then the SID.
                        let sid = if header.AceType <= 1 {
                            sid_string(ace.cast::<u8>().add(8).cast())?
                        } else {
                            String::new()
                        };
                        v.push(Ace {
                            kind: header.AceType,
                            sid,
                        });
                    }
                    Some(v)
                };
                Ok(Entry {
                    path: path.to_path_buf(),
                    owner,
                    protected: control & SE_DACL_PROTECTED != 0,
                    aces,
                })
            })();
            LocalFree(sd.cast());
            result
        }
    }

    pub fn sid_of(account: &str) -> Result<String, String> {
        let name = wide(std::ffi::OsStr::new(account));
        let mut sid = vec![0u8; 256];
        let mut sid_len = 256u32;
        let mut domain = vec![0u16; 256];
        let mut domain_len = 256u32;
        let mut use_: SID_NAME_USE = 0;
        // SAFETY: live buffers with their lengths passed; the SID is converted while the
        // buffer is alive.
        unsafe {
            if LookupAccountNameW(
                std::ptr::null(),
                name.as_ptr(),
                sid.as_mut_ptr().cast(),
                &raw mut sid_len,
                domain.as_mut_ptr(),
                &raw mut domain_len,
                &raw mut use_,
            ) == 0
            {
                return Err(format!("the account {account} was not found"));
            }
            sid_string(sid.as_mut_ptr().cast())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SERVICE: &str = "S-1-5-80-1-2-3-4-5";
    const USERS: &str = "S-1-5-32-545";

    fn allowed() -> [&'static str; 3] {
        [SYSTEM, ADMINISTRATORS, SERVICE]
    }

    fn entry(path: &str, owner: &str, protected: bool, sids: &[&str]) -> Entry {
        Entry {
            path: PathBuf::from(path),
            owner: owner.into(),
            protected,
            aces: Some(
                sids.iter()
                    .map(|s| Ace {
                        kind: 0,
                        sid: (*s).into(),
                    })
                    .collect(),
            ),
        }
    }

    #[test]
    fn only_the_trusted_accounts_may_own_or_open_the_data() {
        let root = Path::new("/data");
        let good = vec![
            entry(
                "/data",
                ADMINISTRATORS,
                true,
                &[SYSTEM, ADMINISTRATORS, SERVICE],
            ),
            entry(
                "/data/keys",
                SERVICE,
                false,
                &[SYSTEM, ADMINISTRATORS, SERVICE],
            ),
            entry(
                "/data/keys/tls.key.dpapi",
                SYSTEM,
                false,
                &[SYSTEM, ADMINISTRATORS, SERVICE],
            ),
        ];
        assert!(evaluate(root, &good, &allowed()).is_empty());

        // A leftover Users grant on a key file, an untrusted owner, an unprotected root,
        // no access list, and an odd entry type are each reported.
        let mut bad = good.clone();
        bad[2].aces.as_mut().unwrap().push(Ace {
            kind: 0,
            sid: USERS.into(),
        });
        bad[1].owner = "S-1-5-21-1-2-3-1001".into();
        bad[0].protected = false;
        bad.push(Entry {
            path: PathBuf::from("/data/server.db"),
            owner: ADMINISTRATORS.into(),
            protected: false,
            aces: None,
        });
        bad.push(Entry {
            path: PathBuf::from("/data/logs"),
            owner: ADMINISTRATORS.into(),
            protected: false,
            aces: Some(vec![Ace {
                kind: 9,
                sid: String::new(),
            }]),
        });
        let p = evaluate(root, &bad, &allowed());
        assert_eq!(p.len(), 5, "{p:?}");
        assert!(p.iter().any(|x| x.contains(USERS)));
        assert!(p.iter().any(|x| x.contains("owned by S-1-5-21")));
        assert!(p.iter().any(|x| x.contains("inherits")));
        assert!(p.iter().any(|x| x.contains("everyone")));
        assert!(p.iter().any(|x| x.contains("type 9")));

        // A deny entry is harmless.
        let mut deny = good;
        deny[1].aces.as_mut().unwrap().push(Ace {
            kind: 1,
            sid: USERS.into(),
        });
        assert!(evaluate(root, &deny, &allowed()).is_empty());
    }
}
