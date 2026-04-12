/// E2E tests for Forge hooks — validates the full pipeline:
/// Claude Code hook → shell script → forge-next CLI → daemon socket → response
///
/// These tests require the release binary to be built and the daemon to be available.

use std::process::Command;

fn forge_next() -> String {
    let candidates = [
        "target/release/forge-next",
        "../target/release/forge-next",
        "../../target/release/forge-next",
    ];
    for c in &candidates {
        if std::path::Path::new(c).exists() {
            return c.to_string();
        }
    }
    "forge-next".to_string()
}

fn hooks_dir() -> String {
    let candidates = [
        "scripts/hooks",
        "../scripts/hooks",
        "../../scripts/hooks",
    ];
    for c in &candidates {
        if std::path::Path::new(c).exists() {
            return c.to_string();
        }
    }
    "scripts/hooks".to_string()
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
    let session_id = format!("hook-test-{}", std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs());

    // Run the session start hook
    let _ = Command::new("bash")
        .arg(format!("{}/session-start.sh", hooks_dir()))
        .env("CLAUDE_CWD", "/mnt/colab-disk/DurgaSaiK/forge")
        .env("CLAUDE_SESSION_ID", &session_id)
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run session-start hook");

    // Verify session was registered
    let output = Command::new(forge_next())
        .args(["sessions", "--all"])
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

    assert_eq!(output.status.code(), Some(0), "normal file should be allowed");
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

        assert_eq!(
            output.status.code(),
            Some(2),
            "{file} should be blocked"
        );
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
    // 1. Store a decision that mentions a file path in content.
    //    Since Session 12, the remember handler auto-creates affects edges
    //    for file paths matching (crates|src|lib|app)/.../*.rs patterns.
    let output = Command::new(forge_next())
        .args([
            "remember",
            "--type", "decision",
            "--title", "Hook E2E test decision",
            "--content", "This decision is linked to src/hook_test.rs for testing",
            "--project", "forge",
        ])
        .output()
        .expect("remember");
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    let id = stdout.split("Stored: ").nth(1).unwrap_or("").trim();

    // 2. Guardrails check on the mentioned file → should find the linked decision
    //    (affects edge was auto-created by the remember handler)
    let output = Command::new(forge_next())
        .args(["check", "--file", "src/hook_test.rs", "--action", "edit"])
        .output()
        .expect("check");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("decision") || stdout.contains("linked"),
        "file mentioned in decision content should have affects edge: {stdout}"
    );

    // 3. Guardrails check on a file NOT mentioned → should be safe
    let output = Command::new(forge_next())
        .args(["check", "--file", "src/unrelated_file_xyz.rs", "--action", "edit"])
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
            .output();
    }
}
