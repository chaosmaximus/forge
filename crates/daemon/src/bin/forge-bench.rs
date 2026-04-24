// forge-bench — benchmark runner binary for the Forge raw layer.
//
// Embedded harness: spins up an in-memory SQLite per question, runs the
// chosen mode against a real `MiniLMEmbedder` (or a `FakeEmbedder` when
// `--fake-embedder` is set for offline smoke tests), and writes per-question
// JSONL plus a summary JSON to disk.
//
// Designed for benchmark parity with MemPalace's reference scripts — see
// docs/benchmarks/plan.md §5 for the harness contract.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use clap::{Parser, Subcommand};

use forge_daemon::bench::locomo::{
    load_samples, run_sample_extract, run_sample_raw, summarize as locomo_summarize,
    QaResult as LocomoQaResult,
};
use forge_daemon::bench::longmemeval::{
    load_entries, run_consolidate, run_extract, run_hybrid, run_raw, summarize, BenchMode,
    QuestionResult, RawStrategy,
};
use forge_daemon::bench::telemetry::{emit_bench_run_completed, BenchRunPayload, DimensionEntry};
use forge_daemon::embed::{minilm::MiniLMEmbedder, Embedder, FakeEmbedder};

#[derive(Parser, Debug)]
#[command(
    name = "forge-bench",
    version,
    about = "Reproducible memory-benchmark runner for the Forge raw layer."
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Run the LongMemEval benchmark.
    Longmemeval {
        /// Path to longmemeval_s_cleaned.json.
        path: PathBuf,
        /// Mode: raw | extract | consolidate | hybrid (raw and extract are implemented).
        #[arg(long, default_value = "raw")]
        mode: String,
        /// Limit to the first N questions (0 = full run).
        #[arg(long, default_value_t = 0)]
        limit: usize,
        /// Output directory for JSONL + summary JSON.
        #[arg(long, default_value = "bench_results")]
        output: PathBuf,
        /// Use the deterministic FakeEmbedder (offline; raw mode smoke tests only).
        #[arg(long, default_value_t = false)]
        fake_embedder: bool,
        /// Extract mode only: Gemini model to use for extraction. Default
        /// `"gemini-2.5-flash"` is the current free-tier Flash model.
        /// `gemini-2.0-flash` is deprecated for new users.
        #[arg(long, default_value = "gemini-2.5-flash")]
        extract_model: String,
        /// Extract mode only: max concurrent Gemini API extraction calls.
        /// Higher = faster but more pressure on rate limits.
        #[arg(long, default_value_t = 8)]
        extract_concurrency: usize,
        /// Raw mode sub-strategy: `knn` preserves the published KNN-only
        /// baseline; `hybrid` (default) uses KNN + FTS5 BM25 fused via RRF.
        #[arg(long, default_value = "hybrid")]
        raw_mode: String,
    },
    /// Run the Forge-Persist benchmark — spawn a daemon, issue a
    /// scripted seeded workload, SIGKILL mid-run, restart, and
    /// verify every HTTP-200-acked op survived. See
    /// `docs/benchmarks/forge-persist-design.md` §8 for the full
    /// flag contract and §6 for the scoring rubric.
    ForgePersist {
        /// Number of `Remember` ops in the workload.
        #[arg(long, default_value_t = 100)]
        memories: usize,
        /// Number of `RawIngest` ops.
        #[arg(long, default_value_t = 50)]
        chunks: usize,
        /// Number of `SessionSend` (FISP) ops.
        #[arg(long, default_value_t = 20)]
        fisp_messages: usize,
        /// ChaCha20 PRNG seed for the workload interleaver. Controls
        /// the shuffled order of ops but NOT their content, which is
        /// index-derived and always deterministic.
        #[arg(long, default_value_t = 42)]
        seed: u64,
        /// Fraction of total ops at which SIGKILL fires. 0.5 = kill
        /// after half the workload has been acked.
        #[arg(long, default_value_t = 0.5)]
        kill_after: f64,
        /// Output directory for `summary.json`, `repro.sh`, and the
        /// pre_kill/post_restart JSONL dumps.
        #[arg(long, default_value = "bench_results")]
        output: PathBuf,
        /// Path to the forge-daemon binary. Defaults to None; cycle
        /// (j)'s orchestrator falls back to locating the binary via
        /// `which forge-daemon` if this flag is omitted.
        #[arg(long)]
        daemon_bin: Option<PathBuf>,
        /// Wall-clock timeout for the daemon's HTTP Health response
        /// after spawn. If Health isn't reachable within this window,
        /// the harness reports `SpawnTimeout` and aborts the run.
        #[arg(long, default_value_t = 5000)]
        recovery_timeout_ms: u64,
        /// Post-restart catch-up window. After the second spawn,
        /// the harness waits this long before scoring so the
        /// async embedder worker can finish processing pre-kill
        /// memories that were acked but not yet embedded at kill time.
        #[arg(long, default_value_t = 10000)]
        worker_catchup_ms: u64,
        /// Per-request total timeout for HttpClient in milliseconds.
        /// The original 5 s default caused `NetworkError::TimedOut` on
        /// stress workloads with 250+ raw ingests. 30 s provides
        /// headroom without hiding real hangs.
        #[arg(long, default_value_t = 30000)]
        request_timeout_ms: u64,
    },
    /// Run the Forge-Context benchmark (proactive intelligence precision).
    /// See docs/benchmarks/forge-context-design.md.
    ForgeContext {
        /// ChaCha20 seed for deterministic dataset generation.
        #[arg(long, default_value_t = 42)]
        seed: u64,
        /// Output directory for summary.json.
        #[arg(long, default_value = "bench_results_context")]
        output: PathBuf,
    },
    /// Run the Forge-Consolidation benchmark (memory consolidation quality).
    /// See docs/benchmarks/forge-consolidation-design.md.
    ForgeConsolidation {
        /// ChaCha20 seed for deterministic dataset generation.
        #[arg(long, default_value_t = 42)]
        seed: u64,
        /// Output directory for summary.json, baseline.json, post.json, repro.sh.
        #[arg(long, default_value = "bench_results_consolidation")]
        output: PathBuf,
        /// Expected recall-improvement delta for pass/fail gate.
        /// Omit on first calibration run — observed delta will be printed.
        #[arg(long)]
        expected_recall_delta: Option<f64>,
    },
    /// Run the Forge-Identity benchmark (memory + identity + skill inference).
    /// See docs/benchmarks/forge-identity-master-design.md.
    ForgeIdentity {
        /// ChaCha20 seed for deterministic dataset generation.
        #[arg(long, default_value_t = 42)]
        seed: u64,
        /// Output directory for summary.json.
        #[arg(long, default_value = "bench_results_forge_identity")]
        output: PathBuf,
        /// Expected composite score for pass/fail gate.
        /// Omit for first calibration run; observed composite will be printed.
        #[arg(long)]
        expected_composite: Option<f64>,
    },
    /// Run the LoCoMo benchmark.
    Locomo {
        /// Path to locomo10.json (from snap-research/locomo).
        path: PathBuf,
        /// Mode: raw | extract (raw uses MiniLM KNN, extract uses Gemini LLM).
        #[arg(long, default_value = "raw")]
        mode: String,
        /// Limit to the first N samples (0 = full run; LoCoMo has 10).
        #[arg(long, default_value_t = 0)]
        limit: usize,
        /// Output directory for JSONL + summary JSON.
        #[arg(long, default_value = "bench_results")]
        output: PathBuf,
        /// Use the deterministic FakeEmbedder (offline; smoke tests only).
        #[arg(long, default_value_t = false)]
        fake_embedder: bool,
        /// Extract mode only: Gemini model to use for extraction.
        #[arg(long, default_value = "gemini-2.5-flash")]
        extract_model: String,
        /// Extract mode only: max concurrent Gemini API extraction calls.
        #[arg(long, default_value_t = 8)]
        extract_concurrency: usize,
        /// Raw mode sub-strategy: `knn` preserves the published KNN-only
        /// baseline; `hybrid` (default) uses KNN + FTS5 BM25 fused via RRF.
        #[arg(long, default_value = "hybrid")]
        raw_mode: String,
    },
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_target(false)
        .compact()
        .init();

    let cli = Cli::parse();
    let outcome = match cli.command {
        Commands::Longmemeval {
            path,
            mode,
            limit,
            output,
            fake_embedder,
            extract_model,
            extract_concurrency,
            raw_mode,
        } => {
            run_longmemeval(
                path,
                mode,
                limit,
                output,
                fake_embedder,
                extract_model,
                extract_concurrency,
                raw_mode,
            )
            .await
        }
        Commands::Locomo {
            path,
            mode,
            limit,
            output,
            fake_embedder,
            extract_model,
            extract_concurrency,
            raw_mode,
        } => {
            run_locomo(
                path,
                mode,
                limit,
                output,
                fake_embedder,
                extract_model,
                extract_concurrency,
                raw_mode,
            )
            .await
        }
        Commands::ForgeContext { seed, output } => run_forge_context(seed, output),
        Commands::ForgeIdentity {
            seed,
            output,
            expected_composite,
        } => run_forge_identity(seed, output, expected_composite).await,
        Commands::ForgeConsolidation {
            seed,
            output,
            expected_recall_delta,
        } => run_forge_consolidation(seed, output, expected_recall_delta),
        Commands::ForgePersist {
            memories,
            chunks,
            fisp_messages,
            seed,
            kill_after,
            output,
            daemon_bin,
            recovery_timeout_ms,
            worker_catchup_ms,
            request_timeout_ms,
        } => run_forge_persist(
            memories,
            chunks,
            fisp_messages,
            seed,
            kill_after,
            output,
            daemon_bin,
            recovery_timeout_ms,
            worker_catchup_ms,
            request_timeout_ms,
        ),
    };
    if let Err(e) = outcome {
        eprintln!("forge-bench: {e}");
        std::process::exit(1);
    }
}

