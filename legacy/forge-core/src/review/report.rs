use serde::Serialize;
use std::collections::HashMap;

#[derive(Serialize)]
pub struct ReviewContext {
    pub path: String,
    pub base: String,
    pub diff_lines: usize,
    pub files_changed: usize,
    pub files: Vec<String>,
    pub diff_preview: String,
}

/// Structured council review request for multi-model dispatch.
/// Each review_prompt is a pre-built prompt that can be sent to a different model
/// (e.g., Claude evaluator for spec compliance, Codex for security, etc.)
#[derive(Serialize)]
pub struct CouncilReviewRequest {
    pub review_request: CouncilPayload,
}

#[derive(Serialize)]
pub struct CouncilPayload {
    pub path: String,
    pub base: String,
    pub diff_lines: usize,
    pub files_changed: usize,
    pub files: Vec<String>,
    pub diff_preview: String,
    pub review_prompts: HashMap<String, String>,
}

pub fn to_json(ctx: &ReviewContext) -> String {
    serde_json::to_string_pretty(ctx).unwrap_or_else(|_| "{}".to_string())
}

pub fn to_markdown(ctx: &ReviewContext) -> String {
    let mut md = format!("## Council Review\n\n**Base:** `{}`\n**Files:** {}\n**Diff lines:** {}\n\n### Files\n\n",
        ctx.base, ctx.files_changed, ctx.diff_lines);
    for f in &ctx.files { md.push_str(&format!("- `{}`\n", f)); }
    md.push_str("\n### Diff Preview\n\n```diff\n");
    md.push_str(&ctx.diff_preview);
    md.push_str("\n```\n");
    md
}

/// Build a council review request with pre-built prompts for multi-model dispatch.
pub fn to_council_json(ctx: &ReviewContext) -> String {
    let file_list = ctx.files.iter()
        .map(|f| format!("  - {}", f))
        .collect::<Vec<_>>()
        .join("\n");

    let mut prompts = HashMap::new();

    prompts.insert(
        "spec_compliance".to_string(),
        format!(
            "Review this changeset for spec compliance. Check each file against the project's \
             conventions and requirements. Base: {base}, {n} files changed ({lines} diff lines).\n\
             Files:\n{files}\n\n\
             Verify: naming conventions, function signatures match interfaces, error handling \
             follows project patterns, documentation is updated where needed.",
            base = ctx.base,
            n = ctx.files_changed,
            lines = ctx.diff_lines,
            files = file_list,
        ),
    );

    prompts.insert(
        "security".to_string(),
        format!(
            "Review this changeset for security issues. Check for exposed secrets, injection \
             vulnerabilities, unsafe deserialization, path traversal, command injection, and \
             insecure defaults. Base: {base}, {n} files changed ({lines} diff lines).\n\
             Files:\n{files}\n\n\
             Flag: hardcoded credentials, unsanitized user input, missing authentication checks, \
             unsafe regex, symlink attacks, TOCTOU races.",
            base = ctx.base,
            n = ctx.files_changed,
            lines = ctx.diff_lines,
            files = file_list,
        ),
    );

    prompts.insert(
        "architecture".to_string(),
        format!(
            "Review this changeset for architectural consistency. Check module boundaries, \
             dependency direction, separation of concerns, and adherence to established patterns. \
             Base: {base}, {n} files changed ({lines} diff lines).\n\
             Files:\n{files}\n\n\
             Evaluate: circular dependency risk, layer violations, API surface changes, \
             backward compatibility, and whether the change belongs in the modules it touches.",
            base = ctx.base,
            n = ctx.files_changed,
            lines = ctx.diff_lines,
            files = file_list,
        ),
    );

    let request = CouncilReviewRequest {
        review_request: CouncilPayload {
            path: ctx.path.clone(),
            base: ctx.base.clone(),
            diff_lines: ctx.diff_lines,
            files_changed: ctx.files_changed,
            files: ctx.files.clone(),
            diff_preview: ctx.diff_preview.clone(),
            review_prompts: prompts,
        },
    };

    serde_json::to_string_pretty(&request).unwrap_or_else(|_| "{}".to_string())
}
