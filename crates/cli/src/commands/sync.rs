use crate::client;
use forge_core::protocol::{Request, Response, ResponseData};

/// Export memories as NDJSON lines with HLC metadata.
/// Outputs one JSON line per memory to stdout.
pub async fn sync_export(project: Option<String>, since: Option<String>) {
    let request = Request::SyncExport { project, since };

    match client::send(&request).await {
        Ok(Response::Ok {
            data: ResponseData::SyncExported { lines, count, node_id },
        }) => {
            // Output NDJSON to stdout (for piping to SSH)
            for line in &lines {
                println!("{line}");
            }
            eprintln!("Exported {count} entries from node {node_id}");
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("unexpected response");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

/// Import NDJSON memory lines from stdin.
pub async fn sync_import() {
    use std::io::BufRead;

    let stdin = std::io::stdin();
    let lines: Vec<String> = stdin
        .lock()
        .lines()
        .map_while(Result::ok)
        .filter(|l| !l.trim().is_empty())
        .collect();

    if lines.is_empty() {
        eprintln!("No input received on stdin.");
        return;
    }

    let request = Request::SyncImport { lines };

    match client::send(&request).await {
        Ok(Response::Ok {
            data: ResponseData::SyncImported { imported, conflicts, skipped },
        }) => {
            println!("Sync import complete:");
            println!("  Imported:  {imported}");
            println!("  Conflicts: {conflicts}");
            println!("  Skipped:   {skipped}");
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("unexpected response");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

/// Pull memories from a remote host via SSH.
/// Runs: ssh <host> forge-next sync-export [--project P] | local sync-import
pub async fn sync_pull(host: String, project: Option<String>) {
    let mut remote_cmd = "forge-next sync-export".to_string();
    if let Some(ref p) = project {
        remote_cmd.push_str(&format!(" --project {p}"));
    }

    eprintln!("Pulling from {host}...");

    // Run ssh to get remote export
    let output = std::process::Command::new("ssh")
        .arg(&host)
        .arg(&remote_cmd)
        .output();

    let output = match output {
        Ok(o) => o,
        Err(e) => {
            eprintln!("error: failed to run ssh: {e}");
            std::process::exit(1);
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        eprintln!("error: ssh command failed: {stderr}");
        std::process::exit(1);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<String> = stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.to_string())
        .collect();

    if lines.is_empty() {
        eprintln!("No data received from remote.");
        return;
    }

    eprintln!("Received {} lines, importing...", lines.len());

    // Import locally
    let request = Request::SyncImport { lines };

    match client::send(&request).await {
        Ok(Response::Ok {
            data: ResponseData::SyncImported { imported, conflicts, skipped },
        }) => {
            println!("Sync pull complete:");
            println!("  Imported:  {imported}");
            println!("  Conflicts: {conflicts}");
            println!("  Skipped:   {skipped}");
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("unexpected response");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

/// Push memories to a remote host via SSH.
/// Runs: local sync-export | ssh <host> forge-next sync-import
pub async fn sync_push(host: String, project: Option<String>) {
    eprintln!("Pushing to {host}...");

    // First, export locally
    let request = Request::SyncExport {
        project,
        since: None,
    };

    let lines = match client::send(&request).await {
        Ok(Response::Ok {
            data: ResponseData::SyncExported { lines, count, node_id: _ },
        }) => {
            eprintln!("Exporting {count} entries...");
            lines
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("unexpected response");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };

    if lines.is_empty() {
        eprintln!("Nothing to push.");
        return;
    }

    // Pipe to remote via SSH
    let ndjson = lines.join("\n");
    let mut child = match std::process::Command::new("ssh")
        .arg(&host)
        .arg("forge-next sync-import")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: failed to run ssh: {e}");
            std::process::exit(1);
        }
    };

    // Write to stdin
    if let Some(ref mut stdin) = child.stdin {
        use std::io::Write;
        let _ = stdin.write_all(ndjson.as_bytes());
        let _ = stdin.write_all(b"\n");
    }
    // Close stdin by dropping it
    drop(child.stdin.take());

    let output = match child.wait_with_output() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("error: ssh command failed: {e}");
            std::process::exit(1);
        }
    };

    let stdout_text = String::from_utf8_lossy(&output.stdout);
    let stderr_text = String::from_utf8_lossy(&output.stderr);

    if !output.status.success() {
        eprintln!("error: remote sync-import failed: {stderr_text}");
        std::process::exit(1);
    }

    println!("Sync push complete.");
    if !stdout_text.is_empty() {
        println!("{}", stdout_text.trim());
    }
    if !stderr_text.is_empty() {
        eprintln!("{}", stderr_text.trim());
    }
}

/// List unresolved sync conflicts.
pub async fn sync_conflicts() {
    let request = Request::SyncConflicts;

    match client::send(&request).await {
        Ok(Response::Ok {
            data: ResponseData::SyncConflictList { conflicts },
        }) => {
            if conflicts.is_empty() {
                println!("No unresolved sync conflicts.");
                return;
            }

            println!("Unresolved Sync Conflicts ({} pairs)", conflicts.len());
            println!("{}", "\u{2500}".repeat(60));

            for (i, pair) in conflicts.iter().enumerate() {
                println!("\n{}. {} [{}]", i + 1, pair.title, pair.memory_type);

                println!("  LOCAL  (node: {}, hlc: {})",
                    pair.local.node_id,
                    pair.local.hlc_timestamp,
                );
                println!("    ID: {}", pair.local.id);
                println!("    Content: {}",
                    truncate(&pair.local.content, 80),
                );

                if !pair.remote.id.is_empty() {
                    println!("  REMOTE (node: {}, hlc: {})",
                        pair.remote.node_id,
                        pair.remote.hlc_timestamp,
                    );
                    println!("    ID: {}", pair.remote.id);
                    println!("    Content: {}",
                        truncate(&pair.remote.content, 80),
                    );
                }

                println!("\n  Resolve: forge-next sync-resolve <id>");
            }
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("unexpected response");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

/// Resolve a sync conflict by keeping the given memory ID.
pub async fn sync_resolve(id: String) {
    let request = Request::SyncResolve {
        keep_id: id.clone(),
    };

    match client::send(&request).await {
        Ok(Response::Ok {
            data: ResponseData::SyncResolved { id: resolved_id, resolved },
        }) => {
            if resolved {
                println!("Conflict resolved. Kept: {resolved_id}");
            } else {
                eprintln!("No conflict found with ID: {resolved_id}");
                std::process::exit(1);
            }
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("unexpected response");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

/// Backfill HLC timestamps on existing memories that have empty hlc_timestamp.
pub async fn hlc_backfill() {
    let request = Request::HlcBackfill;

    match client::send(&request).await {
        Ok(Response::Ok {
            data: ResponseData::HlcBackfilled { count },
        }) => {
            if count == 0 {
                println!("All memories already have HLC timestamps.");
            } else {
                println!("Backfilled {count} memories with HLC timestamps.");
            }
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("unexpected response");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}
