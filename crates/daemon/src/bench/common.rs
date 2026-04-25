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

/// Dimension of the deterministic synthetic embedder used across all bench
/// harnesses (Forge-Consolidation, Forge-Identity, Forge-Isolation, ...).
/// Lifted from `bench/forge_consolidation.rs` per 2A-5 spec §2 fact 13.
pub const DETERMINISTIC_EMBEDDING_DIM: usize = 768;

/// Deterministic unit-vector embedding of dimension
/// [`DETERMINISTIC_EMBEDDING_DIM`] derived from a seed string. Same input →
/// byte-identical output. Used by bench harnesses to produce reproducible
/// synthetic vectors without invoking fastembed (keeps CI runtime sub-second).
pub fn deterministic_embedding(seed_key: &str) -> Vec<f32> {
    use rand::RngExt;
    let hash = sha256_hex(seed_key);
    let mut rng = seeded_rng(u64::from_str_radix(&hash[0..16], 16).unwrap_or(0));
    let raw: Vec<f32> = (0..DETERMINISTIC_EMBEDDING_DIM)
        .map(|_| rng.random_range(-1.0_f32..1.0_f32))
        .collect();
    let norm: f32 = raw.iter().map(|x| x * x).sum::<f32>().sqrt();
    raw.into_iter().map(|x| x / norm).collect()
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

    #[test]
    fn test_deterministic_embedding_is_deterministic() {
        let a = deterministic_embedding("hello world");
        let b = deterministic_embedding("hello world");
        assert_eq!(a, b, "same seed_key must produce byte-identical embedding");
    }

    #[test]
    fn test_deterministic_embedding_dim_matches_const() {
        let v = deterministic_embedding("any seed");
        assert_eq!(v.len(), DETERMINISTIC_EMBEDDING_DIM);
    }

    #[test]
    fn test_deterministic_embedding_is_unit_norm() {
        let v = deterministic_embedding("seed");
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (norm - 1.0).abs() < 1e-5,
            "embedding should be unit-norm, got {norm}"
        );
    }

    #[test]
    fn test_deterministic_embedding_distinct_seeds_distinct_output() {
        let a = deterministic_embedding("seed_a");
        let b = deterministic_embedding("seed_b");
        assert_ne!(a, b);
    }
}
