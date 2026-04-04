// extraction/openai.rs — OpenAI Chat Completions API extraction backend

use super::backend::ExtractionResult;
use super::prompt;

/// Extract memories using the OpenAI Chat Completions API.
///
/// Compatible with any OpenAI-compatible endpoint (custom `endpoint` field).
/// SECURITY: api_key is never logged.
pub async fn extract(
    api_key: &str,
    model: &str,
    endpoint: &str,
    conversation_text: &str,
) -> ExtractionResult {
    let system_prompt = prompt::EXTRACTION_SYSTEM_PROMPT;
    let user_message = format!("Conversation:\n{}", conversation_text);

    let client = reqwest::Client::new();
    let url = format!("{}/chat/completions", endpoint.trim_end_matches('/'));
    let body = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "system", "content": system_prompt},
            {"role": "user", "content": user_message}
        ],
        "max_tokens": 4096,
        "temperature": 0.1
    });

    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .json(&body)
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await;

    match resp {
        Err(e) => {
            if e.is_timeout() {
                ExtractionResult::Error(format!("OpenAI API timed out: {e}"))
            } else {
                ExtractionResult::Unavailable(format!("OpenAI API unreachable: {e}"))
            }
        }
        Ok(r) if !r.status().is_success() => {
            let status = r.status();
            let body_text = r.text().await.unwrap_or_default();
            ExtractionResult::Error(format!("OpenAI API returned {status}: {body_text}"))
        }
        Ok(r) => {
            let json: serde_json::Value = match r.json().await {
                Ok(v) => v,
                Err(e) => {
                    return ExtractionResult::Error(format!(
                        "failed to parse OpenAI response: {e}"
                    ))
                }
            };
            let text = json["choices"][0]["message"]["content"]
                .as_str()
                .unwrap_or("");
            let memories = prompt::parse_extraction_output(text);
            ExtractionResult::Success(memories)
        }
    }
}
