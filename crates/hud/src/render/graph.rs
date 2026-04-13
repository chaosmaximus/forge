use crate::render::colors::*;
use crate::stdin::StdinData;

/// Line 1: [Model | Plan] │ Forge v0.4.0 │ project git:(branch*) │ ⎈ k8s
pub fn render_line1(stdin: &StdinData, state: &crate::state::HudState, _width: usize) -> String {
    let model = sanitize(&stdin.model_name());
    let plan = stdin.plan_name();
    let sep = format!(" {DIM}\u{2502}{RESET} "); // │

    let ver = state
        .version
        .as_deref()
        .unwrap_or(env!("CARGO_PKG_VERSION"));

    let project = sanitize(&stdin.project_name());
    let branch = crate::stdin::git_branch(stdin.cwd_str());
    let dirty = crate::stdin::git_dirty(stdin.cwd_str());

    let git_seg = if !branch.is_empty() {
        let b = sanitize(&branch);
        let d = if dirty { "*" } else { "" };
        format!(" {MAGENTA}git:({CYAN}{b}{d}{MAGENTA}){RESET}")
    } else {
        String::new()
    };

    let mut parts = vec![
        format!("{CYAN}[{model} | {plan}]{RESET}"),
        format!("{BOLD}{GREEN}Forge v{ver}{RESET}"),
    ];

    if !project.is_empty() {
        parts.push(format!("{YELLOW}{project}{RESET}{git_seg}"));
    }

    // K8s context on line 1 (compact)
    if let Some(ctx) = &state.k8s {
        if let Some(name) = &ctx.context {
            if !name.is_empty() {
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
                parts.push(format!("{CYAN}\u{2388} {short}{ns}{RESET}"));
            }
        }
    }

    parts.join(&sep)
}
