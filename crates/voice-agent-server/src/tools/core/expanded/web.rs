use crate::services::background::plugins::PluginError;
use serde_json::{Value, json};

const BODY_LIMIT: usize = 65_536;
const TEXT_LIMIT: usize = 16_000;

fn html_entities(value: &str) -> String {
    value
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
}

/// Best-effort plain-text extraction, not a browser renderer. Discard active
/// script/style/noscript blocks and title markup instead of returning raw HTML.
fn html_plain_text(html: &str) -> Result<(String, String), PluginError> {
    let title_pattern = regex::Regex::new(r"(?is)<title\b[^>]*>(.*?)</title\s*>")
        .map_err(|_| PluginError::WorkerProtocolError)?;
    let ignored = regex::Regex::new(
        r"(?is)<(?:script|style|noscript)\b[^>]*>.*?</(?:script|style|noscript)\s*>",
    )
    .map_err(|_| PluginError::WorkerProtocolError)?;
    let tags = regex::Regex::new(r"(?s)<[^>]*>").map_err(|_| PluginError::WorkerProtocolError)?;
    let title = title_pattern
        .captures(html)
        .and_then(|capture| capture.get(1))
        .map(|part| html_entities(part.as_str()))
        .unwrap_or_default();
    let clean = ignored.replace_all(html, " ");
    let clean = title_pattern.replace_all(&clean, " ");
    let clean = tags.replace_all(&clean, " ");
    let plain = html_entities(&clean);
    Ok((
        title.trim().chars().take(256).collect(),
        plain.split_whitespace().collect::<Vec<_>>().join(" "),
    ))
}

/// Shared HTTP implementation for web.fetch and web.extract. Core-specific
/// network destination policy (Risk 2) remains explicitly deferred.
pub(super) async fn execute(arguments: &Value, extract: bool) -> Result<Value, PluginError> {
    let url = arguments["url"]
        .as_str()
        .ok_or(PluginError::ApplicationError)?;
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(PluginError::ApplicationError);
    }
    let response = reqwest::Client::new()
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
    let body = String::from_utf8_lossy(&bytes);
    let (title, body) = if extract && content_type.to_ascii_lowercase().contains("html") {
        html_plain_text(&body)?
    } else {
        (String::new(), body.into_owned())
    };
    let maximum = arguments["max_chars"]
        .as_u64()
        .unwrap_or(TEXT_LIMIT as u64)
        .min(TEXT_LIMIT as u64) as usize;
    let (text, truncated) = crate::tools::core::take_chars(&body, maximum);
    let capability = if extract { "web_extract" } else { "web_fetch" };
    let provenance = json!([{"source":"web","urls":[url],"capabilities":[capability]}]);
    if extract {
        Ok(json!({"url":url,"title":title,"text":text,"status":status,
            "content_type":content_type,"truncated":truncated || body_limit_reached,
            "provenance":provenance}))
    } else {
        Ok(
            json!({"text":text,"status":status,"content_type":content_type,
            "truncated":truncated || body_limit_reached,"provenance":provenance}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_active_html_and_extracts_title() {
        let (title, text) = html_plain_text(
            "<html><head><title>A &amp; B</title><script>alert(1)</script></head>\
             <body><h1>Hi &lt;there&gt;</h1><p>World</p></body></html>",
        )
        .unwrap();
        assert_eq!(title, "A & B");
        assert_eq!(text, "Hi <there> World");
        assert!(!text.contains("alert"));
    }
}
