use crate::render::colors::*;
use crate::state::HudState;

/// Line 2: Forge-specific — mode, agents, tasks, waves, memory, security, tokens
pub fn render_line2(state: &HudState, width: usize) -> String {
    let session = render_session_mode(&state.session);
    let mem = render_memory(&state.memory);
    let sec = render_security(&state.security);
    let team = render_team(&state.team);
    let tokens = render_tokens(&state.tokens);
    let skills = render_skills(&state.skills);

    if width >= 160 {
        format!("{session} {DIM}|{RESET} {mem} {DIM}|{RESET} {sec} {DIM}|{RESET} {team} {DIM}|{RESET} {tokens} {DIM}|{RESET} {skills}")
    } else if width >= 120 {
        format!("{session} {DIM}|{RESET} {mem} {DIM}|{RESET} {team} {DIM}|{RESET} {tokens}")
    } else if width >= 80 {
        format!("{session} {DIM}|{RESET} {team} {DIM}|{RESET} {tokens}")
    } else {
        format!("{session} {DIM}|{RESET} {team}")
    }
}

fn render_session_mode(s: &crate::state::SessionInfo) -> String {
    match (&s.mode, &s.phase, &s.wave) {
        (Some(m), Some(p), Some(w)) => {
            format!("{BOLD}{MAGENTA}{}{RESET}{DIM}.{RESET}{MAGENTA}{}{RESET} {DIM}w{RESET}{CYAN}{}{RESET}",
                sanitize(m), sanitize(p), sanitize(w))
        }
        (Some(m), Some(p), None) => {
            format!("{BOLD}{MAGENTA}{}{RESET}{DIM}.{RESET}{MAGENTA}{}{RESET}",
                sanitize(m), sanitize(p))
        }
        (Some(m), None, _) => format!("{MAGENTA}{}{RESET}", sanitize(m)),
        _ => format!("{DIM}idle{RESET}"),
    }
}

fn render_memory(m: &crate::state::MemoryStats) -> String {
    if m.decisions == 0 && m.patterns == 0 && m.lessons == 0 {
        return format!("{DIM}0 mem{RESET}");
    }
    format!("{BLUE}{}d {}p {}l{RESET}", m.decisions, m.patterns, m.lessons)
}

fn render_security(s: &crate::state::SecurityStats) -> String {
    let c = crate::render::colors::security_color(s.stale, s.exposed);
    if s.total == 0 {
        return format!("{GREEN}0sec{RESET}");
    }
    if s.stale > 0 {
        format!("{c}{}sec {}stale{RESET}", s.total, s.stale)
    } else {
        format!("{c}{}sec{RESET}", s.total)
    }
}

fn render_team(team: &std::collections::HashMap<String, crate::state::AgentInfo>) -> String {
    if team.is_empty() {
        return format!("{DIM}no agents{RESET}");
    }
    let mut parts = Vec::new();
    for (name, info) in team {
        let short = sanitize(name.strip_prefix("forge-").unwrap_or(name));
        let status_icon = match info.status.as_deref() {
            Some("done") => format!("{GREEN}v{RESET}"),
            Some("running") => format!("{YELLOW}*{RESET}"),
            Some("pending") => format!("{DIM}~{RESET}"),
            Some("blocked") => format!("{RED}!{RESET}"),
            _ => format!("{DIM}?{RESET}"),
        };
        // Show current tool if running
        let tool_info = if info.status.as_deref() == Some("running") {
            info.last_tool.as_ref()
                .map(|t| format!("{DIM}({t}){RESET}"))
                .unwrap_or_default()
        } else {
            String::new()
        };
        parts.push(format!("{short}{status_icon}{tool_info}"));
    }
    parts.join(" ")
}

fn render_tokens(t: &crate::state::TokenStats) -> String {
    let rc = ratio_color(t.deterministic_ratio);
    let pct = (t.deterministic_ratio * 100.0) as u64;
    if t.input == 0 && t.output == 0 {
        return format!("{DIM}0tok{RESET}");
    }
    format!("{rc}{}K/{}K {pct}%det{RESET}", t.input / 1000, t.output / 1000)
}

fn render_skills(s: &crate::state::SkillStats) -> String {
    if s.fix_candidates > 0 {
        format!("{}sk {YELLOW}{}fix{RESET}", s.active, s.fix_candidates)
    } else if s.active > 0 {
        format!("{}sk", s.active)
    } else {
        String::new()
    }
}
