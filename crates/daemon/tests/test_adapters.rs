use forge_daemon::adapters;

#[test]
fn test_detect_adapters_finds_claude() {
    let adapters = adapters::detect_adapters();
    let names: Vec<&str> = adapters.iter().map(|a| a.name()).collect();
    // On this machine, ~/.claude/projects/ exists (we're running in Claude Code)
    assert!(
        names.contains(&"claude-code"),
        "expected claude-code adapter, got: {names:?}"
    );
}

#[test]
fn test_adapter_for_path_routes_correctly() {
    let home = std::env::var("HOME").unwrap_or_default();
    let adapters = adapters::detect_adapters();

    // Claude path should route to claude-code adapter
    let claude_path = format!("{home}/.claude/projects/test/session.jsonl");
    let adapter = adapters::adapter_for_path(&adapters, std::path::Path::new(&claude_path));
    assert!(adapter.is_some(), "should find adapter for Claude path");
    assert_eq!(adapter.unwrap().name(), "claude-code");

    // Unknown path should return None
    let unknown = adapters::adapter_for_path(&adapters, std::path::Path::new("/tmp/random.txt"));
    assert!(
        unknown.is_none(),
        "should not find adapter for unknown path"
    );
}

#[test]
fn test_codex_adapter_detected_if_installed() {
    let adapters = adapters::detect_adapters();
    let names: Vec<&str> = adapters.iter().map(|a| a.name()).collect();
    // On this machine, ~/.codex/sessions/ exists (Codex CLI is installed)
    if std::path::Path::new(&format!(
        "{}/.codex/sessions",
        std::env::var("HOME").unwrap_or_default()
    ))
    .exists()
    {
        assert!(
            names.contains(&"codex"),
            "expected codex adapter when ~/.codex/sessions exists, got: {names:?}"
        );
    }
}

#[test]
fn test_adapter_for_path_codex() {
    let home = std::env::var("HOME").unwrap_or_default();
    let adapters = adapters::detect_adapters();

    let codex_path = format!("{home}/.codex/sessions/2026/04/03/rollout-abc.jsonl");
    let adapter = adapters::adapter_for_path(&adapters, std::path::Path::new(&codex_path));
    if std::path::Path::new(&format!("{home}/.codex/sessions")).exists() {
        assert!(adapter.is_some());
        assert_eq!(adapter.unwrap().name(), "codex");
    }
}
