use crate::services::background::plugins::PluginError;
use serde_json::{Value, json};
use std::time::Duration;

mod entities;
mod html;

use html::html_plain_text;

// Read enough raw HTML to reach article content beyond large head/style blocks.
// The returned text is bounded separately after extraction.
const BODY_LIMIT: usize = 2_097_152;
const TEXT_LIMIT: usize = 16_000;

/// Shared HTTP implementation for web.fetch and web.extract. Core-specific
/// network destination policy (Risk 2) remains explicitly deferred.
pub(super) async fn execute(arguments: &Value, extract: bool) -> Result<Value, PluginError> {
    let url = arguments["url"]
        .as_str()
        .ok_or(PluginError::ApplicationError)?;
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(PluginError::ApplicationError);
    }
    // Bound the request duration and use a browser UA for compatibility.
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent(
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
             (KHTML, like Gecko) Chrome/120.0 Safari/537.36",
        )
        .build()
        .map_err(|_| PluginError::ApplicationError)?;
    // shortcut: each page refetches the URL; use snapshots if changing content needs stable offsets.
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|_| PluginError::ApplicationError)?;
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
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
        let available = BODY_LIMIT.saturating_sub(bytes.len());
        bytes.extend_from_slice(&chunk[..chunk.len().min(available)]);
        if bytes.len() >= BODY_LIMIT {
            body_limit_reached = true;
            break;
        }
    }
    // Apply the text limit after parsing, separately from the raw body cap.
    let body = String::from_utf8_lossy(&bytes);
    let (title, body) = if extract && content_type.to_ascii_lowercase().contains("html") {
        html_plain_text(
            &body,
            arguments["mode"].as_str().unwrap_or("article") == "article",
        )?
    } else {
        (String::new(), body.into_owned())
    };
    let maximum = arguments["max_chars"]
        .as_u64()
        .unwrap_or(TEXT_LIMIT as u64)
        .min(TEXT_LIMIT as u64) as usize;
    let offset = arguments["offset"].as_u64().unwrap_or(0);
    let offset_index = usize::try_from(offset).map_err(|_| PluginError::ApplicationError)?;
    let mut remaining = body.chars().skip(offset_index);
    let text: String = remaining.by_ref().take(maximum).collect();
    let truncated = remaining.next().is_some();
    let next_offset = truncated.then(|| offset + text.chars().count() as u64);
    let capability = if extract { "web_extract" } else { "web_fetch" };
    let provenance = json!([{"source":"web","urls":[url],"capabilities":[capability]}]);
    if extract {
        Ok(json!({"url":url,"title":title,"text":text,"status":status,
            "content_type":content_type,"truncated":truncated || body_limit_reached,
            "offset":offset,"next_offset":next_offset,"body_limit_reached":body_limit_reached,
            "provenance":provenance}))
    } else {
        Ok(
            json!({"text":text,"status":status,"content_type":content_type,
            "truncated":truncated || body_limit_reached,
            "offset":offset,"next_offset":next_offset,"body_limit_reached":body_limit_reached,
            "provenance":provenance}),
        )
    }
}

#[cfg(test)]
mod tests;
