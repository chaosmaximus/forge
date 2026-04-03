// workers/indexer.rs — Periodic code indexer
//
// Uses LSP language servers to extract symbols from source files,
// then stores CodeFile and CodeSymbol records in SQLite.

use crate::db::ops;
use crate::lsp::client::{file_uri, LspClient};
use crate::lsp::detect::{detect_language_servers, LspServerConfig};
use crate::lsp::symbols::convert_symbols;
use crate::lsp::LspManager;
use forge_core::types::{CodeFile, CodeSymbol};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::Duration;
use tokio::sync::{watch, Mutex};

const INDEX_INTERVAL: Duration = Duration::from_secs(5 * 60); // 5 minutes

/// Directories to skip when walking the project tree.
const SKIP_DIRS: &[&str] = &[
    ".git",
    ".hg",
    "target",
    "node_modules",
    "__pycache__",
    ".mypy_cache",
    ".pytest_cache",
    "dist",
    "build",
    ".tox",
    ".venv",
    "venv",
];

/// Content-hash cache: skips re-indexing unchanged files across index runs.
/// Key = file path, Value = size:mtime hash string.
static HASH_CACHE: std::sync::LazyLock<StdMutex<HashMap<String, String>>> =
    std::sync::LazyLock::new(|| StdMutex::new(HashMap::new()));

pub async fn run_indexer(
    state: Arc<Mutex<crate::server::handler::DaemonState>>,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    eprintln!("[indexer] started, interval = {:?}", INDEX_INTERVAL);
    let mut manager: Option<LspManager> = None;

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
                if !std::path::Path::new(&project_dir).exists() {
                    continue;
                }

                // Create or reuse LspManager
                let mgr = match &mut manager {
                    Some(m) if m.project_dir() == project_dir => m,
                    _ => {
                        // Project changed or first run — create new manager
                        if let Some(old) = manager.take() {
                            old.shutdown_all().await;
                        }
                        manager = Some(LspManager::new(project_dir.clone()));
                        manager.as_mut().unwrap()
                    }
                };

                if let Err(e) = run_index(&project_dir, &state, mgr).await {
                    eprintln!("[indexer] error: {}", e);
                }
            }
            _ = shutdown_rx.changed() => {
                if let Some(mgr) = manager.take() {
                    mgr.shutdown_all().await;
                }
                eprintln!("[indexer] shutting down");
                return;
            }
        }
    }
}

/// Discover the project directory from env or Claude transcript paths.
pub fn find_project_dir() -> Option<String> {
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

/// File extensions that each language server handles.
fn extensions_for_language(language: &str) -> &'static [&'static str] {
    match language {
        "rust" => &["rs"],
        "python" => &["py"],
        "typescript" => &["ts", "tsx", "js", "jsx"],
        "go" => &["go"],
        _ => &[],
    }
}

/// Walk the project directory and collect source files matching the given extensions.
/// Skips symlinks, hidden directories, and known non-source directories.
/// Limited to MAX_DEPTH levels to prevent stack overflow.
fn collect_source_files(project_dir: &str, extensions: &[&str]) -> Vec<String> {
    let skip_dirs: HashSet<&str> = SKIP_DIRS.iter().copied().collect();
    let mut files = Vec::new();
    walk_dir_recursive(Path::new(project_dir), &skip_dirs, extensions, &mut files, 0);
    files
}

const MAX_WALK_DEPTH: usize = 20;

