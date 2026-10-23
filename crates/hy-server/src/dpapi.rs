//! Machine-scope DPAPI for the server's key files (V1-5 contract 6.1 and 6.2).
//!
//! The only module in `hy-server` that is allowed `unsafe` code: two Win32 calls,
//! `CryptProtectData` and `CryptUnprotectData`, with their output freed by `LocalFree`.
//! Each call has a SAFETY note.
#![allow(unsafe_code)]

use std::io;

use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Cryptography::{
    CRYPT_INTEGER_BLOB, CRYPTPROTECT_LOCAL_MACHINE, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData,
    CryptUnprotectData,
};

fn blob(bytes: &[u8]) -> io::Result<CRYPT_INTEGER_BLOB> {
    Ok(CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(bytes.len()).map_err(|_| io::Error::other("blob too large"))?,
        pbData: bytes.as_ptr().cast_mut(),
    })
}

/// Take ownership of a DPAPI output blob: copy it out and free it.
fn take(out: &CRYPT_INTEGER_BLOB) -> Vec<u8> {
    if out.pbData.is_null() {
        return Vec::new();
    }
    // SAFETY: on success DPAPI returns `cbData` bytes at `pbData`, allocated with
    // LocalAlloc; we copy them, overwrite the OS buffer (it may hold key material), and
    // free it exactly once.
    unsafe {
        let len = out.cbData as usize;
        let v = std::slice::from_raw_parts(out.pbData, len).to_vec();
        std::ptr::write_bytes(out.pbData, 0, len);
        LocalFree(out.pbData.cast());
        v
    }
}

/// Seal `plain` for this machine.
///
/// # Errors
/// The OS error if DPAPI refuses.
pub fn protect_machine(plain: &[u8], entropy: &[u8]) -> io::Result<Vec<u8>> {
    let input = blob(plain)?;
    let ent = blob(entropy)?;
    let mut out = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    // SAFETY: `input` and `ent` point to live slices for the duration of the call; DPAPI
    // only reads them. `out` is a valid out-pointer; the description, reserved and prompt
    // arguments may be null.
    let ok = unsafe {
        CryptProtectData(
            &input,
            std::ptr::null(),
            &ent,
            std::ptr::null(),
            std::ptr::null(),
            CRYPTPROTECT_LOCAL_MACHINE | CRYPTPROTECT_UI_FORBIDDEN,
            &mut out,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(take(&out))
}

/// Open a blob sealed by [`protect_machine`] on this machine.
///
/// # Errors
/// The OS error if the blob cannot be opened here (another machine, tampering).
pub fn unprotect_machine(sealed: &[u8], entropy: &[u8]) -> io::Result<Vec<u8>> {
    let input = blob(sealed)?;
    let ent = blob(entropy)?;
    let mut out = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    // SAFETY: as in `protect_machine`; the description out-pointer may be null.
    let ok = unsafe {
        CryptUnprotectData(
            &input,
            std::ptr::null_mut(),
            &ent,
            std::ptr::null(),
            std::ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(take(&out))
}
