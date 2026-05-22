//! SHA-256 helpers used for content-addressing memory nodes and commits.
//!
//! memora keeps things git-like but simpler: every content-addressed object
//! is identified by its full lowercase hex SHA-256 digest. There is no
//! ambiguity around shortened ids in the storage layer; the CLI is free to
//! abbreviate for display.

use sha2::{Digest, Sha256};

/// Compute the lowercase hex SHA-256 digest of the given bytes.
///
/// ```
/// use memora_core::hash::sha256_hex;
/// assert_eq!(sha256_hex(b"").len(), 64);
/// ```
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// Abbreviate a full hash for human-readable output. Returns the first
/// `len` characters (default callers typically use 7, like git).
pub fn short(hash: &str, len: usize) -> &str {
    let take = len.min(hash.len());
    &hash[..take]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_string_has_known_digest() {
        // SHA-256 of the empty string — well known constant.
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn short_clamps_to_length() {
        let h = sha256_hex(b"hello");
        assert_eq!(short(&h, 7).len(), 7);
        assert_eq!(short(&h, 1000).len(), 64);
    }
}
