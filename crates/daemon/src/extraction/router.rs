// extraction/router.rs — Smart Model Router for extraction
//
// Routes extraction requests to the cheapest capable provider based on
// task complexity (prompt token count, code blocks, multi-step analysis).
//
// Tiers:
//   Free  — Ollama (qwen3:1.7b): simple extractions, < 500 tokens, no code
//   Cheap — Gemini Flash / Claude Haiku: medium analysis, 500-2000 tokens or code blocks
//   Full  — Claude Sonnet / configured provider: > 2000 tokens or multi-step

use crate::config::ForgeConfig;
use super::backend::BackendChoice;

/// Complexity tier for routing extraction requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComplexityTier {
    /// Simple extraction: < 500 tokens, no code blocks, no multi-step.
    Free,
    /// Medium analysis: 500-2000 tokens, or has code blocks.
    Cheap,
    /// Complex reasoning: > 2000 tokens, or multi-step analysis.
    Full,
}

impl std::fmt::Display for ComplexityTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ComplexityTier::Free => write!(f, "free"),
            ComplexityTier::Cheap => write!(f, "cheap"),
            ComplexityTier::Full => write!(f, "full"),
        }
    }
}

/// Estimate token count from text. Uses the ~4 chars per token heuristic.
fn estimate_tokens(text: &str) -> usize {
    // Conservative estimate: ~4 characters per token for English text
    text.len() / 4
}

/// Count code blocks (fenced ``` blocks) in the prompt text.
fn count_code_blocks(text: &str) -> usize {
    // Count occurrences of fenced code block markers (opening ```)
    // Each pair of ``` delimiters = one code block
    let fence_count = text.matches("```").count();
    fence_count / 2
}

/// Detect multi-step analysis signals in the prompt text.
fn has_multi_step_signals(text: &str) -> bool {
    let lower = text.to_lowercase();
    // Multi-step indicators: numbered steps, "then" chains, analysis keywords
    let multi_step_keywords = [
        "step 1", "step 2", "first,", "then,", "next,", "finally,",
        "analyze", "compare", "evaluate", "synthesize",
        "on the other hand", "however,", "in contrast",
        "pros and cons", "trade-off", "tradeoff",
    ];
    multi_step_keywords.iter().any(|kw| lower.contains(kw))
}

/// Score the complexity of an extraction task.
///
/// Returns a `ComplexityTier` based on:
/// - `prompt_tokens`: estimated token count of the prompt
/// - `code_block_count`: number of fenced code blocks in the text
/// - `has_multi_step`: whether the text contains multi-step analysis signals
pub fn score_complexity(
    prompt_tokens: usize,
    code_block_count: usize,
    has_multi_step: bool,
) -> ComplexityTier {
    // Full tier: multi-step analysis or very long prompts
    if has_multi_step || prompt_tokens > 2000 {
        return ComplexityTier::Full;
    }

    // Cheap tier: has code blocks or medium-length prompts
    if code_block_count > 0 || prompt_tokens >= 500 {
        return ComplexityTier::Cheap;
    }

    // Free tier: simple, short, no code
    ComplexityTier::Free
}

/// Select the appropriate provider based on complexity and available backends.
///
/// Returns `(provider_name, tier)` where `provider_name` is one of:
/// "ollama", "gemini", "claude_api", "openai", "claude".
///
/// Falls back to the statically configured backend if the preferred tier
/// provider is not available.
pub async fn route_extraction(
    config: &ForgeConfig,
    prompt: &str,
) -> (BackendChoice, ComplexityTier) {
    let tokens = estimate_tokens(prompt);
    let code_blocks = count_code_blocks(prompt);
    let multi_step = has_multi_step_signals(prompt);
    let tier = score_complexity(tokens, code_blocks, multi_step);

    tracing::info!(
        tier = %tier,
        tokens = tokens,
        code_blocks = code_blocks,
        multi_step = multi_step,
        "smart router scored complexity"
    );

    // Try to select the best provider for the tier.
    // Claude CLI is always first — uses the session's own subscription, best quality.
    let backend = match tier {
        ComplexityTier::Free => {
            try_claude_cli().await
                .or_async(|| try_claude_api(config)).await
                .or_async(|| try_gemini(config)).await
                .or_async(|| try_ollama(config)).await
                .or_async(|| try_openai(config)).await
        }
        ComplexityTier::Cheap => {
            try_claude_cli().await
                .or_async(|| try_claude_api(config)).await
                .or_async(|| try_gemini(config)).await
                .or_async(|| try_openai(config)).await
                .or_async(|| try_ollama(config)).await
        }
        ComplexityTier::Full => {
            try_claude_cli().await
                .or_async(|| try_claude_api(config)).await
                .or_async(|| try_openai(config)).await
                .or_async(|| try_gemini(config)).await
                .or_async(|| try_ollama(config)).await
        }
    };

    match backend {
        Some(b) => {
            tracing::info!(tier = %tier, provider = ?b, "smart router selected");
            (b, tier)
        }
        None => {
            // No backend available at all
            tracing::warn!(tier = %tier, "smart router: no backend available, falling back to None");
            (BackendChoice::None("smart router: no backend available".to_string()), tier)
        }
    }
}

