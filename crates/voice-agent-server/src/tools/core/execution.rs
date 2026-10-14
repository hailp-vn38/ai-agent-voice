use super::contract;
use crate::services::background::{
    JobEngine,
    plugins::{FrozenChoice, InvokeResult, PluginError},
};
use serde_json::{Value, json};

pub(super) fn take_chars(input: &str, cap: usize) -> (String, bool) {
    let result: String = input.chars().take(cap).collect();
    let truncated = input.chars().count() > cap;
    (result, truncated)
}

pub(crate) async fn invoke(
    engine: &JobEngine,
    choice: &FrozenChoice,
    arguments: Value,
    job_id: &str,
) -> Result<InvokeResult, PluginError> {
    let definition = contract(choice)?;
    let input_validator = jsonschema::validator_for(&definition["input_schema"])
        .map_err(|_| PluginError::WorkerProtocolError)?;
    if !input_validator.is_valid(&arguments) {
        return Err(PluginError::ApplicationError);
    }
    let maximum = arguments["max_chars"].as_u64().unwrap_or(16000).min(16000) as usize;
    let mut result = match choice.operation.as_str() {
        "data.json_query" => {
            let document = arguments["json_text"]
                .as_str()
                .ok_or(PluginError::ApplicationError)?;
            let pointer = arguments["pointer"]
                .as_str()
                .ok_or(PluginError::ApplicationError)?;
            if !valid_pointer(pointer) {
                return Err(PluginError::ApplicationError);
            }
            let value: Value =
                serde_json::from_str(document).map_err(|_| PluginError::ApplicationError)?;
            let selected = value
                .pointer(pointer)
                .ok_or(PluginError::ApplicationError)?;
            if selected.to_string().len() > 16000 {
                return Err(PluginError::ApplicationError);
            }
            json!({"value":selected})
        }
        "artifact.read_text" => {
            let artifact = arguments["artifact_id"]
                .as_str()
                .ok_or(PluginError::ApplicationError)?;
            let media_type: Option<String> =
                sqlx::query_scalar("SELECT media_type FROM job_artifacts WHERE job_id=? AND id=?")
                    .bind(job_id)
                    .bind(artifact)
                    .fetch_optional(engine.database.pool())
                    .await
                    .map_err(|_| PluginError::DatabaseUnavailable)?;
            if !matches!(
                media_type.as_deref(),
                Some("text/plain" | "text/markdown" | "application/json")
            ) {
                return Err(PluginError::ApplicationError);
            }
            let bytes = engine
                .read_artifact(job_id, artifact)
                .await
                .map_err(|_| PluginError::ApplicationError)?;
            let size = bytes.len();
            let content = std::str::from_utf8(&bytes).map_err(|_| PluginError::ApplicationError)?;
            let (text, truncated) = take_chars(content, maximum);
            json!({"text":text,"truncated":truncated,"size_bytes":size})
        }
        "web.fetch" => {
            let url = arguments["url"]
                .as_str()
                .ok_or(PluginError::ApplicationError)?;
            if !(url.starts_with("https://") || url.starts_with("http://")) {
                return Err(PluginError::ApplicationError);
            }
            // Target restrictions are deferred by design; response size is still bounded.
            let response = reqwest::Client::new()
                .get(url)
                .send()
                .await
                .map_err(|_| PluginError::ApplicationError)?;
            let status = response.status().as_u16();
            let content_type = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .chars()
                .take(128)
                .collect::<String>();
            let mut response = response;
            let mut bytes = Vec::new();
            let mut body_limit_reached = false;
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| PluginError::ApplicationError)?
            {
                let budget = 65536usize.saturating_sub(bytes.len());
                bytes.extend_from_slice(&chunk[..chunk.len().min(budget)]);
                if bytes.len() >= 65536 {
                    body_limit_reached = true;
                    break;
                }
            }
            let text = String::from_utf8_lossy(&bytes);
            let (text, truncated) = take_chars(&text, maximum);
            json!({"text":text,"status":status,"content_type":content_type,
                "truncated":truncated || body_limit_reached,
                "provenance":[{"source":"web","urls":[url],"capabilities":["web_fetch"]}]})
        }
        _ => return Err(PluginError::PermissionRequired),
    };
    bound_text_result(&mut result)?;
    let output_validator = jsonschema::validator_for(&definition["output_schema"])
        .map_err(|_| PluginError::WorkerProtocolError)?;
    if !output_validator.is_valid(&result) {
        return Err(PluginError::WorkerProtocolError);
    }
    Ok(InvokeResult {
        result,
        artifacts: Vec::new(),
    })
}

// serde_json resolves pointer tokens but does not reject invalid RFC 6901 escapes.
pub(super) fn valid_pointer(pointer: &str) -> bool {
    if !pointer.is_empty() && !pointer.starts_with('/') {
        return false;
    }
    let mut chars = pointer.chars();
    while let Some(c) = chars.next() {
        if c == '~' && !matches!(chars.next(), Some('0' | '1')) {
            return false;
        }
    }
    true
}

/// Preserve the durable Step's serialized byte budget, including escaping and metadata.
pub(super) fn bound_text_result(result: &mut Value) -> Result<(), PluginError> {
    const MAX_BYTES: usize = 65536;
    if result.to_string().len() <= MAX_BYTES {
        return Ok(());
    }
    let text = result["text"]
        .as_str()
        .ok_or(PluginError::ApplicationError)?
        .to_owned();
    result["truncated"] = json!(true);
    let mut low = 0;
    let mut high = text.chars().count();
    while low < high {
        let mid = low + (high - low).div_ceil(2);
        result["text"] = json!(text.chars().take(mid).collect::<String>());
        if result.to_string().len() <= MAX_BYTES {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    result["text"] = json!(text.chars().take(low).collect::<String>());
    if result.to_string().len() > MAX_BYTES {
        return Err(PluginError::ApplicationError);
    }
    Ok(())
}