// Parameter list mirrors the clap `Commands::Longmemeval` variant fields
// one-for-one, so bundling them into a struct would only add an extra
// destructure step in `main`. The clippy lint is suppressed here rather
// than refactored because the coupling is intentional.
#[allow(clippy::too_many_arguments)]
async fn run_longmemeval(
    path: PathBuf,
    mode_str: String,
    limit: usize,
    output: PathBuf,
    fake_embedder: bool,
    extract_model: String,
    extract_concurrency: usize,
    raw_mode_str: String,
) -> Result<(), String> {
    let mode =
        BenchMode::parse(&mode_str).map_err(|e| format!("invalid --mode '{mode_str}': {e}"))?;
    let raw_strategy = RawStrategy::parse(&raw_mode_str)
        .map_err(|e| format!("invalid --raw-mode '{raw_mode_str}': {e}"))?;
    let needs_embedder = matches!(mode, BenchMode::Raw | BenchMode::Hybrid);
    let needs_api_key = matches!(
        mode,
        BenchMode::Extract | BenchMode::Consolidate | BenchMode::Hybrid
    );
    if mode == BenchMode::Extract && fake_embedder {
        return Err(
            "extract mode does not use the embedder; --fake-embedder is incompatible".to_string(),
        );
    }

    eprintln!("[forge-bench] loading entries from {}", path.display());
    let mut entries = load_entries(&path).map_err(|e| e.to_string())?;
    if limit > 0 && limit < entries.len() {
        entries.truncate(limit);
    }
    eprintln!("[forge-bench] loaded {} questions", entries.len());

    // Embedder is only needed for raw / hybrid modes — skip the load entirely
    // in extract mode so the bench stays fast and offline-capable.
    let embedder: Option<Arc<dyn Embedder>> = if needs_embedder {
        eprintln!(
            "[forge-bench] initializing embedder ({})",
            if fake_embedder {
                "FakeEmbedder"
            } else {
                "MiniLMEmbedder"
            }
        );
        let e: Arc<dyn Embedder> = if fake_embedder {
            Arc::new(FakeEmbedder::new(384))
        } else {
            Arc::new(MiniLMEmbedder::new().map_err(|e| format!("MiniLMEmbedder::new: {e}"))?)
        };
        eprintln!("[forge-bench] embedder ready (dim={})", e.dim());
        Some(e)
    } else {
        None
    };

    if needs_api_key {
        eprintln!(
            "[forge-bench] {} mode — using Gemini `{extract_model}` via HTTP API (concurrency={extract_concurrency})",
            mode.as_str()
        );
    }

    // Read API key up front so we fail before running anything if it's missing.
    let gemini_api_key = if needs_api_key {
        std::env::var("GEMINI_API_KEY").map_err(|_| {
            format!(
                "{} mode requires GEMINI_API_KEY environment variable — get a key from \
                 https://aistudio.google.com/apikey and export it before running",
                mode.as_str()
            )
        })?
    } else {
        String::new()
    };

    std::fs::create_dir_all(&output).map_err(|e| format!("mkdir {}: {e}", output.display()))?;
    let timestamp = unix_secs_string();
    let run_dir = output.join(format!("longmemeval_{}_{}", mode.as_str(), timestamp));
    std::fs::create_dir_all(&run_dir).map_err(|e| format!("mkdir {}: {e}", run_dir.display()))?;
    let jsonl_path = run_dir.join("results.jsonl");
    let summary_path = run_dir.join("summary.json");
    let repro_path = run_dir.join("repro.sh");

    let mut jsonl_writer = std::io::BufWriter::new(
        std::fs::File::create(&jsonl_path)
            .map_err(|e| format!("create {}: {e}", jsonl_path.display()))?,
    );
    use std::io::Write as _;

    let started = Instant::now();
    let mut results: Vec<QuestionResult> = Vec::with_capacity(entries.len());
    for (idx, entry) in entries.iter().enumerate() {
        let result = match mode {
            BenchMode::Raw => {
                let emb = embedder.as_ref().ok_or("raw mode requires an embedder")?;
                run_raw(entry, emb, raw_strategy).map_err(|e| e.to_string())?
            }
            BenchMode::Extract => {
                run_extract(entry, &gemini_api_key, &extract_model, extract_concurrency)
                    .await
                    .map_err(|e| e.to_string())?
            }
            BenchMode::Consolidate => {
                run_consolidate(entry, &gemini_api_key, &extract_model, extract_concurrency)
                    .await
                    .map_err(|e| e.to_string())?
            }
            BenchMode::Hybrid => {
                let emb = embedder
                    .as_ref()
                    .ok_or("hybrid mode requires an embedder")?;
                run_hybrid(
                    entry,
                    emb,
                    &gemini_api_key,
                    &extract_model,
                    extract_concurrency,
                )
                .await
                .map_err(|e| e.to_string())?
            }
        };

        let line = serde_json::to_string(&result).map_err(|e| e.to_string())?;
        writeln!(jsonl_writer, "{line}").map_err(|e| e.to_string())?;
        results.push(result);

        // LLM-using modes are much slower per question — print every
        // question instead of every 10.
        let stride = if needs_api_key || entries.len() < 50 {
            1
        } else {
            10
        };
        if (idx + 1) % stride == 0 || idx + 1 == entries.len() {
            let mean_so_far: f64 =
                results.iter().map(|r| r.recall_at_5).sum::<f64>() / results.len() as f64;
            eprintln!(
                "[forge-bench] {}/{}  R@5 so far: {:.3}  (last: {:.1}s)",
                idx + 1,
                entries.len(),
                mean_so_far,
                results
                    .last()
                    .map(|r| r.elapsed_ms as f64 / 1000.0)
                    .unwrap_or(0.0)
            );
        }
    }
    jsonl_writer.flush().map_err(|e| e.to_string())?;
    drop(jsonl_writer);

    let summary = summarize(&results, mode);
    let summary_json = serde_json::to_string_pretty(&summary).map_err(|e| e.to_string())?;
    std::fs::write(&summary_path, &summary_json)
        .map_err(|e| format!("write {}: {e}", summary_path.display()))?;

    let repro = format!(
        "#!/usr/bin/env bash\n# Reproduce this benchmark run.\nset -euo pipefail\ncd \"$(git rev-parse --show-toplevel 2>/dev/null || pwd)\"\ncargo run --release --bin forge-bench -- longmemeval {} --mode {} --raw-mode {} --limit {} --output {}\n",
        path.display(),
        mode.as_str(),
        raw_strategy.as_str(),
        limit,
        output.display(),
    );
    std::fs::write(&repro_path, &repro)
        .map_err(|e| format!("write {}: {e}", repro_path.display()))?;

    let elapsed = started.elapsed();

    println!();
    println!("=== forge-bench: longmemeval ({mode_str}) ===");
    println!("Questions:           {}", summary.n_questions);
    println!("Mean Recall@5:       {:.4}", summary.mean_recall_at_5);
    println!("Mean Recall@10:      {:.4}", summary.mean_recall_at_10);
    println!("Mean Recall_all@10:  {:.4}", summary.mean_recall_all_at_10);
    println!("Mean NDCG@10:        {:.4}", summary.mean_ndcg_at_10);
    println!("Total elapsed:       {:.2}s", elapsed.as_secs_f64());
    if !summary.by_type_recall_at_5.is_empty() {
        println!();
        println!("Per question_type (R@5):");
        for (k, v) in &summary.by_type_recall_at_5 {
            println!("  {k:30}  {v:.4}");
        }
    }
    println!();
    println!("Results: {}", run_dir.display());
    println!("  results.jsonl   {}", jsonl_path.display());
    println!("  summary.json    {}", summary_path.display());
    println!("  repro.sh        {}", repro_path.display());

    let bench_specific = serde_json::to_value(&summary).unwrap_or(serde_json::Value::Null);
    emit_bench_telemetry(
        "longmemeval",
        BenchRunPayload {
            bench_name: format!("longmemeval-{}", mode.as_str()),
            seed: 0,
            composite: summary.mean_recall_at_5,
            pass: true,
            dimensions: Vec::new(),
            dimension_scores: HashMap::new(),
            bench_specific_stats: bench_specific,
            wall_duration_ms: elapsed.as_millis() as u64,
            result_count: 0,
        },
    );

    Ok(())
}

