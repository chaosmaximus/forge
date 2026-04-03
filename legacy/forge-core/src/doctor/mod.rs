//! `forge doctor` -- 13 system health checks.
//!
//! Verifies the entire Forge installation is working: binaries, venv,
//! graph DB, HUD, hooks, scripts, skills, memory, dependencies.
//! JSON output by default, `--format text` for human-readable.

use serde::Serialize;
use serde_json::json;
use std::path::Path;
use std::process::Command;

#[derive(Serialize)]
pub struct Check {
    pub name: String,
    pub status: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

fn ok(name: &str, msg: &str) -> Check {
    Check { name: name.into(), status: "ok".into(), message: msg.into(), detail: None }
}
fn warn(name: &str, msg: &str) -> Check {
    Check { name: name.into(), status: "warn".into(), message: msg.into(), detail: None }
}
fn warn_detail(name: &str, msg: &str, detail: &str) -> Check {
    Check { name: name.into(), status: "warn".into(), message: msg.into(), detail: Some(detail.into()) }
}
fn error(name: &str, msg: &str) -> Check {
    Check { name: name.into(), status: "error".into(), message: msg.into(), detail: None }
}
fn error_detail(name: &str, msg: &str, detail: &str) -> Check {
    Check { name: name.into(), status: "error".into(), message: msg.into(), detail: Some(detail.into()) }
}

pub fn run(state_dir: &str, format: &str) {
    let checks = vec![
        check_binary(),
        check_python_venv(state_dir),
        check_graph_db(state_dir),
        check_hud_state(state_dir),
        check_agent_tracking(state_dir),
        check_hooks(),
        check_scripts(),
        check_skills(),
        check_memory_cache(state_dir),
        check_dependencies(),
        check_codex(),
        check_plugin_cache(),
        check_security(state_dir),
    ];

    if format == "text" {
        for c in &checks {
            let icon = match c.status.as_str() {
                "ok" => "\u{2713}",
                "warn" => "\u{26a0}",
                _ => "\u{2717}",
            };
            print!("{} {} -- {}", icon, c.name, c.message);
            if let Some(d) = &c.detail {
                print!(" ({})", d);
            }
            println!();
        }
        let errors = checks.iter().filter(|c| c.status == "error").count();
        let warns = checks.iter().filter(|c| c.status == "warn").count();
        let oks = checks.iter().filter(|c| c.status == "ok").count();
        println!();
        if errors > 0 {
            println!("{} error(s), {} warning(s), {} ok", errors, warns, oks);
        } else if warns > 0 {
            println!("{} ok, {} warning(s)", oks, warns);
        } else {
            println!("All {} checks passed", oks);
        }
    } else {
        let summary = json!({
            "checks": checks,
            "summary": {
                "ok": checks.iter().filter(|c| c.status == "ok").count(),
                "warn": checks.iter().filter(|c| c.status == "warn").count(),
                "error": checks.iter().filter(|c| c.status == "error").count(),
            }
        });
        println!("{}", serde_json::to_string_pretty(&summary).unwrap_or_default());
    }
}

// ---- Individual checks ----

fn check_binary() -> Check {
    let version = env!("CARGO_PKG_VERSION");
    ok("binary", &format!("forge v{}", version))
}

fn check_python_venv(state_dir: &str) -> Check {
    let plugin_root = std::env::var("CLAUDE_PLUGIN_ROOT").unwrap_or_default();
    let home = std::env::var("HOME").unwrap_or_default();

    let candidates = [
        format!("{}/forge-graph/.venv/bin/python", plugin_root),
        format!("{}/../../forge-graph/.venv/bin/python", state_dir),
        format!(
            "{}/.claude/plugins/cache/forge-marketplace/forge/0.3.0/forge-graph/.venv/bin/python",
            home
        ),
        format!(
            "{}/.claude/plugins/cache/forge-marketplace/forge/0.2.0/forge-graph/.venv/bin/python",
            home
        ),
        "forge-graph/.venv/bin/python".to_string(),
    ];

    for c in &candidates {
        if !c.is_empty() && Path::new(c).exists() {
            return ok("python_venv", &format!("found at {}", c));
        }
    }

    warn_detail(
        "python_venv",
        "no venv found; will fall back to system python3",
        "Searched plugin root, state dir, and cache paths",
    )
}

fn check_graph_db(state_dir: &str) -> Check {
    let db_path = Path::new(state_dir).join("graph").join("forge.lbdb");
    if db_path.exists() {
        let size = std::fs::metadata(&db_path).map(|m| m.len()).unwrap_or(0);
        ok("graph_db", &format!("database at {} ({}KB)", db_path.display(), size / 1024))
    } else {
        warn_detail(
            "graph_db",
            &format!("database file missing at {}", db_path.display()),
            "Run `forge sync` or `forge health` to initialize",
        )
    }
}

fn check_hud_state(state_dir: &str) -> Check {
    let path = Path::new(state_dir).join("hud").join("hud-state.json");
    match std::fs::read_to_string(&path) {
        Ok(content) => {
            if serde_json::from_str::<serde_json::Value>(&content).is_ok() {
                ok("hud_state", "exists and parses")
            } else {
                error_detail(
                    "hud_state",
                    "exists but invalid JSON",
                    "Delete and let hooks recreate it",
                )
            }
        }
        Err(_) => warn_detail(
            "hud_state",
            &format!("missing at {}", path.display()),
            "Will be created on first hook event",
        ),
    }
}

fn check_agent_tracking(state_dir: &str) -> Check {
    let dir = Path::new(state_dir).join("agents");
    if dir.is_dir() {
        let count = std::fs::read_dir(&dir)
            .map(|d| {
                d.filter_map(|e| e.ok())
                    .filter(|e| {
                        e.path()
                            .extension()
                            .map(|x| x == "jsonl")
                            .unwrap_or(false)
                    })
                    .count()
            })
            .unwrap_or(0);
        ok("agent_tracking", &format!("{} dir exists, {} transcripts", dir.display(), count))
    } else {
        ok("agent_tracking", "agents dir will be created on first spawn")
    }
}

fn check_hooks() -> Check {
    let hook_path = Path::new("hooks").join("hooks.json");
    let plugin_root = std::env::var("CLAUDE_PLUGIN_ROOT").unwrap_or_default();
    let alt = Path::new(&plugin_root).join("hooks").join("hooks.json");

    let content = std::fs::read_to_string(&hook_path)
        .or_else(|_| std::fs::read_to_string(&alt));

    match content {
        Ok(c) => {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&c) {
                let events: Vec<String> = v
                    .get("hooks")
                    .and_then(|h| h.as_object())
                    .map(|o| o.keys().cloned().collect())
                    .unwrap_or_default();
                let expected = [
                    "SessionStart",
                    "SubagentStart",
                    "SubagentStop",
                    "PostToolUse",
                    "PreToolUse",
                    "TaskCompleted",
                    "TeammateIdle",
                    "SessionEnd",
                ];
                let missing: Vec<&str> = expected
                    .iter()
                    .filter(|e| !events.contains(&e.to_string()))
                    .copied()
                    .collect();
                if missing.is_empty() {
                    ok("hooks", &format!("{} events configured", events.len()))
                } else {
                    warn(
                        "hooks",
                        &format!("missing events: {}", missing.join(", ")),
                    )
                }
            } else {
                error("hooks", "hooks.json is malformed")
            }
        }
        Err(_) => error("hooks", "hooks.json not found"),
    }
}