fn walk_dir_recursive(
    dir: &Path,
    skip_dirs: &HashSet<&str>,
    extensions: &[&str],
    out: &mut Vec<String>,
    depth: usize,
) {
    if depth > MAX_WALK_DEPTH {
        return;
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        // Use file_type() which does NOT follow symlinks on DirEntry
        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };

        // Skip symlinks entirely (prevents loops and directory escape)
        if ft.is_symlink() {
            continue;
        }

        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if ft.is_dir() {
            if name_str.starts_with('.') || skip_dirs.contains(name_str.as_ref()) {
                continue;
            }
            walk_dir_recursive(&path, skip_dirs, extensions, out, depth + 1);
        } else if ft.is_file() {
            if let Some(ext) = path.extension() {
                let ext_str = ext.to_string_lossy();
                if extensions.iter().any(|e| *e == ext_str.as_ref()) {
                    if let Some(s) = path.to_str() {
                        out.push(s.to_string());
                    }
                }
            }
        }
    }
}

/// Index all matching files using an LSP client managed by `LspManager`.
///
/// The client is provided by the caller (LspManager handles lifecycle).
/// Returns the collected CodeFiles, CodeSymbols, and call edges on success.
async fn index_with_server(
    client: &mut LspClient,
    config: &LspServerConfig,
    project_dir: &str,
    indexed_at: &str,
) -> Result<(Vec<CodeFile>, Vec<CodeSymbol>, Vec<(String, String)>), String> {
    let extensions = extensions_for_language(&config.language);
    if extensions.is_empty() {
        return Ok((Vec::new(), Vec::new(), Vec::new()));
    }

    let source_files = collect_source_files(project_dir, extensions);
    if source_files.is_empty() {
        return Ok((Vec::new(), Vec::new(), Vec::new()));
    }

    eprintln!(
        "[indexer] {} — found {} files, using persistent LSP server",
        config.language,
        source_files.len()
    );

    // Check server capabilities before requesting symbols (Serena pattern)
    if !client.supports_document_symbols() {
        eprintln!(
            "[indexer] {} does not support documentSymbol, skipping",
            config.command
        );
        return Ok((Vec::new(), Vec::new(), Vec::new()));
    }

    let mut files = Vec::new();
    let mut symbols = Vec::new();

    for file_path in &source_files {
        let uri = file_uri(file_path);
        let hash = file_hash(file_path);

        let file_record = CodeFile {
            id: format!("file:{}", file_path),
            path: file_path.clone(),
            language: config.language.clone(),
            project: project_dir.to_string(),
            hash: hash.clone(),
            indexed_at: indexed_at.to_string(),
        };

        files.push(file_record);

        // Check hash cache — skip LSP symbol request for unchanged files
        // (references are still requested below for ALL symbols, since callers may change)
        let cached = HASH_CACHE.lock().unwrap().get(file_path.as_str()).cloned();
        if cached.as_deref() == Some(&hash) {
            continue; // symbols unchanged — skip didOpen/documentSymbol
        }

        // Send didOpen before requesting symbols (required by LSP protocol)
        let content = std::fs::read_to_string(file_path).unwrap_or_default();
        if let Err(e) = client.did_open(&uri, &config.language, &content).await {
            eprintln!("[indexer] didOpen failed for {}: {}", file_path, e);
            continue;
        }

        // Request symbols with per-file timeout
        match tokio::time::timeout(Duration::from_secs(10), client.document_symbols(&uri)).await {
            Ok(Ok(doc_symbols)) => {
                let converted = convert_symbols(file_path, &doc_symbols);
                symbols.extend(converted);
                // Update hash cache on successful extraction
                HASH_CACHE
                    .lock()
                    .unwrap()
                    .insert(file_path.to_string(), hash);
            }
            Ok(Err(e)) => {
                eprintln!(
                    "[indexer] {} symbols failed for {}: {}",
                    config.language, file_path, e
                );
            }
            Err(_) => {
                eprintln!(
                    "[indexer] {} symbols timed out for {}",
                    config.language, file_path
                );
            }
        }

        // Close the document after processing (Serena pattern: didOpen/didClose lifecycle)
        let _ = client.did_close(&uri).await;
    }

    // Request references for callable symbols -> build "calls" edges
    let mut call_edges: Vec<(String, String)> = Vec::new();
    if client.supports_references() {
        let callable_symbols: Vec<&CodeSymbol> = symbols
            .iter()
            .filter(|s| s.kind == "function" || s.kind == "class")
            .collect();

        // Limit to first 100 symbols to avoid excessive LSP calls
        for sym in callable_symbols.iter().take(100) {
            let sym_uri = file_uri(&sym.file_path);
            let content = std::fs::read_to_string(&sym.file_path).unwrap_or_default();
            let _ = client
                .did_open(&sym_uri, &config.language, &content)
                .await;

            match tokio::time::timeout(
                Duration::from_secs(10),
                // Use character offset 4 (past typical indentation/visibility keywords)
                // to land inside the symbol token rather than at column 0
                // which may be whitespace, a doc comment, or an attribute
                client.references(&sym_uri, sym.line_start as u32, 4),
            )
            .await
            {
                Ok(Ok(refs)) if !refs.is_empty() => {
                    let edges = crate::lsp::symbols::build_call_edges(
                        &sym.id,
                        &sym.file_path,
                        &refs,
                    );
                    call_edges.extend(edges);
                }
                _ => {}
            }

            let _ = client.did_close(&sym_uri).await;
        }
    }

    // Note: no shutdown here — LspManager handles server lifecycle

    Ok((files, symbols, call_edges))
}