/// Emit one `bench_run_completed` row into `${FORGE_DIR}/forge.db`.
/// Telemetry failure is logged but never fails the bench — the bench
/// result on disk is still authoritative.
fn emit_bench_telemetry(name: &str, payload: BenchRunPayload) {
    if let Err(e) = emit_bench_run_completed(&payload) {
        tracing::warn!(bench = name, error = %e, "bench telemetry emit failed");
    }
}

fn unix_secs_string() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}

fn run_forge_context(seed: u64, output: PathBuf) -> Result<(), String> {
    use forge_daemon::bench::forge_context::{run, ContextConfig};

    eprintln!("=== forge-bench: forge-context ===");
    eprintln!("[forge-context] seed={seed}");
    eprintln!("[forge-context] output = {}", output.display());

    let config = ContextConfig {
        seed,
        output_dir: Some(output),
    };

    let started = Instant::now();
    match run(config) {
        Ok(score) => {
            eprintln!("[forge-context] === results ===");
            eprintln!(
                "[forge-context] context_assembly_f1={:.4}",
                score.context_assembly_f1
            );
            eprintln!("[forge-context] guardrails_f1={:.4}", score.guardrails_f1);
            eprintln!("[forge-context] completion_f1={:.4}", score.completion_f1);
            eprintln!(
                "[forge-context] layer_recall_f1={:.4}",
                score.layer_recall_f1
            );
            eprintln!(
                "[forge-context] tool_filter_accuracy={:.4}",
                score.tool_filter_accuracy
            );
            eprintln!("[forge-context] composite={:.4}", score.composite);
            let pass = score.pass;
            if pass {
                eprintln!("[forge-context] PASS");
            } else {
                eprintln!("[forge-context] FAIL");
            }

            // Telemetry: 4 dimensions match compute_composite weights.
            let dim_specs: [(&str, f64, f64); 4] = [
                ("context_assembly", score.context_assembly_f1, 0.0),
                ("guardrails", score.guardrails_f1, 0.0),
                ("completion", score.completion_f1, 0.0),
                ("layer_recall", score.layer_recall_f1, 0.0),
            ];
            let dimensions: Vec<DimensionEntry> = dim_specs
                .iter()
                .map(|(name, s, m)| DimensionEntry {
                    name: (*name).to_string(),
                    score: *s,
                    min: *m,
                    pass: *s >= *m,
                })
                .collect();
            let mut dimension_scores = HashMap::new();
            for d in &dimensions {
                dimension_scores.insert(d.name.clone(), d.score);
            }
            let bench_specific = serde_json::json!({
                "tool_filter_accuracy": score.tool_filter_accuracy,
                "total_queries": score.total_queries,
            });
            emit_bench_telemetry(
                "forge-context",
                BenchRunPayload {
                    bench_name: "forge-context".to_string(),
                    seed,
                    composite: score.composite,
                    pass,
                    dimensions: dimensions.clone(),
                    dimension_scores,
                    bench_specific_stats: bench_specific,
                    wall_duration_ms: started.elapsed().as_millis() as u64,
                    result_count: dimensions.len() as u64,
                },
            );

            if pass {
                Ok(())
            } else {
                Err("forge-context FAIL: composite below threshold".to_string())
            }
        }
        Err(e) => {
            eprintln!("[forge-context] ERROR: {e}");
            Err(format!("forge-context error: {e}"))
        }
    }
}

