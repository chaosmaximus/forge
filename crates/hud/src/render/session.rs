use crate::render::colors::*;
use crate::state::HudState;
use crate::stdin::StdinData;

/// Line 3: Forge version, security status, k8s context, pwd, memory, agents
///   Forge v0.4.0 │ ✓ secure │ ⎈ gke_prod │ 📂 forge │ 42 decisions
pub fn render_line3(stdin: &StdinData, state: &HudState, _width: usize) -> String {
    let sep = format!(" {DIM}\u{2502}{RESET} "); // │

    // Determine which sections to show (from config, or all by default)
    let show_sections: Vec<String> = state
        .hud_config
        .as_ref()
        .filter(|c| !c.sections.is_empty())
        .map(|c| c.sections.clone())
        .unwrap_or_else(|| {
            vec![
                "memory".into(),
                "health".into(),
                "agents".into(),
                "k8s".into(),
                "pwd".into(),
                "git".into(),
                "security".into(),
                "tasks".into(),
            ]
        });

    let section_enabled = |name: &str| show_sections.iter().any(|s| s == name);

    // Line 3: CWD + project memory stats + agents/tasks
    // Version, secure, k8s moved to line 1 to avoid redundancy
    let mut parts: Vec<String> = Vec::new();

    // Security only shows when there's a problem (exposed secrets)
    if section_enabled("security") && state.security.exposed > 0 {
        parts.push(render_security(&state.security));
    }

    if section_enabled("pwd") {
        // Use session CWD from stdin (current Claude Code session), not daemon state
        let session_cwd = stdin.cwd.as_deref().or(state.cwd.as_deref());
        if let Some(pwd_str) = render_pwd(&session_cwd.map(String::from)) {
            parts.push(pwd_str);
        }
    }

    if section_enabled("agents") && !state.team.is_empty() {
        parts.push(render_team(&state.team));
    } else if section_enabled("memory") && state.team.is_empty() {
        // Use project-scoped stats if available, else fall back to global
        let project = stdin.project_name();
        let mem = if !project.is_empty() {
            render_memory_project(state, &project)
        } else {
            render_memory_fallback(&state.memory)
        };
        if !mem.is_empty() {
            parts.push(mem);
        }
    }

    if section_enabled("tasks") {
        if let Some(task_str) = state.tasks.as_ref().and_then(render_tasks) {
            parts.push(task_str);
        }
    }

    // Phase 2A-4d.2 T6: consolidation summary of the latest pass. Lowest
    // priority — omitted when no pass has fired yet (state.consolidation =
    // None) OR when the cached value is stale (handled by the daemon
    // staleness guard before the key is even written).
    if let Some(cons_str) = state.consolidation.as_ref().and_then(render_consolidation) {
        parts.push(cons_str);
    }

    // Show active sessions count (other projects = interesting, same project = context)
    if !state.sessions.is_empty() {
        let current_project = stdin.project_name();
        let other_count = state
            .sessions
            .iter()
            .filter(|s| !s.project.is_empty() && s.project != current_project)
            .count();
        let same_count = state
            .sessions
            .iter()
            .filter(|s| {
                s.project == current_project || (current_project.is_empty() && s.project.is_empty())
            })
            .count();

        let mut session_parts = Vec::new();
        if same_count > 0 {
            session_parts.push(format!("{same_count} here"));
        }
        if other_count > 0 {
            // Show which other projects have active sessions
            let mut other_projects: Vec<&str> = state
                .sessions
                .iter()
                .filter(|s| !s.project.is_empty() && s.project != current_project)
                .map(|s| s.project.as_str())
                .collect();
            other_projects.sort();
            other_projects.dedup();
            let names: String = other_projects
                .iter()
                .take(3)
                .copied()
                .collect::<Vec<_>>()
                .join(", ");
            let extra = if other_projects.len() > 3 {
                format!("+{}", other_projects.len() - 3)
            } else {
                String::new()
            };
            session_parts.push(format!("{other_count} in {names}{extra}"));
        }
        if !session_parts.is_empty() {
            parts.push(format!(
                "{DIM}\u{1F4E1} {}{RESET}",
                session_parts.join(", ")
            )); // 📡
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
        let display_name = info.agent_type.as_deref().unwrap_or(agent_id);
        let short = sanitize(display_name.strip_prefix("forge-").unwrap_or(display_name));

        let (icon, color) = match info.status.as_deref() {
            Some("done") => ("\u{2713}", GREEN),     // ✓
            Some("running") => ("\u{25d0}", YELLOW), // ◐
            Some("pending") => ("\u{23f3}", DIM),    // ⏳
            Some("blocked") => ("\u{2717}", RED),    // ✗
            Some("stale") => ("\u{26a0}", RED),      // ⚠
            _ => ("?", DIM),
        };
        let tool_info = if info.status.as_deref() == Some("running") {
            info.last_tool
                .as_ref()
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

fn render_pwd(cwd: &Option<String>) -> Option<String> {
    let path = cwd.as_ref()?;
    if path.is_empty() {
        return None;
    }
    // Show last 2 path components for brevity (e.g. "DurgaSaiK/forge")
    let components: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let short = if components.len() > 2 {
        components[components.len() - 2..].join("/")
    } else {
        components.join("/")
    };
    Some(format!("{DIM}\u{1F4C2} {}{RESET}", sanitize(&short))) // 📂
}

fn render_k8s(k8s: &Option<crate::state::K8sContext>) -> Option<String> {
    let ctx = k8s.as_ref()?;
    let name = ctx.context.as_ref()?;
    if name.is_empty() {
        return None;
    }
    // Shorten common GKE/EKS prefixes for compact display
    let short = name
        .strip_prefix("gke_")
        .or_else(|| name.strip_prefix("arn:aws:eks:"))
        .unwrap_or(name);
    let short = sanitize(short);
    let ns = ctx
        .namespace
        .as_ref()
        .filter(|n| !n.is_empty() && *n != "default")
        .map(|n| format!("/{}", sanitize(n)))
        .unwrap_or_default();
    Some(format!("{CYAN}\u{2388} {short}{ns}{RESET}")) // ⎈
}

/// Render memory stats for a specific project (from per-project breakdown).
/// Matches by exact name, then by path suffix (project names may be stored as
/// `-mnt-colab-disk-User-project` while we search for `project`).
/// Falls back to global stats if no match.
fn render_memory_project(state: &HudState, project: &str) -> String {
    // Exact match first
    if let Some(proj_stats) = state.projects.get(project) {
        return render_memory_fallback(proj_stats);
    }
    // Suffix match: find project names ending with the basename
    let suffix = format!("-{project}");
    for (name, stats) in &state.projects {
        if name.ends_with(&suffix) || name.ends_with(project) {
            return render_memory_fallback(stats);
        }
    }
    // Aggregate all projects containing the name in their path
    let mut agg = crate::state::MemoryStats::default();
    let lower = project.to_lowercase().replace('-', "_");
    for (name, stats) in &state.projects {
        let normalized = name.to_lowercase().replace('-', "_");
        if normalized.contains(&lower) {
            agg.decisions += stats.decisions;
            agg.lessons += stats.lessons;
            agg.patterns += stats.patterns;
        }
    }
    if agg.decisions + agg.lessons + agg.patterns > 0 {
        return render_memory_fallback(&agg);
    }
    render_memory_fallback(&state.memory)
}

fn render_memory_fallback(m: &crate::state::MemoryStats) -> String {
    let mut parts = Vec::new();
    if m.decisions > 0 {
        parts.push(format!(
            "{} decision{}",
            m.decisions,
            if m.decisions == 1 { "" } else { "s" }
        ));
    }
    if m.patterns > 0 {
        parts.push(format!(
            "{} pattern{}",
            m.patterns,
            if m.patterns == 1 { "" } else { "s" }
        ));
    }
    if m.lessons > 0 {
        parts.push(format!(
            "{} lesson{}",
            m.lessons,
            if m.lessons == 1 { "" } else { "s" }
        ));
    }
    if parts.is_empty() {
        return String::new();
    }
    format!("{BLUE}{}{RESET}", parts.join(" \u{00b7} "))
}

/// Phase 2A-4d.2 T6: render the consolidation segment.
///   `cons:23✓ 1.2s`       — latest pass had zero errors (green)
///   `cons:23 ⚠3e 3.4s`    — latest pass had N errors across its phases (red)
///
/// Note: `error_count` is total errors across the run, not failed-phase
/// count. A previous draft rendered `ok/total err` math on these counts,
/// which was nonsense (T9 Codex Q9). We now render the raw error count.
/// Returns `None` when required fields are missing.
fn render_consolidation(cons: &crate::state::ConsolidationStats) -> Option<String> {
    let phase_count = cons.latest_run_phase_count?;
    let dur_ms = cons.latest_run_wall_duration_ms?;
    let errors = cons.latest_run_error_count.unwrap_or(0);
    let dur_s = dur_ms as f64 / 1000.0;

    if errors == 0 {
        Some(format!(
            "{GREEN}cons:{phase_count}\u{2713} {dur_s:.1}s{RESET}"
        ))
    } else {
        Some(format!(
            "{RED}cons:{phase_count} \u{26a0}{errors}e {dur_s:.1}s{RESET}"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::ConsolidationStats;

    fn cons_with(
        phase_count: Option<u64>,
        duration_ms: Option<u64>,
        errors: Option<u64>,
    ) -> ConsolidationStats {
        ConsolidationStats {
            latest_run_id: Some("01HX".into()),
            latest_run_ts_secs: Some(1),
            latest_run_wall_duration_ms: duration_ms,
            latest_run_error_count: errors,
            latest_run_phase_count: phase_count,
            latest_run_trace_id: None,
            rolling_24h_pass_count: Some(0),
            rolling_24h_error_passes: Some(0),
        }
    }

    #[test]
    fn render_consolidation_green_when_no_errors() {
        let c = cons_with(Some(23), Some(1234), Some(0));
        let out = render_consolidation(&c).expect("segment");
        assert!(out.contains("cons:23\u{2713} 1.2s"), "got: {out}");
    }

    #[test]
    fn render_consolidation_red_when_errors_present() {
        let c = cons_with(Some(23), Some(3400), Some(2));
        let out = render_consolidation(&c).expect("segment");
        // error_count is total errors (not failed-phase count), so render as
        // "cons:23 ⚠2e 3.4s" — phase_count stays as-is, errors shown raw.
        assert!(out.contains("cons:23"), "got: {out}");
        assert!(out.contains("\u{26a0}2e"), "got: {out}");
        assert!(out.contains("3.4s"), "got: {out}");
    }

    #[test]
    fn render_consolidation_absent_when_missing_fields() {
        let c = cons_with(None, Some(1000), Some(0));
        assert!(render_consolidation(&c).is_none());
        let c = cons_with(Some(23), None, Some(0));
        assert!(render_consolidation(&c).is_none());
    }
}
