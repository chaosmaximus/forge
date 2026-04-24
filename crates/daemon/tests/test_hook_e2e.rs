/// E2E tests for Forge hooks — validates the full pipeline:
/// Claude Code hook → shell script → forge-next CLI → daemon socket → response
///
/// Two tests that reach the full hook → CLI → daemon path spin up an
/// isolated daemon in a tempdir (via `env!("CARGO_BIN_EXE_forge-daemon")`)
/// and point the `forge-next` subprocess at the same FORGE_DIR. Other
/// tests exercise only the hook scripts + their CLI subcommands and do
/// not need a daemon.
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

fn forge_next() -> String {
    // forge-next is in a different crate (forge-cli) so CARGO_BIN_EXE_forge-next
    // is NOT set in this test binary's env. Discover on disk instead — CI
    // builds both binaries before running integration tests (2P-1b §11).
    let candidates = [
        "target/release/forge-next",
        "../target/release/forge-next",
        "../../target/release/forge-next",
        "target/debug/forge-next",
        "../target/debug/forge-next",
        "../../target/debug/forge-next",
    ];
    for c in &candidates {
        if std::path::Path::new(c).exists() {
            return c.to_string();
        }
    }
    panic!(
        "forge-next binary not found — run `cargo build --bin forge-next` \
         before these hook e2e tests"
    );
}

fn hooks_dir() -> String {
    let candidates = ["scripts/hooks", "../scripts/hooks", "../../scripts/hooks"];
    for c in &candidates {
        if std::path::Path::new(c).exists() {
            return c.to_string();
        }
    }
    "scripts/hooks".to_string()
}

/// Ephemeral daemon bound to a tempdir FORGE_DIR, killed on drop.
struct TestDaemon {
    process: Child,
    forge_dir: PathBuf,
    _dir: tempfile::TempDir,
}

impl TestDaemon {
    fn start() -> Self {
        let dir = tempfile::tempdir().expect("create tempdir for TestDaemon");
        let forge_dir = dir.path().join(".forge");
        std::fs::create_dir_all(&forge_dir).expect("create .forge subdir");

        let daemon_bin = PathBuf::from(env!("CARGO_BIN_EXE_forge-daemon"));
        assert!(
            daemon_bin.exists(),
            "CARGO_BIN_EXE_forge-daemon should point at a built binary: {daemon_bin:?}"
        );

        let mut process = Command::new(&daemon_bin)
            .env("FORGE_DIR", &forge_dir)
            .env("HOME", dir.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn daemon");

        // Wait for the socket to appear (up to 30s). Debug daemon cold-boot
        // is ~10-15s due to SQLite migrate + worker spawn + ORT link.
        let socket = forge_dir.join("forge.sock");
        for _ in 0..300 {
            if socket.exists() {
                std::thread::sleep(Duration::from_millis(200));
                return TestDaemon {
                    process,
                    forge_dir,
                    _dir: dir,
                };
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        let _ = process.kill();
        let _ = process.wait();
        panic!("daemon did not create socket within 30s — set up pre-test build");
    }

    fn forge_dir(&self) -> &Path {
        &self.forge_dir
    }
}

impl Drop for TestDaemon {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

// ─── Session Start Hook ──────────────────────────────────────────

#[test]
fn test_session_start_hook_outputs_valid_json() {
    let output = Command::new("bash")
        .arg(format!("{}/session-start.sh", hooks_dir()))
        .env("CLAUDE_CWD", "/tmp")
        .env("CLAUDE_SESSION_ID", "test-hook-json")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run session-start hook");

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Should contain valid JSON with hookSpecificOutput
    assert!(
        stdout.contains("hookSpecificOutput"),
        "session-start hook must output hookSpecificOutput JSON, got: {stdout}"
    );
    assert!(
        stdout.contains("forge-context"),
        "session-start hook must include forge-context XML, got: {stdout}"
    );
}

#[test]
fn test_session_start_hook_registers_session() {
    let daemon = TestDaemon::start();
    let session_id = format!(
        "hook-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    );

    let cwd = tempfile::tempdir().expect("tempdir");
    let _ = Command::new("bash")
        .arg(format!("{}/session-start.sh", hooks_dir()))
        .env("CLAUDE_CWD", cwd.path())
        .env("CLAUDE_SESSION_ID", &session_id)
        .env("FORGE_DIR", daemon.forge_dir())
        .env("FORGE_NEXT", forge_next())
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run session-start hook");

    let output = Command::new(forge_next())
        .args(["sessions", "--all"])
        .env("FORGE_DIR", daemon.forge_dir())
        .output()
        .expect("list sessions");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(&session_id),
        "session {session_id} should be registered after hook, got: {stdout}"
    );
}

// ─── Pre-Edit Hook ──────────────────────────────────────────────

#[test]
fn test_pre_edit_hook_allows_normal_files() {
    let output = Command::new("bash")
        .arg(format!("{}/pre-edit.sh", hooks_dir()))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            if let Some(ref mut stdin) = child.stdin {
                stdin.write_all(b"{\"tool_input\":{\"file_path\":\"src/main.rs\"}}")?;
            }
            child.wait_with_output()
        })
        .expect("run pre-edit hook");

    assert_eq!(
        output.status.code(),
        Some(0),
        "normal file should be allowed"
    );
}

#[test]
fn test_pre_edit_hook_blocks_env_file() {
    let output = Command::new("bash")
        .arg(format!("{}/pre-edit.sh", hooks_dir()))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            if let Some(ref mut stdin) = child.stdin {
                stdin.write_all(b"{\"tool_input\":{\"file_path\":\".env\"}}")?;
            }
            child.wait_with_output()
        })
        .expect("run pre-edit hook");