fn run_forge_consolidation(
    seed: u64,
    output: PathBuf,
    expected_recall_delta: Option<f64>,
) -> Result<(), String> {
    use forge_daemon::bench::forge_consolidation::{run, ConsolidationBenchConfig};

    eprintln!("=== forge-bench: forge-consolidation ===");
    eprintln!("[forge-consolidation] seed={seed}");
    eprintln!("[forge-consolidation] output = {}", output.display());
    if let Some(d) = expected_recall_delta {
        eprintln!("[forge-consolidation] expected_recall_delta={d}");
    }

    let config = ConsolidationBenchConfig {
        seed,
        output_dir: output,
        expected_recall_delta,
    };

    let started = Instant::now();
    match run(config) {
        Ok(score) => {
            eprintln!("[forge-consolidation] === results ===");
            eprintln!("[forge-consolidation] composite={:.4}", score.composite);
            eprintln!(
                "[forge-consolidation] recall_delta={:.4}",
                score.recall_delta
            );
            for (name, d) in &score.dimensions {
                eprintln!("[forge-consolidation] {name}={:.4}", d.f1);
            }
            for failure in &score.infrastructure_failures {
                eprintln!("[forge-consolidation] INFRA_FAIL: {failure}");
            }
            let pass = score.pass;
            if pass {
                eprintln!("[forge-consolidation] PASS");
            } else {
                eprintln!("[forge-consolidation] FAIL");
            }

            // Telemetry: 5 dimensions covering dedup / contradictions /
            // reweave / lifecycle / recall_improvement, each as the
            // dimension-keyed F1 from the score's HashMap.
            let dim_names = [
                "dedup",
                "contradictions",
                "reweave",
                "lifecycle",
                "recall_improvement",
            ];
            let dimensions: Vec<DimensionEntry> = dim_names
                .iter()
                .map(|name| {
                    let f1 = score.dimensions.get(*name).map(|d| d.f1).unwrap_or(0.0);
                    DimensionEntry {
                        name: (*name).to_string(),
                        score: f1,
                        min: 0.0,
                        pass: f1 >= 0.0,
                    }
                })
                .collect();
            let mut dimension_scores = HashMap::new();
            for d in &dimensions {
                dimension_scores.insert(d.name.clone(), d.score);
            }
            let bench_specific = serde_json::to_value(&score).unwrap_or(serde_json::Value::Null);
            emit_bench_telemetry(
                "forge-consolidation",
                BenchRunPayload {
                    bench_name: "forge-consolidation".to_string(),
                    seed,
                    composite: score.composite,
                    pass,
                    dimensions: dimensions.clone(),
                    dimension_scores,
                    bench_specific_stats: bench_specific,
                    wall_duration_ms: started.elapsed().as_millis() as u64,
                    result_count: dimensions.len() as u64,
                },
            );

            if pass {
                Ok(())
            } else {
                Err("forge-consolidation FAIL: composite below threshold or infrastructure failures".to_string())
            }
        }
        Err(e) => {
            eprintln!("[forge-consolidation] ERROR: {e}");
            Err(format!("forge-consolidation error: {e}"))
        }
    }
}

