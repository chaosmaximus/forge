// extraction/claude_api.rs — Anthropic Messages API extraction backend

use super::backend::ExtractionResult;
use super::prompt;

/// Extract memories using the Anthropic Messages API.
///
/// Uses the `/v1/messages` endpoint with the provided API key.
/// SECURITY: api_key is never logged.
pub async fn extract(api_key: &str, model: &str, conversation_text: &str) -> ExtractionResult {
    let system_prompt = prompt::EXTRACTION_SYSTEM_PROMPT;
    let user_message = format!("Conversation:\n{}", conversation_text);

    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "model": model,
        "max_tokens": 4096,
        "system": system_prompt,
        "messages": [
            {"role": "user", "content": user_message}
        ]
    });

    let resp = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await;

    match resp {
        Err(e) => {
            if e.is_timeout() {
                ExtractionResult::Error(format!("Claude API timed out: {e}"))
            } else {
                ExtractionResult::Unavailable(format!("Claude API unreachable: {e}"))
            }
        }
        Ok(r) if !r.status().is_success() => {
            let status = r.status();
            let body_text = r.text().await.unwrap_or_default();
            ExtractionResult::Error(format!("Claude API returned {status}: {body_text}"))
        }
        Ok(r) => {
            let json: serde_json::Value = match r.json().await {
                Ok(v) => v,
                Err(e) => {
                    return ExtractionResult::Error(format!(
                        "failed to parse Claude API response: {e}"
                    ))
                }
            };
            // Extract text from content blocks
            let text = json["content"]
                .as_array()
                .and_then(|blocks| blocks.iter().find(|b| b["type"] == "text"))
                .and_then(|b| b["text"].as_str())
                .unwrap_or("");
            let memories = prompt::parse_extraction_output(text);
            ExtractionResult::Success(memories)
        }
    }
}
