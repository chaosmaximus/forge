pub mod claude;
pub mod cline;
pub mod codex;

use forge_core::types::ConversationChunk;
use std::path::PathBuf;

pub trait AgentAdapter: Send + Sync {
    fn name(&self) -> &str;
    fn watch_dirs(&self) -> Vec<PathBuf>;
    fn matches(&self, path: &std::path::Path) -> bool;
    fn file_extension(&self) -> &str;
    fn parse(&self, content: &str) -> Vec<ConversationChunk>;
    fn parse_incremental(&self, content: &str, last_offset: usize) -> (Vec<ConversationChunk>, usize);
}

pub fn detect_adapters() -> Vec<Box<dyn AgentAdapter>> {
    let home = std::env::var("HOME").unwrap_or_default();
    let mut adapters: Vec<Box<dyn AgentAdapter>> = Vec::new();

    let claude = claude::ClaudeAdapter::new(&home);
    if !claude.watch_dirs().is_empty() {
        adapters.push(Box::new(claude));
    }

    let cline = cline::ClineAdapter::new(&home);
    if !cline.watch_dirs().is_empty() {
        adapters.push(Box::new(cline));
    }

    // Codex will be added by another agent — for now create empty placeholder file
    // so the module compiles

    adapters
}

pub fn adapter_for_path<'a>(
    adapters: &'a [Box<dyn AgentAdapter>],
    path: &std::path::Path,
) -> Option<&'a dyn AgentAdapter> {
    adapters.iter().find(|a| a.matches(path)).map(|a| a.as_ref())
}