fn check_scripts() -> Check {
    let scripts = [
        "forge-graph-start.sh",
        "session-end-graph.sh",
        "post-edit-enhanced.sh",
        "protect-sensitive-files.sh",
        "task-completed-gate.sh",
        "teammate-idle-checkpoint.sh",
    ];
    let plugin_root = std::env::var("CLAUDE_PLUGIN_ROOT").unwrap_or_default();
    let mut missing = Vec::new();
    let mut not_exec = Vec::new();

    for s in &scripts {
        let path = Path::new("scripts").join(s);
        let alt = Path::new(&plugin_root).join("scripts").join(s);
        let p = if path.exists() { path } else { alt };

        if !p.exists() {
            missing.push(*s);
        } else {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Ok(m) = std::fs::metadata(&p) {
                    if m.permissions().mode() & 0o111 == 0 {
                        not_exec.push(*s);
                    }
                }
            }
        }
    }

    if !missing.is_empty() {
        error("scripts", &format!("missing: {}", missing.join(", ")))
    } else if !not_exec.is_empty() {
        warn(
            "scripts",
            &format!("not executable: {}", not_exec.join(", ")),
        )
    } else {
        ok(
            "scripts",
            &format!("{} scripts present and executable", scripts.len()),
        )
    }
}

fn check_skills() -> Check {
    let plugin_root = std::env::var("CLAUDE_PLUGIN_ROOT").unwrap_or_default();
    let skills_dir = Path::new("skills");
    let alt = Path::new(&plugin_root).join("skills");
    let dir = if skills_dir.is_dir() {
        skills_dir.to_path_buf()
    } else {
        alt
    };

    let mut count = 0u32;
    if dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let skill_md = entry.path().join("SKILL.md");
                if skill_md.exists() {
                    count += 1;
                }
            }
        }
    }
    if count > 0 {
        ok("skills", &format!("{} SKILL.md files found", count))
    } else {
        warn("skills", "no skills found")
    }
}

