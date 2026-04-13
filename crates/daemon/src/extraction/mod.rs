// extraction/ — LLM extraction backends (Claude CLI, Claude API, OpenAI, Gemini, Ollama)

pub mod backend;
pub mod claude_api;
pub mod claude_cli;
pub mod gemini;
pub mod ollama;
pub mod openai;
pub mod prompt;
pub mod router;

pub use backend::{detect_backend, BackendChoice, ExtractionResult};
pub use prompt::{parse_extraction_output, ExtractedMemory};
