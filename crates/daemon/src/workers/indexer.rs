// workers/indexer.rs — Periodic code indexer
//
// Shells out to the v1 `forge index` binary and parses its NDJSON stdout
// to populate SQLite with code_file and code_symbol records.

use crate::db::ops;
use forge_core::types::{CodeFile, CodeSymbol, V1IndexSymbol};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{watch, Mutex};

const INDEX_INTERVAL: Duration = Duration::from_secs(5 * 60); // 5 minutes

pub async fn run_indexer(
    state: Arc<Mutex<crate::server::handler::DaemonState>>,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    eprintln!("[indexer] started, interval = {:?}", INDEX_INTERVAL);

    loop {
        tokio::select! {
            _ = tokio::time::sleep(INDEX_INTERVAL) => {
                let project_dir = match find_project_dir() {
                    Some(dir) => dir,
                    None => {
                        eprintln!("[indexer] no project directory found (FORGE_PROJECT not set, no Claude transcript dirs)");
                        continue;
                    }
                };
                if std::path::Path::new(&project_dir).exists() {
                    let forge_bin = find_forge_binary();
                    if forge_bin.is_empty() {
                        eprintln!("[indexer] forge binary not found on PATH or known locations");
                        continue;
                    }
                    if let Err(e) = run_index(&project_dir, &forge_bin, &state).await {
                        eprintln!("[indexer] error: {}", e);
                    }
                }
            }
            _ = shutdown_rx.changed() => {
                eprintln!("[indexer] shutting down");
                return;
            }
        }
    }
}

/// Discover the project directory from env or Claude transcript paths.
fn find_project_dir() -> Option<String> {
    // 1. Check FORGE_PROJECT env
    if let Ok(dir) = std::env::var("FORGE_PROJECT") {
        if dir != "." && std::path::Path::new(&dir).is_dir() {
            return Some(dir);
        }
    }

    // 2. Infer from Claude Code transcript directory names.
    //    Claude encodes project paths as e.g. `-mnt-colab-disk-DurgaSaiK-forge`
    //    which maps back to `/mnt/colab-disk/DurgaSaiK/forge`.
    let home = std::env::var("HOME").unwrap_or_default();
    let projects_dir = format!("{}/.claude/projects", home);
    let entries = match std::fs::read_dir(&projects_dir) {
        Ok(e) => e,
        Err(_) => return None,
    };

    // Collect entries and sort by modification time (most recent first)
    let mut candidates: Vec<_> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.starts_with('-') {
                return None;
            }
            let mtime = entry.metadata().ok()?.modified().ok()?;
            Some((name, mtime))
        })
        .collect();
    candidates.sort_by(|a, b| b.1.cmp(&a.1));

    for (name, _) in &candidates {
        // Decode: replace leading/internal dashes with slashes
        let decoded = name.replace('-', "/");
        // Walk backwards from the full decoded path to find a real directory
        // e.g. "/mnt/colab/disk/..." — try progressively shorter prefixes
        // Only check at slash boundaries
        let bytes = decoded.as_bytes();
        for i in (1..bytes.len()).rev() {
            if bytes[i] == b'/' {
                let candidate = &decoded[..i];
                if std::path::Path::new(candidate).is_dir() {
                    return Some(candidate.to_string());
                }
            }
        }
    }

    None
}

/// Find the `forge` (v1) binary on PATH or at known locations.
fn find_forge_binary() -> String {
    // Check PATH first
    if std::process::Command::new("forge")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok()
    {
        return "forge".to_string();
    }

    // Try known locations
    let home = std::env::var("HOME").unwrap_or_default();
    let candidates = [
        format!("{}/.local/bin/forge", home),
        "/mnt/colab-disk/DurgaSaiK/forge/target/release/forge".to_string(),
    ];
    for path in &candidates {
        if std::path::Path::new(path).exists() {
            return path.clone();
        }
    }

    String::new()
}

async fn run_index(
    project_dir: &str,
    forge_bin: &str,
    state: &Arc<Mutex<crate::server::handler::DaemonState>>,
) -> Result<(), String> {
    let output = tokio::time::timeout(
        Duration::from_secs(120),
        tokio::process::Command::new(forge_bin)
            .args(["index", project_dir])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output(),
    )
    .await
    .map_err(|_| "forge index timed out (120s)".to_string())?
    .map_err(|e| format!("forge index failed: {}", e))?;

    if !output.status.success() {
        return Err(format!("forge index exited with {}", output.status));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let indexed_at = now_str();

    // MEDIUM FIX: Parse NDJSON OUTSIDE the Mutex lock to avoid holding it during parsing
    let mut files_to_store: Vec<CodeFile> = Vec::new();
    let mut symbols_to_store: Vec<CodeSymbol> = Vec::new();

    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if let Ok(v1) = serde_json::from_str::<V1IndexSymbol>(trimmed) {
            let file = CodeFile {
                id: format!("file:{}", v1.file_path),
                path: v1.file_path.clone(),
                language: v1.language.clone().unwrap_or_else(|| "unknown".into()),
                project: project_dir.to_string(),
                hash: v1.hash.clone().unwrap_or_default(),
                indexed_at: indexed_at.clone(),
            };
            files_to_store.push(file);

            // MEDIUM FIX: Skip v1 "file" records — they represent files, not symbols.
            // Only store actual symbols (function, class, method, etc.).
            if v1.kind != "file" {
                let sym = CodeSymbol {
                    id: v1.id.clone(),
                    name: v1.name.clone(),
                    kind: v1.kind.clone(),
                    file_path: v1.file_path.clone(),
                    line_start: v1.line_start.unwrap_or(0),
                    line_end: v1.line_end,
                    signature: v1.signature.clone(),
                };
                symbols_to_store.push(sym);
            }
        }
    }

    // Take lock only for the batch DB writes
    let locked = state.lock().await;

    let mut files_stored = 0usize;
    let mut symbols_stored = 0usize;

    for file in &files_to_store {
        if ops::store_file(&locked.conn, file).is_ok() {
            files_stored += 1;
        }
    }
    for sym in &symbols_to_store {
        if ops::store_symbol(&locked.conn, sym).is_ok() {
            symbols_stored += 1;
        }
    }

    // HIGH-2 FIX: Clean up stale entries for files no longer in the index output
    let current_paths: Vec<&str> = files_to_store.iter().map(|f| f.path.as_str()).collect();
    if let Ok(cleaned) = ops::cleanup_stale_files(&locked.conn, &current_paths) {
        if cleaned > 0 {
            eprintln!("[indexer] cleaned {} stale entries", cleaned);
        }
    }

    drop(locked); // release lock immediately

    if symbols_stored > 0 {
        eprintln!(
            "[indexer] indexed {} symbols across {} file entries",
            symbols_stored, files_stored
        );
    }
    Ok(())
}

fn now_str() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{}", secs)
}
