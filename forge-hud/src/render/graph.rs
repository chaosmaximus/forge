use crate::render::colors::*;
use crate::state::HudState;

pub fn render_graph_line(state: &HudState, width: usize) -> String {
    let sec = &state.security;
    let sc = security_color(sec.stale, sec.exposed);
    let m = &state.memory;
    let session = render_session_info(&state.session);

    if width >= 160 {
        format!("{BOLD}forge v0.2.0{RESET} {DIM}|{RESET} {CYAN}{}n {}e{RESET} {DIM}|{RESET} {BLUE}{}d {}p {}l{RESET} {DIM}|{RESET} {sc}{}sec {}stale{RESET} {DIM}|{RESET} {session}",
            state.graph.nodes, state.graph.edges, m.decisions, m.patterns, m.lessons, sec.total, sec.stale)
    } else if width >= 100 {
        format!("{BOLD}forge{RESET} {DIM}|{RESET} {CYAN}{}n/{}e{RESET} {DIM}|{RESET} {BLUE}{}d/{}p{RESET} {DIM}|{RESET} {sc}{}s{RESET} {DIM}|{RESET} {session}",
            state.graph.nodes, state.graph.edges, m.decisions, m.patterns, sec.total)
    } else {
        format!("{BOLD}forge{RESET} {DIM}|{RESET} {session}")
    }
}

fn render_session_info(s: &crate::state::SessionInfo) -> String {
    match (&s.mode, &s.phase) {
        (Some(m), Some(p)) => format!("{MAGENTA}{m} . {p}{RESET}"),
        (Some(m), None) => format!("{MAGENTA}{m}{RESET}"),
        _ => format!("{DIM}idle{RESET}"),
    }
}
