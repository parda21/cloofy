//! The system's store for secrets: the Keychain on macOS.

use std::io;

use crate::os;

/// Whether this system has a store cloofy can keep secrets in.
pub fn available() -> bool {
    os::secrets::AVAILABLE
}

/// The secret saved under `name`, if any.
pub fn get(name: &str) -> io::Result<Option<Vec<u8>>> {
    os::secrets::get(name)
}

/// Saves `secret` under `name`, replacing what was there.
pub fn set(name: &str, secret: &[u8]) -> io::Result<()> {
    os::secrets::set(name, secret)
}

/// Removes the secret saved under `name`; fine when there is none.
pub fn delete(name: &str) -> io::Result<()> {
    os::secrets::delete(name)
}
