use crate::render::colors::*;
use crate::stdin::StdinData;

/// Line 1: [Model | Plan] │ project git:(branch*) │ forge
pub fn render_line1(stdin: &StdinData, _width: usize) -> String {
    let model = sanitize(&stdin.model_name());
    let plan = stdin.plan_name();
    let sep = "\u{2502}"; // │ BOX DRAWINGS LIGHT VERTICAL

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

    format!(
        "{CYAN}[{model} | {plan}]{RESET} {DIM}{sep}{RESET} {YELLOW}{project}{RESET}{git_seg} {DIM}{sep}{RESET} {BOLD}{GREEN}forge{RESET}"
    )
}
