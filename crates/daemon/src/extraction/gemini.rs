// extraction/gemini.rs — Google Gemini API extraction backend

use super::backend::ExtractionResult;
use super::prompt;

/// Extract memories using the Google Gemini API.
///
/// Uses the `generateContent` endpoint with API key passed as query parameter.
/// SECURITY: api_key is never logged.
pub async fn extract(api_key: &str, model: &str, conversation_text: &str) -> ExtractionResult {
    let system_prompt = prompt::EXTRACTION_SYSTEM_PROMPT;
    let user_message = format!("Conversation:\n{}", conversation_text);

    let client = reqwest::Client::new();
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        model, api_key
    );

    // Gemma models don't support system_instruction — prepend to user message instead
    let is_gemma = model.starts_with("gemma");
    // Force JSON output for reliable parsing
    let body = if is_gemma {
        let combined = format!("{}\n\n---\n\n{}", system_prompt, user_message);
        serde_json::json!({
            "contents": [{"parts": [{"text": combined}]}],
            "generationConfig": {
                "maxOutputTokens": 4096,
                "temperature": 0.1,
                "responseMimeType": "application/json"
            }
        })
    } else {
        serde_json::json!({
            "system_instruction": {"parts": [{"text": system_prompt}]},
            "contents": [{"parts": [{"text": user_message}]}],
            "generationConfig": {
                "maxOutputTokens": 4096,
                "temperature": 0.1,
                "responseMimeType": "application/json"
            }
        })
    };

    let resp = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&body)
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await;

    match resp {
        Err(e) => {
            if e.is_timeout() {
                ExtractionResult::Error(format!("Gemini API timed out: {e}"))
            } else {
                ExtractionResult::Unavailable(format!("Gemini API unreachable: {e}"))
            }
        }
        Ok(r) if !r.status().is_success() => {
            let status = r.status();
            let body_text = r.text().await.unwrap_or_default();
            ExtractionResult::Error(format!("Gemini API returned {status}: {body_text}"))
        }
        Ok(r) => {
            let json: serde_json::Value = match r.json().await {
                Ok(v) => v,
                Err(e) => {
                    return ExtractionResult::Error(format!(
                        "failed to parse Gemini response: {e}"
                    ))
                }
            };
            let text = json["candidates"][0]["content"]["parts"][0]["text"]
                .as_str()
                .unwrap_or("");
            let memories = prompt::parse_extraction_output(text);
            ExtractionResult::Success(memories)
        }
    }
}
