// extraction/ — LLM extraction backends (Claude CLI + Ollama)

pub mod backend;
pub mod claude_cli;
pub mod ollama;
pub mod prompt;

pub use backend::{BackendChoice, ExtractionResult, detect_backend};
pub use prompt::{ExtractedMemory, parse_extraction_output};
