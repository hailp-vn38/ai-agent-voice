use super::artifact::read_owned_text;
use crate::services::background::{JobEngine, plugins::PluginError};
use serde_json::{Value, json};

fn normalize_document(media_type: &str, text: &str) -> Result<String, PluginError> {
    match media_type {
        "text/plain" => Ok(text.to_owned()),
        "application/json" => {
            let value: Value =
                serde_json::from_str(text).map_err(|_| PluginError::ApplicationError)?;
            serde_json::to_string_pretty(&value).map_err(|_| PluginError::ApplicationError)
        }
        "text/markdown" => {
            let links = regex::Regex::new(r"\[([^\]]+)\]\([^)]+\)")
                .map_err(|_| PluginError::WorkerProtocolError)?;
            let headers = regex::Regex::new(r"(?m)^\s{0,3}#{1,6}\s+")
                .map_err(|_| PluginError::WorkerProtocolError)?;
            let without_links = links.replace_all(text, "$1");
            let without_headers = headers.replace_all(&without_links, "");
            Ok(without_headers.replace(['*', '\u{60}'], ""))
        }
        _ => Err(PluginError::ApplicationError),
    }
}

pub(super) async fn extract(
    engine: &JobEngine,
    arguments: &Value,
    job_id: &str,
) -> Result<Value, PluginError> {
    let id = arguments["artifact_id"]
        .as_str()
        .ok_or(PluginError::ApplicationError)?;
    let (media_type, content, size) = read_owned_text(engine, job_id, id).await?;
    let normalized = normalize_document(&media_type, &content)?;
    let maximum = arguments["max_chars"]
        .as_u64()
        .unwrap_or(16_000)
        .min(16_000) as usize;
    let (text, truncated) = crate::tools::core::take_chars(&normalized, maximum);
    Ok(json!({"text":text,"format":media_type,"size_bytes":size,
    "truncated":truncated,"provenance":[{
        "source":"artifact","records":[id],"capabilities":["document_extract"]
    }]}))
}

pub(super) async fn inspect(
    engine: &JobEngine,
    arguments: &Value,
    job_id: &str,
) -> Result<Value, PluginError> {
    let id = arguments["artifact_id"]
        .as_str()
        .ok_or(PluginError::ApplicationError)?;
    let (media_type, text, size) = read_owned_text(engine, job_id, id).await?;
    Ok(json!({"media_type":media_type,"size_bytes":size,
        "lines":text.lines().count(),"words":text.split_whitespace().count(),
        "characters":text.chars().count()}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_text_is_distinct_from_raw_artifact_bytes() {
        assert_eq!(
            normalize_document(
                "text/markdown",
                "# Heading\n[Read](https://example.com) *bold*"
            )
            .unwrap(),
            "Heading\nRead bold"
        );
        assert_eq!(
            normalize_document("application/json", r#"{"a":1}"#).unwrap(),
            "{\n  \"a\": 1\n}"
        );
        assert!(normalize_document("application/pdf", "%PDF").is_err());
    }
}
