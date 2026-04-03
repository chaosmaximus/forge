// extraction/claude_cli.rs — Claude CLI extraction backend

use super::backend::ExtractionResult;
use super::prompt;

/// Extract memories using the `claude` CLI tool.
///
/// Shells out to: `claude -p --model {model} "{prompt}"`
/// where prompt = system_prompt + separator + conversation text.
pub async fn extract(model: &str, conversation_text: &str) -> ExtractionResult {
    let full_prompt = format!(
        "{}\n\n---\n\nConversation:\n{}",
        prompt::EXTRACTION_SYSTEM_PROMPT,
        conversation_text
    );

    let result = tokio::process::Command::new("claude")
        .args(["-p", "--model", model])
        .arg(&full_prompt)
        .output()
        .await;

    match result {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            ExtractionResult::Unavailable("claude CLI not found on PATH".to_string())
        }
        Err(e) => ExtractionResult::Error(format!("failed to run claude CLI: {e}")),
        Ok(output) => {
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return ExtractionResult::Error(format!(
                    "claude CLI exited with {}: {}",
                    output.status,
                    stderr.trim()
                ));
            }
            let stdout = String::from_utf8_lossy(&output.stdout);
            let memories = prompt::parse_extraction_output(&stdout);
            ExtractionResult::Success(memories)
        }
    }
}
