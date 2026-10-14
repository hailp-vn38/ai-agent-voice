use serde::Deserialize;
use serde_json::Value;

use super::PluginError;

pub(super) const MAX_STDOUT: usize = 64 * 1024;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ArtifactRef {
    pub path: String,
    pub media_type: String,
    pub size_bytes: Option<u64>,
    pub sha256: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WorkerError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Response {
    schema_version: u32,
    request_id: String,
    pub ok: bool,
    pub result: Option<Value>,
    #[serde(default)]
    pub artifacts: Vec<ArtifactRef>,
    pub error: Option<WorkerError>,
}

pub(super) fn parse_response(
    bytes: &[u8],
    request_id: &str,
    output_schema: &Value,
) -> Result<Response, PluginError> {
    if bytes.len() > MAX_STDOUT
        || !bytes.ends_with(b"\n")
        || bytes[..bytes.len() - 1].contains(&b'\n')
    {
        return Err(PluginError::WorkerProtocolError);
    }
    let response: Response = serde_json::from_slice(&bytes[..bytes.len() - 1])
        .map_err(|_| PluginError::WorkerProtocolError)?;
    if response
        .result
        .as_ref()
        .is_some_and(crate::services::background::plan::forbidden)
    {
        return Err(PluginError::WorkerProtocolError);
    }
    if response.schema_version != 1
        || response.request_id != request_id
        || response.artifacts.len() > 16
        || response.ok && (response.result.is_none() || response.error.is_some())
        || !response.ok
            && (response.result.is_some()
                || response.error.as_ref().is_none_or(|e| {
                    e.code.is_empty() || e.code.len() > 64 || e.message.len() > 256
                }))
    {
        return Err(PluginError::WorkerProtocolError);
    }
    if let Some(result) = &response.result {
        let validator = jsonschema::validator_for(output_schema)
            .map_err(|_| PluginError::WorkerProtocolError)?;
        if !validator.is_valid(result) {
            return Err(PluginError::WorkerProtocolError);
        }
    }
    Ok(response)
}
