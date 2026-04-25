//! Shared helpers for Forge-* benchmark harnesses.

use sha2::{Digest, Sha256};

/// Convert a byte slice to a lowercase hex string.
pub fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Create a deterministic ChaCha20 PRNG from a u64 seed.
pub fn seeded_rng(seed: u64) -> rand_chacha::ChaCha20Rng {
    use rand::SeedableRng;
    rand_chacha::ChaCha20Rng::seed_from_u64(seed)
}

/// Generate a SHA-256 hex digest of the given input string.
/// Used to create unique tokens that resist semantic dedup.
pub fn sha256_hex(input: &str) -> String {
    bytes_to_hex(&Sha256::digest(input.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bytes_to_hex_known_value() {
        assert_eq!(bytes_to_hex(&[0xde, 0xad, 0xbe, 0xef]), "deadbeef");
    }

    #[test]
    fn test_bytes_to_hex_empty() {
        assert_eq!(bytes_to_hex(&[]), "");
    }

    #[test]
    fn test_sha256_hex_deterministic() {
        let a = sha256_hex("hello");
        let b = sha256_hex("hello");
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn test_seeded_rng_deterministic() {
        use rand::RngExt;
        let mut rng1 = seeded_rng(42);
        let mut rng2 = seeded_rng(42);
        let v1: u64 = rng1.random();
        let v2: u64 = rng2.random();
        assert_eq!(v1, v2, "same seed must produce same sequence");
    }
}
