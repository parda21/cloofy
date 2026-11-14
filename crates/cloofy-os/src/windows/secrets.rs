//! Generic credentials in Windows' Credential Manager, under the target "cloofy:<name>".
//!
//! A credential holds 2560 bytes at most; anything longer is refused, and cloofy keeps such a
//! secret in a file instead.

use std::{ffi::c_void, io, ptr};

use windows::{
    Win32::{
        Foundation::ERROR_NOT_FOUND,
        Security::Credentials::{
            CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree, CredReadW, CredWriteW,
        },
    },
    core::{HRESULT, PCWSTR, PWSTR},
};

pub const AVAILABLE: bool = true;

/// The longest secret Credential Manager takes, in bytes.
const MAX_LEN: usize = 2560;

fn target(name: &str) -> Vec<u16> {
    format!("cloofy:{name}").encode_utf16().chain(Some(0)).collect()
}

pub fn get(name: &str) -> io::Result<Option<Vec<u8>>> {
    let target = target(name);
    let mut credential: *mut CREDENTIALW = ptr::null_mut();
    // SAFETY: the target outlives the call, and what the call hands back is freed below.
    if let Err(err) = unsafe { CredReadW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, None, &mut credential) } {
        return match err.code() == HRESULT::from_win32(ERROR_NOT_FOUND.0) {
            true => Ok(None),
            false => Err(io::Error::other(err)),
        };
    }
    // SAFETY: a successful read hands back one credential, and an empty blob is no pointer at all.
    let secret = unsafe {
        let credential = &*credential;
        let secret = match credential.CredentialBlobSize {
            0 => Vec::new(),
            len => std::slice::from_raw_parts(credential.CredentialBlob, len as usize).to_vec(),
        };
        CredFree(credential as *const CREDENTIALW as *const c_void);
        secret
    };
    Ok(Some(secret))
}

pub fn set(name: &str, secret: &[u8]) -> io::Result<()> {
    check_len(secret)?;
    let mut target = target(name);
    let credential = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(target.as_mut_ptr()),
        CredentialBlobSize: secret.len() as u32,
        // The call copies the blob, it doesn't write through it.
        CredentialBlob: secret.as_ptr().cast_mut(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        ..Default::default()
    };
    // SAFETY: every pointer in the credential points at memory that outlives the call.
    unsafe { CredWriteW(&credential, 0) }.map_err(io::Error::other)
}

pub fn delete(name: &str) -> io::Result<()> {
    let target = target(name);
    // SAFETY: the target outlives the call.
    match unsafe { CredDeleteW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, None) } {
        Err(err) if err.code() != HRESULT::from_win32(ERROR_NOT_FOUND.0) => Err(io::Error::other(err)),
        _ => Ok(()),
    }
}

/// Refuses a secret Credential Manager can't hold, so the caller can keep it elsewhere.
fn check_len(secret: &[u8]) -> io::Result<()> {
    match secret.len() <= MAX_LEN {
        true => Ok(()),
        false => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Credential Manager holds {MAX_LEN} bytes at most"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_a_secret_that_does_not_fit() {
        assert!(check_len(&[0; MAX_LEN]).is_ok());
        assert_eq!(check_len(&[0; MAX_LEN + 1]).unwrap_err().kind(), io::ErrorKind::InvalidInput);
    }

    /// Writes to the user's credential store, which the tests shouldn't: run it by hand with
    /// `cargo test -p cloofy-os -- --ignored`.
    #[test]
    #[ignore = "uses the credential store"]
    fn keeps_and_forgets_a_secret() {
        let name = format!("test-{}", std::process::id());
        assert_eq!(get(&name).unwrap(), None);
        set(&name, b"one").unwrap();
        set(&name, b"two").unwrap();
        assert_eq!(get(&name).unwrap().as_deref(), Some(&b"two"[..]));
        delete(&name).unwrap();
        delete(&name).unwrap();
        assert_eq!(get(&name).unwrap(), None);
    }
}
