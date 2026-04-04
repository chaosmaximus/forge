// extraction/ — LLM extraction backends (Claude CLI, Claude API, OpenAI, Gemini, Ollama)

pub mod backend;
pub mod claude_api;
pub mod claude_cli;
pub mod gemini;
pub mod ollama;
pub mod openai;
pub mod prompt;

pub use backend::{BackendChoice, ExtractionResult, detect_backend};
pub use prompt::{ExtractedMemory, parse_extraction_output};
