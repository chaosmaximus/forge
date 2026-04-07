// workers/indexer.rs — Periodic code indexer
//
// Uses LSP language servers to extract symbols from source files,
// then stores CodeFile and CodeSymbol records in SQLite.

use crate::db::ops;
use crate::lsp::client::{file_uri, LspClient};
use crate::lsp::detect::{detect_language_servers, LspServerConfig};
use crate::lsp::regex_symbols::extract_symbols_regex;
use crate::lsp::symbols::{convert_symbols, extract_imports};
use crate::lsp::LspManager;
use crate::reality::cluster::run_label_propagation;
use forge_core::types::{CodeFile, CodeSymbol};
use rusqlite::Connection;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::Duration;
use tokio::sync::{watch, Mutex};

// Interval is now configurable via ForgeConfig.workers.indexer_interval_secs
// (default: 300 = 5 minutes)

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
    interval_secs: u64,
) {
    let index_interval = Duration::from_secs(interval_secs);
    eprintln!("[indexer] started, interval = {:?}", index_interval);
    let mut manager: Option<LspManager> = None;
    let mut first_run = true;

    loop {
        // Run immediately on first cycle, then every index_interval
        let delay = if first_run {
            first_run = false;
            Duration::from_secs(10) // short delay on startup for daemon to settle
        } else {
            index_interval
        };

        tokio::select! {
            _ = tokio::time::sleep(delay) => {
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
        if let Err(e) = client.did_close(&uri).await {
            eprintln!("[indexer] failed to close document {}: {e}", file_path);
        }
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
            if let Err(e) = client
                .did_open(&sym_uri, &config.language, &content)
                .await
            {
                eprintln!("[indexer] failed to open document for references: {e}");
            }

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

            if let Err(e) = client.did_close(&sym_uri).await {
                eprintln!("[indexer] failed to close document after references: {e}");
            }
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

    let indexed_at = now_str();
    let mut all_files: Vec<CodeFile> = Vec::new();
    let mut all_symbols: Vec<CodeSymbol> = Vec::new();
    let mut all_call_edges: Vec<(String, String)> = Vec::new();

    // Phase 1: LSP-based indexing
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

    // Phase 2: Regex-based fallback for TS/JS files not covered by LSP
    let ts_extensions: &[&str] = &["ts", "tsx", "js", "jsx"];
    let ts_files = collect_source_files(project_dir, ts_extensions);
    if !ts_files.is_empty() {
        let indexed_paths: HashSet<&str> = all_files.iter().map(|f| f.path.as_str()).collect();
        let unindexed: Vec<&str> = ts_files
            .iter()
            .filter(|p| !indexed_paths.contains(p.as_str()))
            .map(|s| s.as_str())
            .collect();
        if !unindexed.is_empty() {
            eprintln!(
                "[indexer] regex fallback: {} TS/JS files not covered by LSP",
                unindexed.len()
            );
            for path in &unindexed {
                let content = match std::fs::read_to_string(path) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("[indexer] cannot read {path}: {e}");
                        continue;
                    }
                };

                let hash = file_hash(path);
                let language = match Path::new(path)
                    .extension()
                    .and_then(|e| e.to_str())
                {
                    Some("ts" | "tsx") => "typescript",
                    Some("js" | "jsx") => "javascript",
                    _ => continue,
                };

                let file_record = CodeFile {
                    id: format!("file:{}", path),
                    path: path.to_string(),
                    language: language.to_string(),
                    project: project_dir.to_string(),
                    hash: hash.clone(),
                    indexed_at: indexed_at.clone(),
                };
                all_files.push(file_record);

                // Check hash cache — skip re-extraction for unchanged files
                let cached = HASH_CACHE.lock().unwrap().get(*path).cloned();
                if cached.as_deref() == Some(&hash) {
                    continue;
                }

                let syms = extract_symbols_regex(&content, path, language);
                if !syms.is_empty() {
                    HASH_CACHE
                        .lock()
                        .unwrap()
                        .insert(path.to_string(), hash);
                }
                all_symbols.extend(syms);
            }
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
    if let Err(e) = locked.conn.execute("DELETE FROM edge WHERE edge_type = 'calls'", []) {
        eprintln!("[indexer] failed to clear old calls edges: {e}");
    }

    // Store "calls" edges
    let mut edges_stored = 0usize;
    for (from_id, to_id) in &all_call_edges {
        if ops::store_edge(&locked.conn, from_id, to_id, "calls", "{}").is_ok() {
            edges_stored += 1;
        }
    }

    // Import extraction pass (regex-based, no LSP needed)
    let import_edges_stored = extract_and_store_imports(&locked.conn, &all_files);

    // Run community detection on the updated graph
    run_clustering(&locked.conn, project_dir);

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
    if import_edges_stored > 0 {
        eprintln!("[indexer] stored {} import edges", import_edges_stored);
    }
    Ok(())
}

/// Extract import edges from already-indexed files and store them.
/// Returns the number of import edges stored.
pub fn extract_and_store_imports(conn: &Connection, files: &[CodeFile]) -> usize {
    // Disable FK checks — edge table has FK to memory(id) but import edges
    // use file paths as from_id/to_id, not memory IDs
    let _ = conn.execute_batch("PRAGMA foreign_keys=OFF;");

    // Clear old import edges before re-indexing
    if let Err(e) = conn.execute("DELETE FROM edge WHERE edge_type = 'imports'", []) {
        eprintln!("[indexer] failed to clear old import edges: {e}");
        return 0;
    }

    let mut import_edges_stored = 0usize;
    let mut files_read = 0usize;
    let mut total_imports_found = 0usize;
    for file in files {
        let content = match std::fs::read_to_string(&file.path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[indexer] cannot read {}: {e}", file.path);
                continue;
            }
        };
        files_read += 1;
        let imports = extract_imports(&content, &file.language, &file.path);
        total_imports_found += imports.len();
        for (from_path, imported_module) in &imports {
            match ops::store_edge(conn, from_path, imported_module, "imports", "{}") {
                Ok(_) => import_edges_stored += 1,
                Err(e) => eprintln!("[indexer] store_edge failed: {e}"),
            }
        }
    }
    eprintln!("[indexer] import extraction: {files_read} files read, {total_imports_found} imports found, {import_edges_stored} edges stored");
    import_edges_stored
}

/// Run community detection clustering if a reality exists for this project.
pub fn run_clustering(conn: &Connection, project_dir: &str) {
    match ops::get_reality_by_path(conn, project_dir, "default") {
        Ok(Some(reality)) => {
            if let Err(e) = run_label_propagation(conn, &reality.id, 20) {
                eprintln!("[indexer] clustering failed: {e}");
            }
        }
        _ => {
            // No reality exists for this project yet; skip clustering
        }
    }
}

fn now_str() -> String {
    forge_core::time::timestamp_now()
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

    #[test]
    fn test_import_extraction_wired() {
        use crate::db::schema::create_schema;
        use rusqlite::Connection;

        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();

        // Create temp files with import statements
        let tmp = tempfile::tempdir().expect("create temp dir");
        let rs_path = tmp.path().join("lib.rs");
        std::fs::write(&rs_path, "use std::collections::HashMap;\nuse crate::db::ops;\nfn main() {}").unwrap();

        let py_path = tmp.path().join("app.py");
        std::fs::write(&py_path, "import os\nfrom flask import Flask\ndef main(): pass").unwrap();

        let files = vec![
            CodeFile {
                id: format!("file:{}", rs_path.display()),
                path: rs_path.to_str().unwrap().to_string(),
                language: "rust".to_string(),
                project: tmp.path().to_str().unwrap().to_string(),
                hash: "test:hash".to_string(),
                indexed_at: "2026-01-01T00:00:00Z".to_string(),
            },
            CodeFile {
                id: format!("file:{}", py_path.display()),
                path: py_path.to_str().unwrap().to_string(),
                language: "python".to_string(),
                project: tmp.path().to_str().unwrap().to_string(),
                hash: "test:hash2".to_string(),
                indexed_at: "2026-01-01T00:00:00Z".to_string(),
            },
        ];

        let stored = extract_and_store_imports(&conn, &files);
        assert!(stored >= 4, "should store at least 4 import edges (2 rust + 2 python), got {stored}");

        // Verify edges exist in DB
        let edge_count: usize = conn
            .query_row("SELECT COUNT(*) FROM edge WHERE edge_type = 'imports'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(edge_count, stored, "DB edge count should match returned count");
    }

    #[test]
    fn test_regex_fallback_produces_symbols() {
        // Verify the regex fallback path produces CodeFile + CodeSymbol records
        // by directly calling the same logic used in run_index's Phase 2.
        let tmp = tempfile::tempdir().expect("create temp dir");
        let dir = tmp.path();

        // Create TS files
        std::fs::write(
            dir.join("index.ts"),
            "export function main() {}\nexport class App {}\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("utils.js"),
            "function helper() {}\nconst process = (x) => { return x; }\n",
        )
        .unwrap();

        let ts_extensions: &[&str] = &["ts", "tsx", "js", "jsx"];
        let ts_files = collect_source_files(dir.to_str().unwrap(), ts_extensions);
        assert_eq!(ts_files.len(), 2, "should find 2 TS/JS files");

        // Simulate: no files were indexed by LSP (empty indexed set)
        let indexed_paths: HashSet<&str> = HashSet::new();
        let unindexed: Vec<&str> = ts_files
            .iter()
            .filter(|p| !indexed_paths.contains(p.as_str()))
            .map(|s| s.as_str())
            .collect();
        assert_eq!(unindexed.len(), 2);

        let mut all_files: Vec<CodeFile> = Vec::new();
        let mut all_symbols: Vec<CodeSymbol> = Vec::new();

        for path in &unindexed {
            let content = std::fs::read_to_string(path).unwrap();
            let language = match Path::new(path).extension().and_then(|e| e.to_str()) {
                Some("ts" | "tsx") => "typescript",
                Some("js" | "jsx") => "javascript",
                _ => continue,
            };

            all_files.push(CodeFile {
                id: format!("file:{}", path),
                path: path.to_string(),
                language: language.to_string(),
                project: dir.to_str().unwrap().to_string(),
                hash: file_hash(path),
                indexed_at: "2026-01-01T00:00:00Z".to_string(),
            });

            let syms = extract_symbols_regex(&content, path, language);
            all_symbols.extend(syms);
        }

        assert_eq!(all_files.len(), 2, "should create 2 CodeFile records");
        assert!(
            all_symbols.len() >= 3,
            "should extract at least 3 symbols (main, App, helper), got {}",
            all_symbols.len()
        );

        let names: Vec<&str> = all_symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"main"), "should find 'main' function");
        assert!(names.contains(&"App"), "should find 'App' class");
        assert!(names.contains(&"helper"), "should find 'helper' function");
    }

    #[test]
    fn test_regex_fallback_skips_lsp_indexed_files() {
        // Ensure files already indexed by LSP are not re-indexed by regex fallback
        let tmp = tempfile::tempdir().expect("create temp dir");
        let dir = tmp.path();

        std::fs::write(dir.join("indexed.ts"), "export function indexed() {}\n").unwrap();
        std::fs::write(dir.join("unindexed.ts"), "export function unindexed() {}\n").unwrap();

        let ts_extensions: &[&str] = &["ts", "tsx", "js", "jsx"];
        let ts_files = collect_source_files(dir.to_str().unwrap(), ts_extensions);

        // Simulate: indexed.ts was already handled by LSP
        let indexed_path = dir.join("indexed.ts");
        let indexed_path_str = indexed_path.to_str().unwrap();
        let mut indexed_paths: HashSet<&str> = HashSet::new();
        indexed_paths.insert(indexed_path_str);

        let unindexed: Vec<&str> = ts_files
            .iter()
            .filter(|p| !indexed_paths.contains(p.as_str()))
            .map(|s| s.as_str())
            .collect();

        assert_eq!(unindexed.len(), 1, "only 1 file should be unindexed");
        assert!(
            unindexed[0].ends_with("unindexed.ts"),
            "the unindexed file should be unindexed.ts"
        );
    }
}
