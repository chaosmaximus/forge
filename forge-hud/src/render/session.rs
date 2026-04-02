use crate::render::colors::*;
use crate::state::HudState;

/// Line 2: Forge status — session, memory, security, agents, tokens
pub fn render_line2(state: &HudState, width: usize) -> String {
    let session = render_session(&state.session);
    let mem = render_memory(&state.memory);
    let sec = render_security(&state.security);
    let team = render_team(&state.team);
    let tokens = render_tokens(&state.tokens);
    let skills = render_skills(&state.skills);

    // Progressive disclosure based on terminal width
    if width >= 180 {
        format!("{session} {DIM}|{RESET} {mem} {DIM}|{RESET} {sec} {DIM}|{RESET} {team} {DIM}|{RESET} {tokens} {DIM}|{RESET} {skills}")
    } else if width >= 140 {
        format!("{session} {DIM}|{RESET} {mem} {DIM}|{RESET} {team} {DIM}|{RESET} {tokens}")
    } else if width >= 100 {
        format!("{session} {DIM}|{RESET} {mem} {DIM}|{RESET} {team}")
    } else {
        format!("{session} {DIM}|{RESET} {mem}")
    }
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
        _ => format!("{DIM}idle{RESET}"),
    }
}

fn render_memory(m: &crate::state::MemoryStats) -> String {
    let total = m.decisions + m.patterns + m.lessons;
    if total == 0 {
        return format!("{DIM}no memory{RESET}");
    }
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
    if team.is_empty() {
        return format!("{DIM}no agents{RESET}");
    }
    let mut parts = Vec::new();
    // Sort agents by name for consistent display
    let mut entries: Vec<_> = team.iter().collect();
    entries.sort_by_key(|(k, _)| k.clone());

    for (name, info) in entries {
        let short = sanitize(name.strip_prefix("forge-").unwrap_or(name));
        let (icon, color) = match info.status.as_deref() {
            Some("done") => ("\u{2713}", GREEN),     // ✓
            Some("running") => ("\u{25b6}", YELLOW),  // ▶
            Some("pending") => ("\u{23f3}", DIM),     // ⏳
            Some("blocked") => ("\u{2717}", RED),     // ✗
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
    if t.input == 0 && t.output == 0 {
        return format!("{DIM}0 tokens{RESET}");
    }
    let rc = ratio_color(t.deterministic_ratio);
    let pct = (t.deterministic_ratio * 100.0) as u64;
    format!("{rc}{}K in \u{00b7} {}K out \u{00b7} {pct}% deterministic{RESET}", t.input / 1000, t.output / 1000)
}

fn render_skills(s: &crate::state::SkillStats) -> String {
    if s.fix_candidates > 0 {
        format!("{} skill{} {YELLOW}({} need{} fix){RESET}",
            s.active, plural(s.active), s.fix_candidates, if s.fix_candidates == 1 { "s" } else { "" })
    } else if s.active > 0 {
        format!("{} skill{}", s.active, plural(s.active))
    } else {
        String::new()
    }
}

fn plural(n: u64) -> &'static str {
    if n == 1 { "" } else { "s" }
}