    assert_eq!(
        output.status.code(),
        Some(2),
        ".env file should be blocked (exit 2)"
    );
}

#[test]
fn test_pre_edit_hook_blocks_sensitive_files() {
    let sensitive = vec![
        ".env.local",
        "credentials.json",
        "secrets.yaml",
        "server.key",
        "cert.pem",
        "id_rsa",
        "kubeconfig",
        "service-account.json",
        "token.json",
    ];

    for file in sensitive {
        let output = Command::new("bash")
            .arg(format!("{}/pre-edit.sh", hooks_dir()))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write;
                if let Some(ref mut stdin) = child.stdin {
                    let input = format!("{{\"tool_input\":{{\"file_path\":\"{file}\"}}}}");
                    stdin.write_all(input.as_bytes())?;
                }
                child.wait_with_output()
            })
            .expect("run pre-edit hook");

        assert_eq!(output.status.code(), Some(2), "{file} should be blocked");
    }
}

#[test]
fn test_pre_edit_hook_rejects_shell_injection() {
    let malicious = vec![
        "; rm -rf /",
        "| cat /etc/passwd",
        "$(whoami)",
        "`id`",
        "file\\npath",
    ];

    for input in malicious {
        let output = Command::new("bash")
            .arg(format!("{}/pre-edit.sh", hooks_dir()))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write;
                if let Some(ref mut stdin) = child.stdin {
                    let json = format!("{{\"tool_input\":{{\"file_path\":\"{input}\"}}}}");
                    stdin.write_all(json.as_bytes())?;
                }
                child.wait_with_output()
            })
            .expect("run pre-edit hook");

        // Should exit 0 (silently ignore) not crash
        assert!(
            output.status.success(),
            "shell injection '{}' should be safely ignored, got exit {}",
            input,
            output.status.code().unwrap_or(-1)
        );
    }
}

// ─── Session End Hook ──────────────────────────────────────────

#[test]
fn test_session_end_hook_succeeds() {
    let output = Command::new("bash")
        .arg(format!("{}/session-end.sh", hooks_dir()))
        .env("CLAUDE_SESSION_ID", "test-end-hook")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run session-end hook");

    assert!(output.status.success(), "session-end hook should succeed");
}

// ─── Full Pipeline: Hook → Daemon → Response ──────────────────

#[test]
fn test_full_pipeline_remember_check_via_cli() {
    let daemon = TestDaemon::start();

    // 1. Store a decision that mentions a file path in content.
    //    Since Session 12, the remember handler auto-creates affects edges
    //    for file paths matching (crates|src|lib|app)/.../*.rs patterns.
    let output = Command::new(forge_next())
        .args([
            "remember",
            "--type",
            "decision",
            "--title",
            "Hook E2E test decision",
            "--content",
            "This decision is linked to src/hook_test.rs for testing",
            "--project",
            "forge",
        ])
        .env("FORGE_DIR", daemon.forge_dir())
        .output()
        .expect("remember");
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    let id = stdout.split("Stored: ").nth(1).unwrap_or("").trim();

    // 2. Guardrails check on the mentioned file → should find the linked decision
    //    (affects edge was auto-created by the remember handler)
    let output = Command::new(forge_next())
        .args(["check", "--file", "src/hook_test.rs", "--action", "edit"])
        .env("FORGE_DIR", daemon.forge_dir())
        .output()
        .expect("check");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("decision") || stdout.contains("linked"),
        "file mentioned in decision content should have affects edge: {stdout}"
    );

    // 3. Guardrails check on a file NOT mentioned → should be safe
    let output = Command::new(forge_next())
        .args([
            "check",
            "--file",
            "src/unrelated_file_xyz.rs",
            "--action",
            "edit",
        ])
        .env("FORGE_DIR", daemon.forge_dir())
        .output()
        .expect("check");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Safe to proceed"),
        "unrelated file should be safe: {stdout}"
    );

    // 4. Clean up
    if !id.is_empty() {
        let _ = Command::new(forge_next())
            .args(["forget", id])
            .env("FORGE_DIR", daemon.forge_dir())
            .output();
    }
}
