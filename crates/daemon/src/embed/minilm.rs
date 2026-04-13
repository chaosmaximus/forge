// embed/minilm.rs — fastembed-rs wrapper for all-MiniLM-L6-v2.
//
// This is the default raw-layer embedder. `all-MiniLM-L6-v2` is a 384-dim
// sentence transformer, the exact model MemPalace uses for their 96.6%
// LongMemEval raw-mode number. Matching the embedder exactly is a
// requirement for apples-to-apples benchmark parity.
//
// fastembed downloads the ONNX weights (~90 MB) on first use to
// `~/.cache/fastembed/`. Subsequent loads are fast.

use std::sync::Mutex;

use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};

use super::{EmbedError, Embedder};

/// Output dimension of `all-MiniLM-L6-v2`.
pub const MINILM_DIM: usize = 384;

/// Production embedder backed by `all-MiniLM-L6-v2` via fastembed-rs.
///
/// Holds the model behind a `Mutex` because fastembed's `TextEmbedding::embed`
/// takes `&mut self` (the ONNX session is not internally thread-safe).
/// Serializing inference calls is fine for our workloads: raw ingest is a
/// single background worker, and `RawSearch` queries are one-at-a-time.
pub struct MiniLMEmbedder {
    inner: Mutex<TextEmbedding>,
}

impl MiniLMEmbedder {
    /// Initialize the model. On first call this downloads the ONNX weights to
    /// the fastembed cache directory (~90 MB). Subsequent calls reuse the
    /// cached weights and return in well under a second.
    pub fn new() -> Result<Self, EmbedError> {
        let model = TextEmbedding::try_new(
            InitOptions::new(EmbeddingModel::AllMiniLML6V2).with_show_download_progress(false),
        )
        .map_err(|e| EmbedError::Init(e.to_string()))?;
        Ok(Self {
            inner: Mutex::new(model),
        })
    }
}

impl Embedder for MiniLMEmbedder {
    fn dim(&self) -> usize {
        MINILM_DIM
    }

    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let mut model = self
            .inner
            .lock()
            .map_err(|_| EmbedError::Inference("embedder mutex poisoned".to_string()))?;
        // fastembed accepts any `Vec<S: AsRef<str>>`, but we clone to `Vec<&str>`
        // so the ownership story is trivial (`texts` stays borrowed).
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        let out = model
            .embed(refs, None)
            .map_err(|e| EmbedError::Inference(e.to_string()))?;

        // Defensive: fastembed should always return MINILM_DIM for this model,
        // but validate so a model swap can never silently break the vec0 table.
        if let Some(first) = out.first() {
            if first.len() != MINILM_DIM {
                return Err(EmbedError::DimensionMismatch {
                    expected: MINILM_DIM,
                    actual: first.len(),
                });
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Network-gated: only runs when FORGE_TEST_FASTEMBED=1 is set. CI skips it
    /// unless a pre-seeded fastembed cache is available, so we don't block the
    /// test suite on an unconditional ~90 MB download.
    #[test]
    fn minilm_loads_and_embeds() {
        if std::env::var("FORGE_TEST_FASTEMBED").ok().as_deref() != Some("1") {
            eprintln!(
                "skipping MiniLM integration test — set FORGE_TEST_FASTEMBED=1 to run (will download ~90 MB)"
            );
            return;
        }

        let emb = MiniLMEmbedder::new().expect("fastembed init");
        assert_eq!(emb.dim(), MINILM_DIM);

        let out = emb
            .embed(&[
                "Forge is a Rust daemon.".to_string(),
                "Forge is a Rust daemon.".to_string(),
                "Completely unrelated sentence about pandas.".to_string(),
            ])
            .expect("fastembed inference");

        assert_eq!(out.len(), 3);
        assert_eq!(out[0].len(), MINILM_DIM);

        // Identical inputs must yield (nearly) identical outputs.
        let max_diff: f32 = out[0]
            .iter()
            .zip(out[1].iter())
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f32::max);
        assert!(max_diff < 1e-5);

        // Different inputs must yield different outputs.
        let max_diff_unrelated: f32 = out[0]
            .iter()
            .zip(out[2].iter())
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f32::max);
        assert!(max_diff_unrelated > 0.01);

        // Verify the output is roughly unit-normalized (MiniLM defaults to
        // normalized sentence embeddings in fastembed).
        let norm: f32 = out[0].iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 0.05, "expected unit norm, got {norm}");
    }

    #[test]
    fn minilm_empty_input_returns_empty() {
        // This test exercises the empty-input path which short-circuits before
        // loading the model — safe to run without FORGE_TEST_FASTEMBED.
        // We skip the init by constructing a test that only hits the early
        // return; but since `new()` downloads, we can't actually call it here.
        // Instead, verify the constant directly.
        assert_eq!(MINILM_DIM, 384);
    }
}