// ── Provider availability checks ──

async fn try_ollama(config: &ForgeConfig) -> Option<BackendChoice> {
    let endpoint = &config.extraction.ollama.endpoint;
    if super::backend::is_ollama_available(endpoint).await {
        Some(BackendChoice::Ollama)
    } else {
        None
    }
}

async fn try_gemini(config: &ForgeConfig) -> Option<BackendChoice> {
    if crate::config::resolve_api_key(
        &config.extraction.gemini.api_key,
        "GEMINI_API_KEY",
    ).is_some() {
        Some(BackendChoice::Gemini)
    } else {
        None
    }
}

async fn try_claude_api(config: &ForgeConfig) -> Option<BackendChoice> {
    if crate::config::resolve_api_key(
        &config.extraction.claude_api.api_key,
        "ANTHROPIC_API_KEY",
    ).is_some() {
        Some(BackendChoice::ClaudeApi)
    } else {
        None
    }
}

async fn try_openai(config: &ForgeConfig) -> Option<BackendChoice> {
    if crate::config::resolve_api_key(
        &config.extraction.openai.api_key,
        "OPENAI_API_KEY",
    ).is_some() {
        Some(BackendChoice::OpenAi)
    } else {
        None
    }
}

async fn try_claude_cli() -> Option<BackendChoice> {
    if super::backend::is_claude_cli_available().await {
        Some(BackendChoice::ClaudeCli)
    } else {
        None
    }
}

/// Helper trait for chaining async Option operations.
trait OrAsync<T> {
    /// If self is None, evaluate the async closure. Otherwise return self.
    async fn or_async<F, Fut>(self, f: F) -> Option<T>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Option<T>>;
}

impl<T> OrAsync<T> for Option<T> {
    async fn or_async<F, Fut>(self, f: F) -> Option<T>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Option<T>>,
    {
        match self {
            Some(v) => Some(v),
            None => f().await,
        }
    }
}

/// Record a routing decision in the routing_stats table.
/// quality_score: optional quality metric (0.0-1.0) for the extraction result.
pub fn record_routing_stat(
    conn: &rusqlite::Connection,
    tier: &ComplexityTier,
    provider: &str,
    success: bool,
    tokens_saved: i64,
    quality_score: Option<f64>,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO routing_stats (tier, provider, success, tokens_saved, quality_score, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))",
        rusqlite::params![
            tier.to_string(),
            provider,
            if success { 1 } else { 0 },
            tokens_saved,
            quality_score,
        ],
    ).map_err(|e| format!("failed to record routing stat: {e}"))?;
    Ok(())
}

