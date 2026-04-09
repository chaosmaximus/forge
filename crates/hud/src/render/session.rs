use crate::render::colors::*;
use crate::state::HudState;

/// Line 3: Forge version, security status, agent team, k8s context
///   Forge v0.3.0 │ ✓ secure │ ⎈ gke_prod │ ◐ planner (Bash)
pub fn render_line3(state: &HudState, _width: usize) -> String {
    let sep = format!(" {DIM}\u{2502}{RESET} "); // │

    // Determine which sections to show (from config, or all by default)
    let show_sections: Vec<String> = state.hud_config.as_ref()
        .filter(|c| !c.sections.is_empty())
        .map(|c| c.sections.clone())
        .unwrap_or_else(|| vec![
            "memory".into(), "health".into(), "agents".into(),
            "k8s".into(), "git".into(), "security".into(), "tasks".into(),
        ]);

    let section_enabled = |name: &str| show_sections.iter().any(|s| s == name);

    let version = render_version(&state.version);
    let mut parts: Vec<String> = vec![version];

    if section_enabled("security") {
        parts.push(render_security(&state.security));
    }

    if section_enabled("k8s") {
        if let Some(k8s_str) = render_k8s(&state.k8s) {
            parts.push(k8s_str);
        }
    }

    if section_enabled("agents") && !state.team.is_empty() {
        parts.push(render_team(&state.team));
    } else if section_enabled("memory") && state.team.is_empty() {
        let mem = render_memory_fallback(&state.memory);
        if !mem.is_empty() {
            parts.push(mem);
        }
    }

    if section_enabled("tasks") {
        if let Some(task_str) = state.tasks.as_ref().and_then(render_tasks) {
            parts.push(task_str);
        }
    }

    format!("  {}", parts.join(&sep))
}

fn render_version(v: &Option<String>) -> String {
    let ver = v.as_deref().unwrap_or(env!("CARGO_PKG_VERSION"));
    format!("{BOLD}{GREEN}Forge v{}{RESET}", sanitize(ver))
}

fn render_security(s: &crate::state::SecurityStats) -> String {
    if s.exposed > 0 {
        format!("{RED}\u{26a0} {} exposed{RESET}", s.exposed)
    } else if s.stale > 0 {
        format!("{YELLOW}\u{26a0} {} stale{RESET}", s.stale)
    } else {
        format!("{GREEN}\u{2713} secure{RESET}")
    }
}

fn render_team(team: &std::collections::HashMap<String, crate::state::AgentInfo>) -> String {
    let mut entries: Vec<_> = team.iter().collect();
    entries.sort_by_key(|(k, _)| (*k).clone());

    let mut parts = Vec::new();
    for (agent_id, info) in entries {
        let display_name = info
            .agent_type
            .as_deref()
            .unwrap_or(agent_id);
        let short = sanitize(display_name.strip_prefix("forge-").unwrap_or(display_name));

        let (icon, color) = match info.status.as_deref() {
            Some("done") => ("\u{2713}", GREEN),       // ✓
            Some("running") => ("\u{25d0}", YELLOW),    // ◐
            Some("pending") => ("\u{23f3}", DIM),       // ⏳
            Some("blocked") => ("\u{2717}", RED),       // ✗
            Some("stale") => ("\u{26a0}", RED),         // ⚠
            _ => ("?", DIM),
        };
        let tool_info = if info.status.as_deref() == Some("running") {
            info.last_tool.as_ref()
                .map(|t| format!(" {DIM}({t}){RESET}"))
                .unwrap_or_default()
        } else {
            String::new()
        };
        parts.push(format!("{color}{icon}{RESET} {short}{tool_info}"));
    }
    parts.join(" ")
}

fn render_tasks(t: &crate::state::TaskStats) -> Option<String> {
    // Only render if there's an in-progress task
    let subject = t.in_progress.as_ref()?;
    let truncated = if subject.chars().count() > 40 {
        let s: String = subject.chars().take(40).collect();
        format!("{s}...")
    } else {
        subject.clone()
    };
    let truncated = sanitize(&truncated);
    Some(format!(
        "{YELLOW}\u{25b8}{RESET} {truncated} {DIM}({}/{}){RESET}",
        t.completed, t.total
    ))
}

fn render_k8s(k8s: &Option<crate::state::K8sContext>) -> Option<String> {
    let ctx = k8s.as_ref()?;
    let name = ctx.context.as_ref()?;
    if name.is_empty() {
        return None;
    }
    // Shorten common GKE/EKS prefixes for compact display
    let short = name
        .strip_prefix("gke_").or_else(|| name.strip_prefix("arn:aws:eks:"))
        .unwrap_or(name);
    let short = sanitize(short);
    let ns = ctx.namespace.as_ref()
        .filter(|n| !n.is_empty() && *n != "default")
        .map(|n| format!("/{}", sanitize(n)))
        .unwrap_or_default();
    Some(format!("{CYAN}\u{2388} {short}{ns}{RESET}")) // ⎈
}

fn render_memory_fallback(m: &crate::state::MemoryStats) -> String {
    let mut parts = Vec::new();
    if m.decisions > 0 {
        parts.push(format!("{} decision{}", m.decisions, if m.decisions == 1 { "" } else { "s" }));
    }
    if m.patterns > 0 {
        parts.push(format!("{} pattern{}", m.patterns, if m.patterns == 1 { "" } else { "s" }));
    }
    if m.lessons > 0 {
        parts.push(format!("{} lesson{}", m.lessons, if m.lessons == 1 { "" } else { "s" }));
    }
    if parts.is_empty() {
        return String::new();
    }
    format!("{BLUE}{}{RESET}", parts.join(" \u{00b7} "))
}
