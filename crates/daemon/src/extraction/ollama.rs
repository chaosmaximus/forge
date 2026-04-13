// extraction/ollama.rs — Ollama HTTP API extraction + embedding backend

use super::backend::ExtractionResult;
use super::prompt;

/// Extract memories via Ollama's `/api/generate` endpoint.
pub async fn extract(endpoint: &str, model: &str, conversation_text: &str) -> ExtractionResult {
    let full_prompt = format!(
        "{}\n\n---\n\nConversation:\n{}",
        prompt::EXTRACTION_SYSTEM_PROMPT,
        conversation_text
    );

    let client = reqwest::Client::new();
    let url = format!("{endpoint}/api/generate");

    let body = serde_json::json!({
        "model": model,
        "prompt": full_prompt,
        "stream": false,
    });

    let resp = client
        .post(&url)
        .json(&body)
        .timeout(std::time::Duration::from_secs(120))
        .send()
        .await;

    match resp {
        Err(e) => {
            if e.is_connect() || e.is_timeout() {
                ExtractionResult::Unavailable(format!("ollama not reachable at {endpoint}: {e}"))
            } else {
                ExtractionResult::Error(format!("ollama request failed: {e}"))
            }
        }
        Ok(r) if !r.status().is_success() => {
            let status = r.status();
            let body = r.text().await.unwrap_or_default();
            ExtractionResult::Error(format!("ollama returned {status}: {body}"))
        }
        Ok(r) => {
            let json: serde_json::Value = match r.json().await {
                Ok(v) => v,
                Err(e) => {
                    return ExtractionResult::Error(format!("failed to parse ollama response: {e}"))
                }
            };

            let response_text = json.get("response").and_then(|v| v.as_str()).unwrap_or("");

            let memories = prompt::parse_extraction_output(response_text);
            ExtractionResult::Success(memories)
        }
    }
}

/// Generate embeddings via Ollama's `/api/embed` endpoint.
pub async fn embed(endpoint: &str, model: &str, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
    let client = reqwest::Client::new();
    let url = format!("{endpoint}/api/embed");

    let body = serde_json::json!({
        "model": model,
        "input": texts,
    });

    let resp = client
        .post(&url)
        .json(&body)
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| format!("ollama embed request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("ollama embed returned {status}: {body}"));
    }

    #[derive(serde::Deserialize)]
    struct EmbedResponse {
        embeddings: Vec<Vec<f32>>,
    }

    let parsed: EmbedResponse = resp
        .json()
        .await
        .map_err(|e| format!("failed to parse ollama embed response: {e}"))?;

    Ok(parsed.embeddings)
}
