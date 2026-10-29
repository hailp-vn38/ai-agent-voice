use serde_json::{Value, json};

use crate::{providers::llm::LlmError, services::llm_gateway::GatewayLease};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WireError {
    NoLease,
    RateLimited { retry_after_secs: Option<u64> },
    Unauthorized,
    Other,
}

pub(crate) fn apply_lease_headers(
    mut builder: reqwest::RequestBuilder,
    lease: &GatewayLease,
) -> reqwest::RequestBuilder {
    builder = builder.bearer_auth(lease.access_token.expose());
    for (k, v) in &lease.extra_headers {
        builder = builder.header(k, v);
    }
    builder
}

pub(crate) fn check_wire_status(resp: reqwest::Response) -> Result<reqwest::Response, WireError> {
    let status = resp.status();
    if status.as_u16() == 429 {
        let retry_after = resp
            .headers()
            .get("retry-after")
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.trim().parse::<u64>().ok());
        return Err(WireError::RateLimited {
            retry_after_secs: retry_after,
        });
    }
    if matches!(status.as_u16(), 401 | 403) {
        return Err(WireError::Unauthorized);
    }
    if !status.is_success() {
        return Err(WireError::Other);
    }
    Ok(resp)
}

pub(crate) fn parse_tool_arguments(raw: &str) -> Result<Value, LlmError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(trimmed).map_err(|_| LlmError::Failed)
}
