use crate::render::colors::*;
use crate::state::HudState;

/// Line 2: Forge status — version, session, memory, security, agents, tokens, skills.
/// Only sections with real data are shown; empty sections are omitted entirely.
pub fn render_line2(state: &HudState, width: usize) -> String {
    let sep = format!(" {DIM}|{RESET} ");

    // Version is always shown
    let mut parts: Vec<String> = vec![render_version(&state.version)];

    // Session: only if mode is set
    if state.session.mode.is_some() {
        parts.push(render_session(&state.session));
    }

    // Memory: only if decisions + patterns + lessons > 0
    let mem_total = state.memory.decisions + state.memory.patterns + state.memory.lessons;
    if mem_total > 0 {
        parts.push(render_memory(&state.memory));
    }

    // Security: always shown (even "✓ secure" is useful)
    parts.push(render_security(&state.security));

    // Team: only if non-empty
    if !state.team.is_empty() {
        parts.push(render_team(&state.team));
    }

    // Tokens: only if non-zero AND width >= 140
    if width >= 140 && (state.tokens.input > 0 || state.tokens.output > 0) {
        parts.push(render_tokens(&state.tokens));
    }

    // Skills: only if non-zero AND width >= 180
    if width >= 180 && state.skills.active > 0 {
        parts.push(render_skills(&state.skills));
    }

    parts.join(&sep)
}

fn render_version(v: &Option<String>) -> String {
    let ver = v.as_deref().unwrap_or(env!("CARGO_PKG_VERSION"));
    format!("{BOLD}{GREEN}Forge v{}{RESET}", sanitize(ver))
}

fn render_session(s: &crate::state::SessionInfo) -> String {
    match (&s.mode, &s.phase, &s.wave) {
        (Some(m), Some(p), Some(w)) => {
            format!("{BOLD}{MAGENTA}{}{RESET} {DIM}\u{203a}{RESET} {MAGENTA}{}{RESET} {DIM}[wave {CYAN}{}{RESET}{DIM}]{RESET}",
                sanitize(m), sanitize(p), sanitize(w))
        }
        (Some(m), Some(p), None) => {
            format!("{BOLD}{MAGENTA}{}{RESET} {DIM}\u{203a}{RESET} {MAGENTA}{}{RESET}",
                sanitize(m), sanitize(p))
        }
        (Some(m), None, _) => format!("{MAGENTA}{}{RESET}", sanitize(m)),
        _ => String::new(),
    }
}

fn render_memory(m: &crate::state::MemoryStats) -> String {
    let mut parts = Vec::new();
    if m.decisions > 0 { parts.push(format!("{} decision{}", m.decisions, plural(m.decisions))); }
    if m.patterns > 0 { parts.push(format!("{} pattern{}", m.patterns, plural(m.patterns))); }
    if m.lessons > 0 { parts.push(format!("{} lesson{}", m.lessons, plural(m.lessons))); }
    format!("{BLUE}{}{RESET}", parts.join(" \u{00b7} "))
}

fn render_security(s: &crate::state::SecurityStats) -> String {
    let c = crate::render::colors::security_color(s.stale, s.exposed);
    if s.total == 0 {
        return format!("{GREEN}\u{2713} secure{RESET}");
    }
    if s.exposed > 0 {
        format!("{c}\u{26a0} {} secret{} ({} exposed){RESET}", s.total, plural(s.total), s.exposed)
    } else if s.stale > 0 {
        format!("{c}\u{26a0} {} secret{} ({} stale){RESET}", s.total, plural(s.total), s.stale)
    } else {
        format!("{c}{} secret{}{RESET}", s.total, plural(s.total))
    }
}

fn render_team(team: &std::collections::HashMap<String, crate::state::AgentInfo>) -> String {
    let mut parts = Vec::new();
    // Sort agents by key for consistent display
    let mut entries: Vec<_> = team.iter().collect();
    entries.sort_by_key(|(k, _)| (*k).clone());

    for (agent_id, info) in entries {
        // Display name: prefer agent_type, strip "forge-" prefix; fallback to agent_id
        let display_name = info
            .agent_type
            .as_deref()
            .unwrap_or(agent_id);
        let short = sanitize(display_name.strip_prefix("forge-").unwrap_or(display_name));

        let (icon, color) = match info.status.as_deref() {
            Some("done") => ("\u{2713}", GREEN),     // checkmark
            Some("running") => ("\u{25b6}", YELLOW),  // play
            Some("pending") => ("\u{23f3}", DIM),     // hourglass
            Some("blocked") => ("\u{2717}", RED),     // x
            Some("stale") => ("\u{26a0}", RED),       // warning
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
    parts.join("  ")
}

fn render_tokens(t: &crate::state::TokenStats) -> String {
    let rc = ratio_color(t.deterministic_ratio);
    let pct = (t.deterministic_ratio * 100.0) as u64;
    format!("{rc}{}K in \u{00b7} {}K out \u{00b7} {pct}% deterministic{RESET}", t.input / 1000, t.output / 1000)
}

fn render_skills(s: &crate::state::SkillStats) -> String {
    if s.fix_candidates > 0 {
        format!("{} skill{} {YELLOW}({} need{} fix){RESET}",
            s.active, plural(s.active), s.fix_candidates, if s.fix_candidates == 1 { "s" } else { "" })
    } else {
        format!("{} skill{}", s.active, plural(s.active))
    }
}

fn plural(n: u64) -> &'static str {
    if n == 1 { "" } else { "s" }
}
