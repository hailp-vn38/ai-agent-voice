//! Public, bounded Admin diagnostics for already-loaded provider runtimes.

use std::time::Instant;

use super::*;
use crate::services::provider_diagnostic::{
    ProviderDiagnosticError, ProviderDiagnosticRequestError,
};

const MAX_LLM_INPUT_BYTES: usize = 8 * 1024;
const MAX_TTS_INPUT_BYTES: usize = 4 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LlmTestRequest {
    input: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TtsTestRequest {
    text: String,
    voice: Option<String>,
    language: Option<String>,
}

pub(super) async fn test_llm_provider(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    let (request, body): (_, LlmTestRequest) = match json(request).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    if body.input.is_empty() || body.input.len() > MAX_LLM_INPUT_BYTES {
        return error(&request, StatusCode::BAD_REQUEST, "invalid_test_input");
    }
    let started = Instant::now();
    let outcome = state
        .provider_diagnostics
        .execute_llm(&key, body.input)
        .await;
    let diagnostic = match outcome {
        Ok(value) => value,
        Err(diagnostic_error) => return request_error_response(&request, diagnostic_error),
    };
    Json(serde_json::json!({
        "provider_key": diagnostic.provider_key,
        "type": "llm",
        "status": "success",
        "result": { "text": diagnostic.text },
        "metrics": { "elapsed_ms": started.elapsed().as_millis() },
        "runtime": {
            "runtime_status": "loaded",
            "tested_runtime": "loaded",
            "runtime_matches_desired": diagnostic.runtime.runtime_matches_desired,
            "requires_restart": diagnostic.runtime.requires_restart,
        }
    }))
    .into_response()
}

pub(super) async fn test_tts_provider(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    let (request, body): (_, TtsTestRequest) = match json(request).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let valid_option = |value: &Option<String>| {
        value
            .as_ref()
            .is_none_or(|value| !value.is_empty() && value.len() <= 128)
    };
    if body.text.is_empty()
        || body.text.len() > MAX_TTS_INPUT_BYTES
        || !valid_option(&body.voice)
        || !valid_option(&body.language)
    {
        return error(&request, StatusCode::BAD_REQUEST, "invalid_test_input");
    }
    let started = Instant::now();
    let diagnostic = match state
        .provider_diagnostics
        .execute_tts(
            &key,
            crate::providers::TtsDiagnosticRequest {
                text: body.text,
                voice: body.voice,
                language: body.language,
            },
        )
        .await
    {
        Ok(value) => value,
        Err(error_value) => return request_error_response(&request, error_value),
    };
    let mut response = diagnostic.wav.into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("audio/wav"));
    headers.insert(
        "x-provider-test-elapsed-ms",
        HeaderValue::from_str(&started.elapsed().as_millis().to_string()).expect("elapsed header"),
    );
    headers.insert(
        "x-provider-runtime-matches-desired",
        HeaderValue::from_static(if diagnostic.runtime.runtime_matches_desired {
            "true"
        } else {
            "false"
        }),
    );
    headers.insert(
        "x-provider-requires-restart",
        HeaderValue::from_static(if diagnostic.runtime.requires_restart {
            "true"
        } else {
            "false"
        }),
    );
    headers.insert(
        "x-provider-key",
        HeaderValue::from_str(&diagnostic.provider_key)
            .unwrap_or_else(|_| HeaderValue::from_static("invalid")),
    );
    response
}

fn request_error_response(
    request: &Request,
    error_value: ProviderDiagnosticRequestError,
) -> Response {
    match error_value {
        ProviderDiagnosticRequestError::NotFound => {
            error(request, StatusCode::NOT_FOUND, "provider_not_found")
        }
        ProviderDiagnosticRequestError::Disabled => {
            error(request, StatusCode::CONFLICT, "provider_disabled")
        }
        ProviderDiagnosticRequestError::DatabaseUnavailable => error(
            request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        ),
        ProviderDiagnosticRequestError::TypeMismatch => {
            error(request, StatusCode::BAD_REQUEST, "provider_type_mismatch")
        }
        ProviderDiagnosticRequestError::InvalidInput => {
            error(request, StatusCode::BAD_REQUEST, "invalid_test_input")
        }
        ProviderDiagnosticRequestError::Diagnostic(error_value) => {
            diagnostic_error_response(request, error_value)
        }
    }
}

fn diagnostic_error_response(request: &Request, error_value: ProviderDiagnosticError) -> Response {
    let (status, code) = match error_value {
        ProviderDiagnosticError::Busy => (StatusCode::TOO_MANY_REQUESTS, "provider_test_busy"),
        ProviderDiagnosticError::TypeMismatch => {
            (StatusCode::BAD_REQUEST, "provider_type_mismatch")
        }
        ProviderDiagnosticError::RuntimeNotLoaded => {
            (StatusCode::CONFLICT, "provider_runtime_not_loaded")
        }
        ProviderDiagnosticError::Timeout => (StatusCode::GATEWAY_TIMEOUT, "provider_test_timeout"),
        ProviderDiagnosticError::Unavailable => {
            (StatusCode::SERVICE_UNAVAILABLE, "provider_unavailable")
        }
        ProviderDiagnosticError::InvalidResponse => {
            (StatusCode::BAD_GATEWAY, "provider_invalid_response")
        }
        ProviderDiagnosticError::Failed => (StatusCode::BAD_GATEWAY, "provider_test_failed"),
    };
    error(request, status, code)
}
