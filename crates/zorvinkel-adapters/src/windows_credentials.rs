//! Thin access to the Windows Credential Manager (generic credentials of the current user).
//! The only unsafe code of the Windows client lives here. Items are persisted per machine and
//! are never roamed; reading or writing needs no extra approval for the owning user.
#![allow(unsafe_code)]
use std::ptr::{null_mut, slice_from_raw_parts};
use windows_sys::Win32::{
    Foundation::{ERROR_NOT_FOUND, GetLastError},
    Security::Credentials::{
        CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree,
        CredReadW, CredWriteW,
    },
};
use zeroize::Zeroizing;

/// The system limit for one credential blob (5 * 512 bytes).
pub(crate) const MAX_BLOB: usize = 2560;

/// Any failure except a missing item; the system's reason is deliberately not exposed.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Failed;

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// Missing is distinct from every other failure.
pub(crate) fn read(target: &str) -> Result<Option<Zeroizing<Vec<u8>>>, Failed> {
    let name = wide(target);
    let mut found: *mut CREDENTIALW = null_mut();
    // SAFETY: `name` is NUL-terminated and `found` is a valid out pointer.
    if unsafe { CredReadW(name.as_ptr(), CRED_TYPE_GENERIC, 0, &mut found) } == 0 {
        // SAFETY: reads the thread's last error right after the failed call.
        return match unsafe { GetLastError() } {
            ERROR_NOT_FOUND => Ok(None),
            _ => Err(Failed),
        };
    }
    // SAFETY: on success `found` points to a credential owned by the system until freed.
    let credential = unsafe { &*found };
    let size = credential.CredentialBlobSize as usize;
    let bytes = if size == 0 || credential.CredentialBlob.is_null() {
        Zeroizing::new(Vec::new())
    } else {
        // SAFETY: the blob holds `size` readable bytes.
        Zeroizing::new(unsafe { &*slice_from_raw_parts(credential.CredentialBlob, size) }.to_vec())
    };
    if size != 0 && !credential.CredentialBlob.is_null() {
        // SAFETY: the buffer is ours until `CredFree`; wipe the system's copy of the secret.
        unsafe { std::ptr::write_bytes(credential.CredentialBlob, 0, size) };
    }
    // SAFETY: `found` was returned by `CredReadW` and is not used afterwards.
    unsafe { CredFree(found.cast()) };
    Ok(Some(bytes))
}

/// Creates the item or replaces its content.
pub(crate) fn write(target: &str, value: &[u8]) -> Result<(), Failed> {
    if value.len() > MAX_BLOB {
        return Err(Failed);
    }
    let mut name = wide(target);
    let mut user = wide("zorvinkel");
    // SAFETY: the structure is plain data for which all-zero is valid (null pointers, zeros).
    let mut credential: CREDENTIALW = unsafe { std::mem::zeroed() };
    credential.Type = CRED_TYPE_GENERIC;
    credential.TargetName = name.as_mut_ptr();
    credential.UserName = user.as_mut_ptr();
    credential.CredentialBlobSize = value.len() as u32;
    // `CredWriteW` only reads the blob.
    credential.CredentialBlob = value.as_ptr().cast_mut();
    credential.Persist = CRED_PERSIST_LOCAL_MACHINE;
    // SAFETY: all pointers stay valid and NUL-terminated or sized for the duration of the call.
    if unsafe { CredWriteW(&credential, 0) } == 0 {
        return Err(Failed);
    }
    Ok(())
}

/// Removing a missing item succeeds.
pub(crate) fn delete(target: &str) -> Result<(), Failed> {
    let name = wide(target);
    // SAFETY: `name` is NUL-terminated.
    if unsafe { CredDeleteW(name.as_ptr(), CRED_TYPE_GENERIC, 0) } == 0 {
        // SAFETY: reads the thread's last error right after the failed call.
        return match unsafe { GetLastError() } {
            ERROR_NOT_FOUND => Ok(()),
            _ => Err(Failed),
        };
    }
    Ok(())
}
