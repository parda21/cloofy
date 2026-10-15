//! Generic passwords in the login keychain, under the service "cloofy".

use std::io;

use security_framework::passwords;

pub const AVAILABLE: bool = true;
const SERVICE: &str = "cloofy";
/// `errSecItemNotFound`.
const NOT_FOUND: i32 = -25300;

pub fn get(name: &str) -> io::Result<Option<Vec<u8>>> {
    match passwords::get_generic_password(SERVICE, name) {
        Ok(secret) => Ok(Some(secret)),
        Err(err) if err.code() == NOT_FOUND => Ok(None),
        Err(err) => Err(io::Error::other(err)),
    }
}

pub fn set(name: &str, secret: &[u8]) -> io::Result<()> {
    passwords::set_generic_password(SERVICE, name, secret).map_err(io::Error::other)
}

pub fn delete(name: &str) -> io::Result<()> {
    match passwords::delete_generic_password(SERVICE, name) {
        Err(err) if err.code() != NOT_FOUND => Err(io::Error::other(err)),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    /// Writes to the login keychain, which may ask for permission: run it by hand with
    /// `cargo test -p cloofy-os -- --ignored`.
    #[test]
    #[ignore = "uses the login keychain"]
    fn keeps_and_forgets_a_secret() {
        let name = format!("test-{}", std::process::id());
        assert_eq!(super::get(&name).unwrap(), None);
        super::set(&name, b"one").unwrap();
        super::set(&name, b"two").unwrap();
        assert_eq!(super::get(&name).unwrap().as_deref(), Some(&b"two"[..]));
        super::delete(&name).unwrap();
        super::delete(&name).unwrap();
        assert_eq!(super::get(&name).unwrap(), None);
    }
}
