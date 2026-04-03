// extraction/claude_cli.rs — Claude CLI extraction backend

use super::backend::ExtractionResult;
use super::prompt;
use tokio::time::{timeout, Duration};

/// Maximum time to wait for the claude CLI to respond.
const EXTRACTION_TIMEOUT: Duration = Duration::from_secs(60);

/// Extract memories using the `claude` CLI tool.
///
/// Shells out to: `claude -p --model {model} "{prompt}"`
/// where prompt = system_prompt + separator + conversation text.
///
/// The command is wrapped in a 60-second timeout to prevent hangs.
pub async fn extract(model: &str, conversation_text: &str) -> ExtractionResult {
    let full_prompt = format!(
        "{}\n\n---\n\nConversation:\n{}",
        prompt::EXTRACTION_SYSTEM_PROMPT,
        conversation_text
    );

    let mut cmd = tokio::process::Command::new("claude");
    cmd.args(["-p", "--model", model])
        .arg(&full_prompt)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true); // Kill child if future is dropped (e.g., timeout)

    let result = timeout(EXTRACTION_TIMEOUT, cmd.output()).await;

    match result {
        Ok(Ok(output)) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let memories = prompt::parse_extraction_output(&stdout);
            ExtractionResult::Success(memories)
        }
        Ok(Ok(output)) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            ExtractionResult::Error(format!(
                "claude CLI exited with {}: {}",
                output.status,
                stderr.trim()
            ))
        }
        Ok(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            ExtractionResult::Unavailable("claude CLI not found on PATH".to_string())
        }
        Ok(Err(e)) => ExtractionResult::Error(format!("failed to run claude CLI: {e}")),
        Err(_) => ExtractionResult::Error("claude CLI timed out (60s)".into()),
    }
}
