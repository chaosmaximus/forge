// forge-bench — benchmark runner binary for the Forge raw layer.
//
// Embedded harness: spins up an in-memory SQLite per question, runs the
// chosen mode against a real `MiniLMEmbedder` (or a `FakeEmbedder` when
// `--fake-embedder` is set for offline smoke tests), and writes per-question
// JSONL plus a summary JSON to disk.
//
// Designed for benchmark parity with MemPalace's reference scripts — see
// docs/benchmarks/plan.md §5 for the harness contract.

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

    Ok(())
}

fn unix_secs_string() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
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

    Ok(())
}