async fn run_forge_identity(
    seed: u64,
    output: PathBuf,
    expected_composite: Option<f64>,
) -> Result<(), String> {
    use forge_daemon::bench::forge_identity::{run_bench, BenchConfig};
    eprintln!("[forge-identity] starting seed={seed}");
    let cfg = BenchConfig {
        seed,
        output_dir: output,
        expected_composite,
    };
    let score = run_bench(cfg).await?;
    eprintln!("[forge-identity] === results ===");
    eprintln!("[forge-identity] composite={:.4}", score.composite);
    eprintln!("[forge-identity] pass={}", score.pass);
    for d in &score.dimensions {
        eprintln!(
            "[forge-identity] {} = {:.4} (min {:.2}, pass={})",
            d.name, d.score, d.min, d.pass
        );
    }
    eprintln!(
        "[forge-identity] wall_duration_ms={}",
        score.wall_duration_ms
    );
    eprintln!(
        "[forge-identity] {}",
        if score.pass { "PASS" } else { "FAIL" }
    );

    // Telemetry: 6 dimensions match the master v6 fixed shape.
    let dimensions: Vec<DimensionEntry> = score
        .dimensions
        .iter()
        .map(|d| DimensionEntry {
            name: d.name.clone(),
            score: d.score,
            min: d.min,
            pass: d.pass,
        })
        .collect();
    let mut dimension_scores = HashMap::new();
    for d in &dimensions {
        dimension_scores.insert(d.name.clone(), d.score);
    }
    let infra_passed = score
        .infrastructure_checks
        .iter()
        .filter(|c| c.passed)
        .count();
    let bench_specific = serde_json::json!({
        "infrastructure_checks_passed": infra_passed,
        "infrastructure_checks_total": score.infrastructure_checks.len(),
        "wall_duration_ms": score.wall_duration_ms,
    });
    emit_bench_telemetry(
        "forge-identity",
        BenchRunPayload {
            bench_name: "forge-identity".to_string(),
            seed,
            composite: score.composite,
            pass: score.pass,
            dimensions: dimensions.clone(),
            dimension_scores,
            bench_specific_stats: bench_specific,
            wall_duration_ms: score.wall_duration_ms,
            result_count: dimensions.len() as u64,
        },
    );

    if let Some(expected) = expected_composite {
        if (score.composite - expected).abs() > 0.05 {
            return Err(format!(
                "composite {:.4} drifted from expected {:.4} by > 0.05",
                score.composite, expected
            ));
        }
    }
    // Match forge-context / forge-consolidation: non-pass = process exit 1,
    // so CI gates can rely on the binary's exit code without parsing
    // summary.json. Exit code matches `score.pass`.
    if score.pass {
        Ok(())
    } else {
        Err("forge-identity FAIL: composite below threshold or per-dim minimum".to_string())
    }
}

