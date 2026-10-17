//! SHA-256 of a file, via the system's CNG (`bcrypt.dll`).

use std::io;
use std::path::Path;

use windows::Win32::Security::Cryptography::{BCRYPT_SHA256_ALG_HANDLE, BCryptHash};

/// Length of a SHA-256 digest in bytes.
pub const SHA256_LEN: usize = 32;

/// Hashes `data` with SHA-256.
///
/// # Errors
///
/// Returns an error if the system's hash provider reports a failure.
pub fn sha256(data: &[u8]) -> Result<[u8; SHA256_LEN], io::Error> {
    let mut digest = [0u8; SHA256_LEN];
    // SAFETY: `BCRYPT_SHA256_ALG_HANDLE` is a predefined pseudo handle that needs no opening or
    // closing; the input and output slices are valid for the call and `digest` is exactly the
    // digest length.
    let status = unsafe { BCryptHash(BCRYPT_SHA256_ALG_HANDLE, None, data, &mut digest) };
    if status.is_ok() {
        Ok(digest)
    } else {
        Err(io::Error::other(format!(
            "BCryptHash failed: {:#x}",
            status.0
        )))
    }
}

/// Lowercase hex text of `bytes`.
#[must_use]
pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;

    bytes.iter().fold(String::new(), |mut text, byte| {
        let _ = write!(text, "{byte:02x}");
        text
    })
}

/// Hashes the file at `path` and returns the digest as lowercase hex.
///
/// # Errors
///
/// Returns an error if the file can't be read or hashing fails.
pub fn sha256_file_hex(path: &Path) -> Result<String, io::Error> {
    Ok(hex(&sha256(&std::fs::read(path)?)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_the_empty_input() {
        assert_eq!(
            hex(&sha256(b"").unwrap()),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