fn check_memory_cache(state_dir: &str) -> Check {
    let cache = Path::new(state_dir).join("memory").join("cache.json");
    let pending = Path::new(state_dir).join("memory").join("pending.jsonl");

    let cache_count = if cache.exists() {
        std::fs::read_to_string(&cache)
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|v| v.get("entries")?.as_array().map(|a| a.len()))
            .unwrap_or(0)
    } else {
        0
    };

    let pending_count = if pending.exists() {
        std::fs::read_to_string(&pending)
            .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count())
            .unwrap_or(0)
    } else {
        0
    };

    if cache_count == 0 && pending_count == 0 {
        warn_detail(
            "memory_cache",
            "empty -- no memories stored yet",
            "Run `forge remember` to create the first entry",
        )
    } else if pending_count > 0 {
        warn(
            "memory_cache",
            &format!("{} cached, {} entries pending sync", cache_count, pending_count),
        )
    } else {
        ok(
            "memory_cache",
            &format!("{} memories cached", cache_count),
        )
    }
}

fn check_dependencies() -> Check {
    let deps = [("python3", "--version"), ("jq", "--version"), ("node", "--version")];
    let mut found = Vec::new();
    let mut missing = Vec::new();

    for (name, flag) in &deps {
        if Command::new(name)
            .arg(flag)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            found.push(*name);
        } else {
            missing.push(*name);
        }
    }

    if missing.is_empty() {
        ok("dependencies", &format!("{} available", found.join(", ")))
    } else {
        warn(
            "dependencies",
            &format!("missing: {}", missing.join(", ")),
        )
    }
}

fn check_codex() -> Check {
    if Command::new("codex")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        ok("codex", "available")
    } else {
        warn_detail(
            "codex",
            "not found -- adversarial review unavailable",
            "Install: npm install -g @openai/codex",
        )
    }
}

fn check_plugin_cache() -> Check {
    let home = std::env::var("HOME").unwrap_or_default();
    let v03 = format!(
        "{}/.claude/plugins/cache/forge-marketplace/forge/0.3.0",
        home
    );
    let v02 = format!(
        "{}/.claude/plugins/cache/forge-marketplace/forge/0.2.0",
        home
    );

    if Path::new(&v03).is_dir() {
        ok("plugin_cache", "v0.3.0 cache found")
    } else if Path::new(&v02).is_dir() {
        warn_detail(
            "plugin_cache",
            "v0.2.0 cache found -- consider upgrading",
            "Reinstall plugin to update cache",
        )
    } else {
        warn("plugin_cache", "no plugin cache found")
    }
}