// Parameter list mirrors the clap `Commands::ForgePersist` variant
// fields one-for-one, same rationale as `run_longmemeval` above —
// bundling would add a destructure step in `main` without simplifying
// the call site.
//
// **Cycle (j2.2):** replaces the cycle (i1) stub with the real
// orchestrator dispatch. Resolves `daemon_bin` (falling back to a
// sibling `forge-daemon` binary in this binary's parent dir if
// `--daemon-bin` is omitted), builds a `PersistConfig`, calls
// `bench::forge_persist::run`, and prints a structured verdict.
// Exit code 0 if `summary.passed` (production score_run thresholds),
// 1 otherwise.
#[allow(clippy::too_many_arguments)]
fn run_forge_persist(
    memories: usize,
    chunks: usize,
    fisp_messages: usize,
    seed: u64,
    kill_after: f64,
    output: PathBuf,
    daemon_bin: Option<PathBuf>,
    recovery_timeout_ms: u64,
    worker_catchup_ms: u64,
    request_timeout_ms: u64,
) -> Result<(), String> {
    use forge_daemon::bench::forge_persist::{run, PersistConfig};
    use std::time::Duration;

    // Resolve daemon_bin — if --daemon-bin is omitted, default to a
    // sibling `forge-daemon` next to this binary. forge-bench and
    // forge-daemon live in the same target/{profile}/ directory by
    // construction, so this is the right local fallback.
    let daemon_bin = match daemon_bin {
        Some(p) => p,
        None => {
            let self_exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
            let parent = self_exe
                .parent()
                .ok_or_else(|| "current_exe has no parent directory".to_string())?;
            parent.join("forge-daemon")
        }
    };
    if !daemon_bin.exists() {
        return Err(format!(
            "forge-persist: daemon binary not found at {} — pass --daemon-bin <path> or build forge-daemon first",
            daemon_bin.display()
        ));
    }

    eprintln!("=== forge-bench: forge-persist ===");
    eprintln!("[forge-persist] daemon_bin = {}", daemon_bin.display());
    eprintln!(
        "[forge-persist] workload: memories={memories} chunks={chunks} fisp_messages={fisp_messages} seed={seed}"
    );
    eprintln!(
        "[forge-persist] kill_after={kill_after} recovery_timeout_ms={recovery_timeout_ms} worker_catchup_ms={worker_catchup_ms}"
    );
    eprintln!("[forge-persist] output = {}", output.display());

    let config = PersistConfig {
        daemon_bin,
        memories,
        chunks,
        fisp_messages,
        seed,
        kill_after,
        recovery_timeout: Duration::from_millis(recovery_timeout_ms),
        worker_catchup: Duration::from_millis(worker_catchup_ms),
        output_dir: Some(output),
        request_timeout: Duration::from_millis(request_timeout_ms),
    };

    let started = Instant::now();
    let summary = run(config).map_err(|e| format!("forge-persist run failed: {e:?}"))?;

    eprintln!("[forge-persist] === verdict ===");
    eprintln!(
        "[forge-persist] total_ops={} acked_pre_kill={} recovered={} matched={}",
        summary.total_ops, summary.acked_pre_kill, summary.recovered, summary.matched
    );
    eprintln!(
        "[forge-persist] recovery_rate={:.4} consistency_rate={:.4} recovery_time_ms={}",
        summary.recovery_rate, summary.consistency_rate, summary.recovery_time_ms
    );
    eprintln!(
        "[forge-persist] wall_time_ms={} daemon_version={}",
        summary.wall_time_ms, summary.daemon_version
    );

    let bench_specific = serde_json::to_value(&summary).unwrap_or(serde_json::Value::Null);
    emit_bench_telemetry(
        "forge-persist",
        BenchRunPayload {
            bench_name: "forge-persist".to_string(),
            seed,
            composite: summary.recovery_rate,
            pass: summary.passed,
            dimensions: Vec::new(),
            dimension_scores: HashMap::new(),
            bench_specific_stats: bench_specific,
            wall_duration_ms: started.elapsed().as_millis() as u64,
            result_count: 0,
        },
    );

    if summary.passed {
        eprintln!("[forge-persist] PASS");
        Ok(())
    } else {
        // Surface WHICH metric failed so the user can act on it.
        // Threshold gating mirrors `score_run` in bench::forge_persist.
        let mut reasons = Vec::new();
        if summary.total_ops == 0 {
            reasons.push("zero-op workload".to_string());
        }
        if summary.recovery_rate < forge_daemon::bench::forge_persist::RECOVERY_RATE_THRESHOLD {
            reasons.push(format!(
                "recovery_rate {:.4} < {}",
                summary.recovery_rate,
                forge_daemon::bench::forge_persist::RECOVERY_RATE_THRESHOLD
            ));
        }
        if summary.consistency_rate < forge_daemon::bench::forge_persist::CONSISTENCY_RATE_THRESHOLD
        {
            reasons.push(format!(
                "consistency_rate {:.4} < {}",
                summary.consistency_rate,
                forge_daemon::bench::forge_persist::CONSISTENCY_RATE_THRESHOLD
            ));
        }
        if summary.recovery_time_ms
            >= forge_daemon::bench::forge_persist::RECOVERY_TIME_MS_THRESHOLD
        {
            reasons.push(format!(
                "recovery_time_ms {} >= {}",
                summary.recovery_time_ms,
                forge_daemon::bench::forge_persist::RECOVERY_TIME_MS_THRESHOLD
            ));
        }
        Err(format!("forge-persist FAIL: {}", reasons.join("; ")))
    }
}

