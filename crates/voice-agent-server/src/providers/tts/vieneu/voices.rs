use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::HashSet, time::Duration};

use super::config::VieneuHttpConfig;
use crate::{database::secrets::SecretValue, providers::ProviderInspectError};

#[derive(Deserialize)]
struct VoiceList {
    data: Vec<Voice>,
}

#[derive(Deserialize)]
struct Voice {
    id: String,
    name: String,
}

pub(crate) async fn discover(
    selection: &Value,
    credential: Option<&SecretValue>,
) -> Result<Value, ProviderInspectError> {
    let mut selection = selection.clone();
    let fields = selection
        .as_object_mut()
        .ok_or(ProviderInspectError::InvalidSelection)?;
    if fields.contains_key("api_key") {
        return Err(ProviderInspectError::InvalidSelection);
    }
    // Discovery precedes choosing a voice; it never uses this placeholder for synthesis.
    fields.insert("voice".into(), json!("__discovery__"));
    let config: VieneuHttpConfig =
        serde_json::from_value(selection).map_err(|_| ProviderInspectError::InvalidSelection)?;
    if !config.valid() {
        return Err(ProviderInspectError::InvalidSelection);
    }
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_millis(config.timeout_ms.min(10_000)))
        .build()
        .map_err(|_| ProviderInspectError::RemoteUnavailable)?;
    let mut request = client.get(format!(
        "{}/voices",
        config.base_url.as_str().trim_end_matches('/')
    ));
    if let Some(credential) = credential {
        request = request.bearer_auth(credential.expose());
    }
    let response = request
        .send()
        .await
        .map_err(|_| ProviderInspectError::RemoteUnavailable)?;
    if !response.status().is_success() {
        return Err(ProviderInspectError::RemoteUnavailable);
    }
    let mut stream = response.bytes_stream();
    let mut body = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| ProviderInspectError::RemoteUnavailable)?;
        if body.len() + chunk.len() > 65_536 {
            return Err(ProviderInspectError::RemoteUnavailable);
        }
        body.extend_from_slice(&chunk);
    }
    let list: VoiceList =
        serde_json::from_slice(&body).map_err(|_| ProviderInspectError::RemoteUnavailable)?;
    let mut ids = HashSet::new();
    if list.data.len() > 512
        || list.data.iter().any(|voice| {
            [&voice.id, &voice.name]
                .iter()
                .any(|v| v.trim().is_empty() || v.len() > 128 || v.chars().any(char::is_control))
                || !ids.insert(&voice.id)
        })
    {
        return Err(ProviderInspectError::RemoteUnavailable);
    }
    Ok(json!({"voices": list.data.iter().map(|voice| json!({
        "id": voice.id, "name": voice.name, "languages": ["vi-VN"]
    })).collect::<Vec<_>>(), "languages": [{"id": "vi-VN", "name": "Vietnamese"}]}))
}

#[cfg(test)]
mod tests;
