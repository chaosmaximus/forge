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
    fn parse_incremental(
        &self,
        content: &str,
        last_offset: usize,
    ) -> (Vec<ConversationChunk>, usize);
}

/// Return ALL known adapters regardless of whether their directories exist yet.
/// Directories may be created later when the user first runs that agent.
/// The watcher handles directory polling; the adapter just needs to exist for routing.
pub fn detect_adapters() -> Vec<Box<dyn AgentAdapter>> {
    let home = std::env::var("HOME").unwrap_or_default();
    vec![
        Box::new(claude::ClaudeAdapter::new(&home)),
        Box::new(cline::ClineAdapter::new(&home)),
        Box::new(codex::CodexAdapter::new(&home)),
    ]
}

pub fn adapter_for_path<'a>(
    adapters: &'a [Box<dyn AgentAdapter>],
    path: &std::path::Path,
) -> Option<&'a dyn AgentAdapter> {
    adapters
        .iter()
        .find(|a| a.matches(path))
        .map(|a| a.as_ref())
}