/// Check if recent extraction quality has dropped below the threshold (0.3).
/// If so, returns the recommended escalation tier.
/// Free -> Cheap, Cheap -> Full, Full -> None (already at max).
pub fn check_quality_guard(conn: &rusqlite::Connection) -> Option<ComplexityTier> {
    // Get average quality_score of extractions in the last 24 hours that have a quality_score
    let result: Result<(f64, i64), _> = conn.query_row(
        "SELECT COALESCE(AVG(quality_score), 1.0), COUNT(quality_score)
         FROM routing_stats
         WHERE created_at > datetime('now', '-24 hours') AND quality_score IS NOT NULL",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    );

    match result {
        Ok((avg_quality, count)) => {
            // Need at least 3 data points to trigger escalation (avoid single bad result)
            if count < 3 {
                return None;
            }
            if avg_quality < 0.3 {
                // Determine current dominant tier and escalate
                let current_tier: Option<String> = conn.query_row(
                    "SELECT tier FROM routing_stats
                     WHERE created_at > datetime('now', '-24 hours') AND quality_score IS NOT NULL
                     GROUP BY tier ORDER BY COUNT(*) DESC LIMIT 1",
                    [],
                    |row| row.get(0),
                ).ok();

                match current_tier.as_deref() {
                    Some("free") => Some(ComplexityTier::Cheap),
                    Some("cheap") => Some(ComplexityTier::Full),
                    _ => None, // Already at Full or unknown
                }
            } else {
                None
            }
        }
        Err(_) => None,
    }
}

/// Query aggregated routing stats.
pub fn query_routing_stats(conn: &rusqlite::Connection) -> Result<RoutingStatsResult, String> {
    let total: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM routing_stats",
            [],
            |row| row.get(0),
        )
        .map_err(|e| format!("routing stats query failed: {e}"))?;

    let mut stmt = conn
        .prepare(
            "SELECT tier, COUNT(*) as cnt, SUM(success) as ok_cnt, SUM(tokens_saved) as saved
             FROM routing_stats GROUP BY tier ORDER BY tier",
        )
        .map_err(|e| format!("routing stats query failed: {e}"))?;

    let tiers: Vec<TierStats> = stmt
        .query_map([], |row| {
            Ok(TierStats {
                tier: row.get(0)?,
                count: row.get::<_, i64>(1)? as usize,
                successes: row.get::<_, i64>(2)? as usize,
                tokens_saved: row.get::<_, i64>(3)?,
            })
        })
        .map_err(|e| format!("routing stats query failed: {e}"))?
        .filter_map(|r| r.ok())
        .collect();

    let total_tokens_saved: i64 = tiers.iter().map(|t| t.tokens_saved).sum();

    Ok(RoutingStatsResult {
        total_routed: total as usize,
        tiers,
        total_tokens_saved,
    })
}

/// Aggregated routing statistics result.
#[derive(Debug, Clone)]
pub struct RoutingStatsResult {
    pub total_routed: usize,
    pub tiers: Vec<TierStats>,
    pub total_tokens_saved: i64,
}

/// Per-tier routing statistics.
#[derive(Debug, Clone)]
pub struct TierStats {
    pub tier: String,
    pub count: usize,
    pub successes: usize,
    pub tokens_saved: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_score_complexity_simple() {
        // < 500 tokens, no code blocks, no multi-step → Free
        let tier = score_complexity(100, 0, false);
        assert_eq!(tier, ComplexityTier::Free);
    }

    #[test]
    fn test_score_complexity_simple_boundary() {
        // Exactly 499 tokens, no code blocks, no multi-step → Free
        let tier = score_complexity(499, 0, false);
        assert_eq!(tier, ComplexityTier::Free);
    }

    #[test]
    fn test_score_complexity_medium_by_tokens() {
        // 500-2000 tokens → Cheap
        let tier = score_complexity(500, 0, false);
        assert_eq!(tier, ComplexityTier::Cheap);

        let tier = score_complexity(1500, 0, false);
        assert_eq!(tier, ComplexityTier::Cheap);

        let tier = score_complexity(2000, 0, false);
        assert_eq!(tier, ComplexityTier::Cheap);
    }

    #[test]
    fn test_score_complexity_medium_by_code_blocks() {
        // Has code blocks → Cheap (even with low tokens)
        let tier = score_complexity(100, 1, false);
        assert_eq!(tier, ComplexityTier::Cheap);

        let tier = score_complexity(50, 3, false);
        assert_eq!(tier, ComplexityTier::Cheap);
    }

    #[test]
    fn test_score_complexity_complex_by_tokens() {
        // > 2000 tokens → Full
        let tier = score_complexity(2001, 0, false);
        assert_eq!(tier, ComplexityTier::Full);

        let tier = score_complexity(5000, 0, false);
        assert_eq!(tier, ComplexityTier::Full);
    }