// Parameter list mirrors the clap `Commands::Locomo` variant fields
// one-for-one — see `run_longmemeval` comment above for why this lint
// is suppressed rather than refactored.
#[allow(clippy::too_many_arguments)]
async fn run_locomo(
    path: PathBuf,
    mode_str: String,
    limit: usize,
    output: PathBuf,
    fake_embedder: bool,
    extract_model: String,
    extract_concurrency: usize,
    raw_mode_str: String,
) -> Result<(), String> {
    let mode = match mode_str.as_str() {
        "raw" => "raw",
        "extract" => "extract",
        other => {
            return Err(format!(
                "invalid --mode '{other}' (supported: raw | extract)"
            ));
        }
    };
    let raw_strategy = RawStrategy::parse(&raw_mode_str)
        .map_err(|e| format!("invalid --raw-mode '{raw_mode_str}': {e}"))?;
    eprintln!("[forge-bench] loading samples from {}", path.display());
    let mut samples = load_samples(&path).map_err(|e| e.to_string())?;
    if limit > 0 && limit < samples.len() {
        samples.truncate(limit);
    }
    let total_qa: usize = samples.iter().map(|s| s.qa.len()).sum();
    eprintln!(
        "[forge-bench] loaded {} samples ({} QA pairs)",
        samples.len(),
        total_qa
    );

    let embedder: Option<Arc<dyn Embedder>> = if mode == "raw" {
        eprintln!(
            "[forge-bench] initializing embedder ({})",
            if fake_embedder {
                "FakeEmbedder"
            } else {
                "MiniLMEmbedder"
            }
        );
        let e: Arc<dyn Embedder> = if fake_embedder {
            Arc::new(FakeEmbedder::new(384))
        } else {
            Arc::new(MiniLMEmbedder::new().map_err(|e| format!("MiniLMEmbedder::new: {e}"))?)
        };
        eprintln!("[forge-bench] embedder ready (dim={})", e.dim());
        Some(e)
    } else {
        None
    };

    let gemini_api_key = if mode == "extract" {
        eprintln!(
            "[forge-bench] extract mode — using Gemini `{extract_model}` via HTTP API (concurrency={extract_concurrency})"
        );
        std::env::var("GEMINI_API_KEY")
            .map_err(|_| "extract mode requires GEMINI_API_KEY environment variable".to_string())?
    } else {
        String::new()
    };

    std::fs::create_dir_all(&output).map_err(|e| format!("mkdir {}: {e}", output.display()))?;
    let timestamp = unix_secs_string();
    let run_dir = output.join(format!("locomo_{mode}_{timestamp}"));
    std::fs::create_dir_all(&run_dir).map_err(|e| format!("mkdir {}: {e}", run_dir.display()))?;
    let jsonl_path = run_dir.join("results.jsonl");
    let summary_path = run_dir.join("summary.json");
    let repro_path = run_dir.join("repro.sh");

    let mut jsonl_writer = std::io::BufWriter::new(
        std::fs::File::create(&jsonl_path)
            .map_err(|e| format!("create {}: {e}", jsonl_path.display()))?,
    );
    use std::io::Write as _;

    let started = Instant::now();
    let mut all_results: Vec<LocomoQaResult> = Vec::with_capacity(total_qa);
    for (idx, sample) in samples.iter().enumerate() {
        let sample_started = Instant::now();
        let results = match mode {
            "raw" => {
                let emb = embedder.as_ref().ok_or("raw mode requires an embedder")?;
                run_sample_raw(sample, emb, raw_strategy).map_err(|e| e.to_string())?
            }
            "extract" => {
                run_sample_extract(sample, &gemini_api_key, &extract_model, extract_concurrency)
                    .await
                    .map_err(|e| e.to_string())?
            }
            _ => unreachable!(),
        };
        for result in &results {
            let line = serde_json::to_string(result).map_err(|e| e.to_string())?;
            writeln!(jsonl_writer, "{line}").map_err(|e| e.to_string())?;
        }
        all_results.extend(results);

        let mean_so_far: f64 = if !all_results.is_empty() {
            all_results.iter().map(|r| r.recall_at_10).sum::<f64>() / all_results.len() as f64
        } else {
            0.0
        };
        eprintln!(
            "[forge-bench] sample {}/{} ({}) — {} QAs — R@10 so far: {:.3} — {:.1}s",
            idx + 1,
            samples.len(),
            sample.sample_id,
            all_results.len(),
            mean_so_far,
            sample_started.elapsed().as_secs_f64()
        );
    }
    jsonl_writer.flush().map_err(|e| e.to_string())?;
    drop(jsonl_writer);

    let summary = locomo_summarize(&all_results);
    let summary_json = serde_json::to_string_pretty(&summary).map_err(|e| e.to_string())?;
    std::fs::write(&summary_path, &summary_json)
        .map_err(|e| format!("write {}: {e}", summary_path.display()))?;

    let repro = format!(
        "#!/usr/bin/env bash\n# Reproduce this LoCoMo benchmark run.\nset -euo pipefail\ncd \"$(git rev-parse --show-toplevel 2>/dev/null || pwd)\"\ncargo run --release --bin forge-bench -- locomo {} --mode {} --raw-mode {} --limit {} --output {}\n",
        path.display(),
        mode,
        raw_strategy.as_str(),
        limit,
        output.display(),
    );
    std::fs::write(&repro_path, &repro)
        .map_err(|e| format!("write {}: {e}", repro_path.display()))?;

    let elapsed = started.elapsed();

    println!();
    println!("=== forge-bench: locomo ({mode}) ===");
    println!("Samples:             {}", samples.len());
    println!("QA pairs:            {}", summary.n_questions);
    println!("Mean Recall@5:       {:.4}", summary.mean_recall_at_5);
    println!("Mean Recall@10:      {:.4}", summary.mean_recall_at_10);
    println!("Mean NDCG@10:        {:.4}", summary.mean_ndcg_at_10);
    println!("Total elapsed:       {:.2}s", elapsed.as_secs_f64());
    if !summary.by_category_recall_at_10.is_empty() {
        println!();
        println!("Per category (R@10) — 1=single-hop 2=temporal 3=temporal-inf 4=open-domain 5=adversarial:");
        for (k, v) in &summary.by_category_recall_at_10 {
            println!("  category {k}             {v:.4}");
        }
    }
    println!();
    println!("Results: {}", run_dir.display());
    println!("  results.jsonl   {}", jsonl_path.display());
    println!("  summary.json    {}", summary_path.display());
    println!("  repro.sh        {}", repro_path.display());

    let bench_specific = serde_json::to_value(&summary).unwrap_or(serde_json::Value::Null);
    emit_bench_telemetry(
        "locomo",
        BenchRunPayload {
            bench_name: format!("locomo-{mode}"),
            seed: 0,
            composite: summary.mean_recall_at_10,
            pass: true,
            dimensions: Vec::new(),
            dimension_scores: HashMap::new(),
            bench_specific_stats: bench_specific,
            wall_duration_ms: elapsed.as_millis() as u64,
            result_count: 0,
        },
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_parses_forge_persist_subcommand_with_defaults() {
        // Cycle (i1): drives the ForgePersist variant into existence
        // via clap's Derive parser. Only `--daemon-bin` is provided;
        // every other flag must fall back to its default from §8.
        let cli = Cli::try_parse_from([
            "forge-bench",
            "forge-persist",
            "--daemon-bin",
            "/tmp/forge-daemon",
        ])
        .expect("forge-persist subcommand should parse with --daemon-bin only");
        match cli.command {
            Commands::ForgePersist {
                memories,
                chunks,
                fisp_messages,
                seed,
                kill_after,
                output,
                daemon_bin,
                recovery_timeout_ms,
                worker_catchup_ms,
                request_timeout_ms,
            } => {
                assert_eq!(memories, 100);
                assert_eq!(chunks, 50);
                assert_eq!(fisp_messages, 20);
                assert_eq!(seed, 42);
                assert_eq!(kill_after, 0.5);
                assert_eq!(output, PathBuf::from("bench_results"));
                assert_eq!(daemon_bin, Some(PathBuf::from("/tmp/forge-daemon")));
                assert_eq!(recovery_timeout_ms, 5000);
                assert_eq!(worker_catchup_ms, 10000);
                assert_eq!(request_timeout_ms, 30000);
            }
            other => panic!("expected Commands::ForgePersist, got {other:?}"),
        }
    }

    #[test]
    fn test_cli_forge_persist_accepts_all_flags() {
        // Cycle (i1): end-to-end flag override test — every flag
        // from §8 overridden on the command line, and every value
        // correctly propagated into the parsed variant.
        let cli = Cli::try_parse_from([
            "forge-bench",
            "forge-persist",
            "--memories",
            "25",
            "--chunks",
            "5",
            "--fisp-messages",
            "3",
            "--seed",
            "7",
            "--kill-after",
            "0.25",
            "--output",
            "/tmp/persist_out",
            "--daemon-bin",
            "/tmp/forge-daemon",
            "--recovery-timeout-ms",
            "9000",
            "--worker-catchup-ms",
            "15000",
            "--request-timeout-ms",
            "60000",
        ])
        .expect("all flags should parse");
        match cli.command {
            Commands::ForgePersist {
                memories,
                chunks,
                fisp_messages,
                seed,
                kill_after,
                output,
                daemon_bin,
                recovery_timeout_ms,
                worker_catchup_ms,
                request_timeout_ms,
            } => {
                assert_eq!(memories, 25);
                assert_eq!(chunks, 5);
                assert_eq!(fisp_messages, 3);
                assert_eq!(seed, 7);
                assert_eq!(kill_after, 0.25);
                assert_eq!(output, PathBuf::from("/tmp/persist_out"));
                assert_eq!(daemon_bin, Some(PathBuf::from("/tmp/forge-daemon")));
                assert_eq!(recovery_timeout_ms, 9000);
                assert_eq!(worker_catchup_ms, 15000);
                assert_eq!(request_timeout_ms, 60000);
            }
            other => panic!("expected Commands::ForgePersist, got {other:?}"),
        }
    }

    #[test]
    fn test_cli_parses_forge_context_subcommand_with_defaults() {
        let cli = Cli::try_parse_from(["forge-bench", "forge-context"])
            .expect("forge-context subcommand should parse with defaults");
        match cli.command {
            Commands::ForgeContext { seed, output } => {
                assert_eq!(seed, 42);
                assert_eq!(output, PathBuf::from("bench_results_context"));
            }
            other => panic!("expected Commands::ForgeContext, got {other:?}"),
        }
    }

    #[test]
    fn test_cli_forge_context_accepts_all_flags() {
        let cli = Cli::try_parse_from([
            "forge-bench",
            "forge-context",
            "--seed",
            "99",
            "--output",
            "/tmp/context_out",
        ])
        .expect("all flags should parse");
        match cli.command {
            Commands::ForgeContext { seed, output } => {
                assert_eq!(seed, 99);
                assert_eq!(output, PathBuf::from("/tmp/context_out"));
            }
            other => panic!("expected Commands::ForgeContext, got {other:?}"),
        }
    }

    #[test]
    fn test_cli_parses_forge_consolidation_subcommand_with_defaults() {
        let cli = Cli::try_parse_from(["forge-bench", "forge-consolidation"])
            .expect("forge-consolidation subcommand should parse with defaults");
        match cli.command {
            Commands::ForgeConsolidation {
                seed,
                output,
                expected_recall_delta,
            } => {
                assert_eq!(seed, 42);
                assert_eq!(output, PathBuf::from("bench_results_consolidation"));
                assert!(expected_recall_delta.is_none());
            }
            other => panic!("expected Commands::ForgeConsolidation, got {other:?}"),
        }
    }

    #[test]
    fn test_cli_forge_consolidation_accepts_all_flags() {
        let cli = Cli::try_parse_from([
            "forge-bench",
            "forge-consolidation",
            "--seed",
            "7",
            "--output",
            "/tmp/cons_out",
            "--expected-recall-delta",
            "0.15",
        ])
        .expect("all flags should parse");
        match cli.command {
            Commands::ForgeConsolidation {
                seed,
                output,
                expected_recall_delta,
            } => {
                assert_eq!(seed, 7);
                assert_eq!(output, PathBuf::from("/tmp/cons_out"));
                assert_eq!(expected_recall_delta, Some(0.15));
            }
            other => panic!("expected Commands::ForgeConsolidation, got {other:?}"),
        }
    }
}