fn check_security(state_dir: &str) -> Check {
    let sec = crate::hud_state::read(state_dir).security;
    if sec.exposed > 0 {
        error_detail(
            "security",
            &format!("{} exposed secrets detected", sec.exposed),
            "Run `forge scan .` for details",
        )
    } else if sec.stale > 0 {
        warn(
            "security",
            &format!("{} secrets need rotation", sec.stale),
        )
    } else {
        ok("security", "no exposed secrets")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_binary() {
        let c = check_binary();
        assert_eq!(c.status, "ok");
        assert!(c.message.contains("forge v"));
    }

    #[test]
    fn test_check_graph_db_missing() {
        let c = check_graph_db("/nonexistent/path");
        assert_eq!(c.status, "warn");
        assert!(c.message.contains("missing"));
    }

    #[test]
    fn test_check_hud_state_missing() {
        let c = check_hud_state("/nonexistent/path");
        assert_eq!(c.status, "warn");
    }

    #[test]
    fn test_check_memory_cache_missing() {
        let c = check_memory_cache("/nonexistent/path");
        assert_eq!(c.status, "warn");
    }

    #[test]
    fn test_check_hud_state_valid() {
        let dir = tempfile::tempdir().unwrap();
        let hud_dir = dir.path().join("hud");
        std::fs::create_dir_all(&hud_dir).unwrap();
        std::fs::write(
            hud_dir.join("hud-state.json"),
            r#"{"version":"0.3.0"}"#,
        )
        .unwrap();

        let c = check_hud_state(dir.path().to_str().unwrap());
        assert_eq!(c.status, "ok");
    }

    #[test]
    fn test_check_hud_state_invalid_json() {
        let dir = tempfile::tempdir().unwrap();
        let hud_dir = dir.path().join("hud");
        std::fs::create_dir_all(&hud_dir).unwrap();
        std::fs::write(hud_dir.join("hud-state.json"), "not json").unwrap();

        let c = check_hud_state(dir.path().to_str().unwrap());
        assert_eq!(c.status, "error");
    }

    #[test]
    fn test_check_memory_cache_with_entries() {
        let dir = tempfile::tempdir().unwrap();
        let mem_dir = dir.path().join("memory");
        std::fs::create_dir_all(&mem_dir).unwrap();
        std::fs::write(
            mem_dir.join("cache.json"),
            r#"{"entries":[{"type":"decision","status":"active","title":"test"}]}"#,
        )
        .unwrap();

        let c = check_memory_cache(dir.path().to_str().unwrap());
        assert_eq!(c.status, "ok");
        assert!(c.message.contains("1 memories"));
    }

    #[test]
    fn test_check_memory_cache_with_pending() {
        let dir = tempfile::tempdir().unwrap();
        let mem_dir = dir.path().join("memory");
        std::fs::create_dir_all(&mem_dir).unwrap();
        std::fs::write(
            mem_dir.join("cache.json"),
            r#"{"entries":[{"type":"decision","status":"active"}]}"#,
        )
        .unwrap();
        std::fs::write(mem_dir.join("pending.jsonl"), "{\"id\":\"1\"}\n{\"id\":\"2\"}\n")
            .unwrap();

        let c = check_memory_cache(dir.path().to_str().unwrap());
        assert_eq!(c.status, "warn");
        assert!(c.message.contains("pending"));
    }

    #[test]
    fn test_check_security_exposed() {
        let dir = tempfile::tempdir().unwrap();
        let hud_dir = dir.path().join("hud");
        std::fs::create_dir_all(&hud_dir).unwrap();
        std::fs::write(
            hud_dir.join("hud-state.json"),
            r#"{"security":{"total":5,"stale":0,"exposed":2}}"#,
        )
        .unwrap();

        let c = check_security(dir.path().to_str().unwrap());
        assert_eq!(c.status, "error");
        assert!(c.message.contains("2 exposed"));
    }

    #[test]
    fn test_check_security_clean() {
        let dir = tempfile::tempdir().unwrap();
        let c = check_security(dir.path().to_str().unwrap());
        assert_eq!(c.status, "ok");
    }

    #[test]
    fn test_run_json_format() {
        let dir = tempfile::tempdir().unwrap();
        run(dir.path().to_str().unwrap(), "json");
    }

    #[test]
    fn test_run_text_format() {
        let dir = tempfile::tempdir().unwrap();
        run(dir.path().to_str().unwrap(), "text");
    }
}