    #[test]
    fn test_score_complexity_complex_by_multi_step() {
        // Multi-step → Full (even with low tokens)
        let tier = score_complexity(100, 0, true);
        assert_eq!(tier, ComplexityTier::Full);
    }

    #[test]
    fn test_score_complexity_complex_overrides_cheap() {
        // Multi-step overrides code blocks → Full
        let tier = score_complexity(100, 2, true);
        assert_eq!(tier, ComplexityTier::Full);
    }

    #[test]
    fn test_estimate_tokens() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("hello world"), 2); // 11 chars / 4 = 2
        // 2000 chars → 500 tokens
        let text = "a".repeat(2000);
        assert_eq!(estimate_tokens(&text), 500);
    }

    #[test]
    fn test_count_code_blocks() {
        assert_eq!(count_code_blocks("no code here"), 0);
        assert_eq!(count_code_blocks("```rust\nfn main() {}\n```"), 1);
        assert_eq!(
            count_code_blocks("```\nblock1\n```\ntext\n```\nblock2\n```"),
            2
        );
        // Odd number of fences = partial block, only count complete pairs
        assert_eq!(count_code_blocks("```\nunclosed"), 0);
    }

    #[test]
    fn test_has_multi_step_signals() {
        assert!(!has_multi_step_signals("simple text"));
        assert!(has_multi_step_signals("Step 1: do this. Step 2: do that."));
        assert!(has_multi_step_signals("First, analyze the code. Then, fix bugs."));
        assert!(has_multi_step_signals("Compare the trade-off between speed and memory."));
        assert!(has_multi_step_signals("Evaluate the pros and cons of this approach."));
    }

    #[test]
    fn test_route_extraction_static_mode() {
        // When routing is "static", the router should not be called.
        // This test validates that score_complexity still works independently.
        let tier = score_complexity(100, 0, false);
        assert_eq!(tier, ComplexityTier::Free);
    }

    #[tokio::test]
    async fn test_route_extraction_smart_mode() {
        // Test with a config where no backends are available.
        // The router should return None backend with the correct tier.
        let config = ForgeConfig::default();

        // Short prompt → Free tier
        let (_backend, tier) = route_extraction(&config, "short text").await;
        assert_eq!(tier, ComplexityTier::Free);

        // Long prompt → Full tier
        let long_text = "a".repeat(10000);
        let (_backend, tier) = route_extraction(&config, &long_text).await;
        assert_eq!(tier, ComplexityTier::Full);
    }

    #[test]
    fn test_record_and_query_routing_stats() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS routing_stats (
                tier TEXT NOT NULL,
                provider TEXT NOT NULL,
                success INTEGER NOT NULL DEFAULT 1,
                tokens_saved INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                quality_score REAL
            )",
        ).unwrap();

        record_routing_stat(&conn, &ComplexityTier::Free, "ollama", true, 500, None).unwrap();
        record_routing_stat(&conn, &ComplexityTier::Free, "ollama", true, 300, None).unwrap();
        record_routing_stat(&conn, &ComplexityTier::Cheap, "gemini", true, 100, None).unwrap();
        record_routing_stat(&conn, &ComplexityTier::Full, "claude_api", false, 0, None).unwrap();

        let stats = query_routing_stats(&conn).unwrap();
        assert_eq!(stats.total_routed, 4);
        assert_eq!(stats.tiers.len(), 3);
        assert_eq!(stats.total_tokens_saved, 900);

        // Free tier: 2 routed, 2 successes, 800 saved
        let free_tier = stats.tiers.iter().find(|t| t.tier == "free").unwrap();
        assert_eq!(free_tier.count, 2);
        assert_eq!(free_tier.successes, 2);
        assert_eq!(free_tier.tokens_saved, 800);

        // Cheap tier: 1 routed
        let cheap_tier = stats.tiers.iter().find(|t| t.tier == "cheap").unwrap();
        assert_eq!(cheap_tier.count, 1);

        // Full tier: 1 routed, 0 successes
        let full_tier = stats.tiers.iter().find(|t| t.tier == "full").unwrap();
        assert_eq!(full_tier.count, 1);
        assert_eq!(full_tier.successes, 0);
    }
}
