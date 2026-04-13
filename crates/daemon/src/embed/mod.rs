// embed/ — raw layer embedder.
//
// The `Embedder` trait is the only surface the raw ingest path depends on.
// Implementations are pluggable (for tests we use a deterministic fake; in
// production we use `minilm::MiniLMEmbedder` backed by fastembed-rs).
//
// **Shared instance.** Production runs install a single `Arc<dyn Embedder>`
// into `GLOBAL_EMBEDDER` at daemon startup. Every `DaemonState` (worker,
// writer, per-connection readers) reads from that global, so the model is
// loaded exactly once. Tests bypass the global entirely by setting
// `DaemonState::raw_embedder` to a `FakeEmbedder` per-instance — no global
// mutation, no parallel-test races.

pub mod minilm;

use std::fmt;
use std::sync::{Arc, OnceLock};

/// A pluggable text embedder. Implementations must be thread-safe.
pub trait Embedder: Send + Sync {
    /// Dimensionality of the output embeddings. Must be stable across calls.
    fn dim(&self) -> usize;

    /// Produce one embedding per input text. Order preserved.
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError>;
}

/// Errors surfaced by the embedder layer.
#[derive(Debug)]
pub enum EmbedError {
    /// Failed to load / initialize the underlying model.
    Init(String),
    /// Inference failed at call time.
    Inference(String),
    /// Returned embedding dimension did not match the trait's `dim()` contract.
    DimensionMismatch { expected: usize, actual: usize },
}

impl fmt::Display for EmbedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EmbedError::Init(msg) => write!(f, "embedder init failed: {msg}"),
            EmbedError::Inference(msg) => write!(f, "embedder inference failed: {msg}"),
            EmbedError::DimensionMismatch { expected, actual } => write!(
                f,
                "embedder dimension mismatch: expected {expected}, got {actual}"
            ),
        }
    }
}

impl std::error::Error for EmbedError {}

/// Process-wide embedder, set once at daemon startup.
///
/// Use [`install_global_embedder`] to populate it and [`global_embedder`] to
/// read it. Handlers and workers should prefer this over loading their own
/// model — fastembed's MiniLM weights are ~90 MB and we want to amortize the
/// initial load across the whole process.
static GLOBAL_EMBEDDER: OnceLock<Arc<dyn Embedder>> = OnceLock::new();

/// Install the process-wide embedder. Returns `Err` if an embedder was already
/// installed (this happens in test runs that share a binary, which is fine —
/// callers should treat the error as a no-op).
pub fn install_global_embedder(emb: Arc<dyn Embedder>) -> Result<(), Arc<dyn Embedder>> {
    GLOBAL_EMBEDDER.set(emb)
}

/// Read the process-wide embedder, if one was installed via
/// [`install_global_embedder`]. Returns `None` before the daemon has loaded
/// MiniLM (e.g. on cold start or when the model download failed).
pub fn global_embedder() -> Option<Arc<dyn Embedder>> {
    GLOBAL_EMBEDDER.get().cloned()
}

/// Deterministic fake embedder for unit and integration tests.
///
/// Produces a fixed-dim vector derived from a rolling hash of the input
/// bytes — different texts yield different vectors, and identical texts
/// yield identical vectors. Output is L2-normalized so cosine distance is
/// meaningful.
///
/// **Never use in production.** Quality is unrelated to any real embedding
/// space. This type is intentionally `pub` (not `cfg(test)`) so that
/// integration tests living in `crates/daemon/tests/*.rs` can construct one
/// without depending on a real model download. The cost of shipping the type
/// in the public API is ~50 lines of trivially auditable code.
pub struct FakeEmbedder {
    dim: usize,
}

impl FakeEmbedder {
    pub fn new(dim: usize) -> Self {
        Self { dim }
    }
}

impl Embedder for FakeEmbedder {
    fn dim(&self) -> usize {
        self.dim
    }

    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        Ok(texts
            .iter()
            .map(|t| {
                // Hash each byte into one of `dim` buckets, then normalize.
                let mut v = vec![0.0f32; self.dim];
                for (i, b) in t.bytes().enumerate() {
                    let idx = (i + b as usize) % self.dim;
                    v[idx] += b as f32;
                }
                // Normalize to unit length so cosine similarity is meaningful.
                let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
                if norm > 0.0 {
                    for x in v.iter_mut() {
                        *x /= norm;
                    }
                }
                v
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_embedder_produces_correct_dim() {
        let emb = FakeEmbedder::new(384);
        assert_eq!(emb.dim(), 384);
        let out = emb
            .embed(&[
                "hello".to_string(),
                "world".to_string(),
                "hello".to_string(),
            ])
            .unwrap();
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].len(), 384);
        // Identical inputs must yield identical outputs.
        assert_eq!(out[0], out[2]);
        // Different inputs must yield different outputs.
        assert_ne!(out[0], out[1]);
    }

    #[test]
    fn fake_embedder_outputs_are_normalized() {
        let emb = FakeEmbedder::new(384);
        let out = emb.embed(&["hello world".to_string()]).unwrap();
        let norm: f32 = out[0].iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5, "expected unit norm, got {norm}");
    }

    #[test]
    fn embed_error_display() {
        let e = EmbedError::DimensionMismatch {
            expected: 384,
            actual: 768,
        };
        assert!(format!("{e}").contains("384"));
        assert!(format!("{e}").contains("768"));
    }
}
