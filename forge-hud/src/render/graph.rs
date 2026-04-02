use crate::render::colors::*;
use crate::state::HudState;
use crate::stdin::StdinData;

pub fn render_graph_line(stdin: &StdinData, state: &HudState, width: usize) -> String {
    let sec = &state.security;
    let sc = security_color(sec.stale, sec.exposed);
    let m = &state.memory;

    // Model name (short)
    let model = if !stdin.model.display_name.is_empty() {
        sanitize(&stdin.model.display_name)
    } else {
        "forge".to_string()
    };

    // Context window bar
    let ctx = render_context_bar(stdin.context_window.used_percentage);

    // Git branch
    let branch = crate::stdin::git_branch(&stdin.cwd);
    let git_seg = if !branch.is_empty() {
        let b = sanitize(&branch);
        format!(" {DIM}git:{RESET}{CYAN}{b}{RESET}")
    } else {
        String::new()
    };

    // Session mode
    let session = render_session_info(&state.session);

    if width >= 160 {
        format!(
            "{BOLD}[{model}]{RESET} {ctx}{git_seg} {DIM}|{RESET} {CYAN}{}n {}e{RESET} {DIM}|{RESET} {BLUE}{}d {}p {}l{RESET} {DIM}|{RESET} {sc}{}sec{RESET} {DIM}|{RESET} {session}",
            state.graph.nodes, state.graph.edges, m.decisions, m.patterns, m.lessons, sec.total
        )
    } else if width >= 100 {
        format!(
            "{BOLD}[{model}]{RESET} {ctx}{git_seg} {DIM}|{RESET} {BLUE}{}d/{}p{RESET} {DIM}|{RESET} {sc}{}s{RESET} {DIM}|{RESET} {session}",
            m.decisions, m.patterns, sec.total
        )
    } else {
        format!("{BOLD}[{model}]{RESET} {ctx} {DIM}|{RESET} {session}")
    }
}

/// Render a context window usage bar like: Context [████████░░] 78%
fn render_context_bar(pct: f64) -> String {
    let pct_clamped = pct.clamp(0.0, 100.0);
    let filled = (pct_clamped / 10.0).round() as usize;
    let empty = 10_usize.saturating_sub(filled);
    let color = if pct_clamped >= 85.0 {
        RED
    } else if pct_clamped >= 60.0 {
        YELLOW
    } else {
        GREEN
    };
    let bar_filled = "█".repeat(filled);
    let bar_empty = "░".repeat(empty);
    format!("{DIM}Ctx{RESET} {color}[{bar_filled}{DIM}{bar_empty}{color}]{RESET} {color}{:.0}%{RESET}", pct_clamped)
}

fn render_session_info(s: &crate::state::SessionInfo) -> String {
    match (&s.mode, &s.phase) {
        (Some(m), Some(p)) => {
            let m = sanitize(m);
            let p = sanitize(p);
            format!("{MAGENTA}{m} . {p}{RESET}")
        }
        (Some(m), None) => {
            let m = sanitize(m);
            format!("{MAGENTA}{m}{RESET}")
        }
        _ => format!("{DIM}idle{RESET}"),
    }
}