/// Compute a simple hash string from file size + mtime.
fn file_hash(path: &str) -> String {
    match std::fs::metadata(path) {
        Ok(meta) => {
            let size = meta.len();
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            format!("{}:{}", size, mtime)
        }
        Err(_) => String::new(),
    }
}

async fn run_index(
    project_dir: &str,
    state: &Arc<Mutex<crate::server::handler::DaemonState>>,
    manager: &mut LspManager,
) -> Result<(), String> {
    let servers = detect_language_servers(project_dir);
    if servers.is_empty() {
        return Ok(()); // no language servers available
    }

    let indexed_at = now_str();
    let mut all_files: Vec<CodeFile> = Vec::new();
    let mut all_symbols: Vec<CodeSymbol> = Vec::new();
    let mut all_call_edges: Vec<(String, String)> = Vec::new();

    for config in &servers {
        match manager.get_client(config).await {
            Ok(client) => {
                match index_with_server(client, config, project_dir, &indexed_at).await {
                    Ok((files, symbols, edges)) => {
                        all_files.extend(files);
                        all_symbols.extend(symbols);
                        all_call_edges.extend(edges);
                    }
                    Err(e) => eprintln!("[indexer] {} failed: {}", config.language, e),
                }
            }
            Err(e) => eprintln!("[indexer] {} spawn failed: {}", config.command, e),
        }
    }

    if all_files.is_empty() {
        return Ok(());
    }

    // Take lock only for the batch DB writes
    let locked = state.lock().await;

    let mut files_stored = 0usize;
    let mut symbols_stored = 0usize;

    for file in &all_files {
        if ops::store_file(&locked.conn, file).is_ok() {
            files_stored += 1;
        }
    }
    for sym in &all_symbols {
        if ops::store_symbol(&locked.conn, sym).is_ok() {
            symbols_stored += 1;
        }
    }

    // Clear old "calls" edges before inserting fresh ones (prevents duplicates across runs)
    let _ = locked.conn.execute("DELETE FROM edge WHERE edge_type = 'calls'", []);

    // Store "calls" edges
    let mut edges_stored = 0usize;
    for (from_id, to_id) in &all_call_edges {
        if ops::store_edge(&locked.conn, from_id, to_id, "calls", "{}").is_ok() {
            edges_stored += 1;
        }
    }

    // Clean up stale entries for files no longer in the index output
    let current_paths: Vec<&str> = all_files.iter().map(|f| f.path.as_str()).collect();
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
    if edges_stored > 0 {
        eprintln!("[indexer] stored {} call edges", edges_stored);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extensions_for_language() {
        assert_eq!(extensions_for_language("rust"), &["rs"]);
        assert_eq!(extensions_for_language("python"), &["py"]);
        assert_eq!(extensions_for_language("typescript"), &["ts", "tsx", "js", "jsx"]);
        assert_eq!(extensions_for_language("go"), &["go"]);
        assert!(extensions_for_language("unknown").is_empty());
    }

    #[test]
    fn test_collect_source_files_skips_hidden() {
        let tmp = tempfile::tempdir().expect("create temp dir");
        let dir = tmp.path();

        // Create a visible .rs file
        std::fs::write(dir.join("main.rs"), "fn main() {}").unwrap();

        // Create a hidden directory with an .rs file — should be skipped
        let hidden = dir.join(".hidden");
        std::fs::create_dir(&hidden).unwrap();
        std::fs::write(hidden.join("secret.rs"), "fn secret() {}").unwrap();

        // Create a node_modules directory — should be skipped
        let nm = dir.join("node_modules");
        std::fs::create_dir(&nm).unwrap();
        std::fs::write(nm.join("dep.rs"), "fn dep() {}").unwrap();

        let files = collect_source_files(dir.to_str().unwrap(), &["rs"]);
        assert_eq!(files.len(), 1, "should only find main.rs, got: {:?}", files);
        assert!(files[0].ends_with("main.rs"));
    }

    #[test]
    fn test_collect_source_files_recursive() {
        let tmp = tempfile::tempdir().expect("create temp dir");
        let dir = tmp.path();

        let src = dir.join("src");
        std::fs::create_dir(&src).unwrap();
        std::fs::write(src.join("lib.rs"), "").unwrap();
        std::fs::write(src.join("main.rs"), "").unwrap();
        std::fs::write(src.join("readme.md"), "").unwrap(); // non-matching extension

        let files = collect_source_files(dir.to_str().unwrap(), &["rs"]);
        assert_eq!(files.len(), 2, "should find 2 .rs files, got: {:?}", files);
    }

    #[test]
    fn test_collect_source_files_typescript_extensions() {
        let tmp = tempfile::tempdir().expect("create temp dir");
        let dir = tmp.path();

        std::fs::write(dir.join("index.ts"), "").unwrap();
        std::fs::write(dir.join("App.tsx"), "").unwrap();
        std::fs::write(dir.join("utils.js"), "").unwrap();
        std::fs::write(dir.join("Button.jsx"), "").unwrap();
        std::fs::write(dir.join("styles.css"), "").unwrap(); // not matched

        let files = collect_source_files(dir.to_str().unwrap(), &["ts", "tsx", "js", "jsx"]);
        assert_eq!(files.len(), 4, "should find 4 TS/JS files, got: {:?}", files);
    }

    #[test]
    fn test_file_hash_nonexistent() {
        let h = file_hash("/nonexistent/path/to/file.rs");
        assert!(h.is_empty());
    }

    #[test]
    fn test_file_hash_valid() {
        let tmp = tempfile::NamedTempFile::new().expect("create temp file");
        std::fs::write(tmp.path(), "hello world").unwrap();
        let h = file_hash(tmp.path().to_str().unwrap());
        assert!(!h.is_empty());
        assert!(h.contains(':'), "hash should be size:mtime format");
    }

    #[test]
    fn test_find_project_dir_env() {
        // This test only verifies that FORGE_PROJECT is checked.
        // We can't easily test the Claude transcript path logic.
        let original = std::env::var("FORGE_PROJECT").ok();

        // Set to a known existing directory
        std::env::set_var("FORGE_PROJECT", "/tmp");
        let result = find_project_dir();
        assert_eq!(result, Some("/tmp".to_string()));

        // Restore
        match original {
            Some(v) => std::env::set_var("FORGE_PROJECT", v),
            None => std::env::remove_var("FORGE_PROJECT"),
        }
    }
}
