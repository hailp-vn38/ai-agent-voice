//! Public, bounded Admin diagnostics for already-loaded provider runtimes.

use std::{io::Cursor, time::Instant};

use super::*;
use crate::services::provider_diagnostic::{
    ProviderDiagnosticError, ProviderDiagnosticRequestError,
};

const MAX_LLM_INPUT_BYTES: usize = 8 * 1024;
const MAX_TTS_INPUT_BYTES: usize = 4 * 1024;
const MAX_ASR_DURATION_SECONDS: u32 = 30;

pub(super) async fn test_vad_provider(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    let (parts, body) = request.into_parts();
    let request = Request::from_parts(parts, Body::empty());
    match to_bytes(body, 1).await {
        Ok(body) if body.is_empty() => {}
        _ => return error(&request, StatusCode::BAD_REQUEST, "invalid_test_input"),
    }
    let started = Instant::now();
    let diagnostic = match state.provider_diagnostics.execute_vad(&key).await {
        Ok(value) => value,
        Err(value) => return request_error_response(&request, value),
    };
    Json(serde_json::json!({
        "provider_key": diagnostic.provider_key,
        "type": "vad",
        "status": "success",
        "result": { "probability": diagnostic.probability, "start_sample": diagnostic.start_sample, "end_sample": diagnostic.end_sample },
        "metrics": { "elapsed_ms": started.elapsed().as_millis() },
        "runtime": { "runtime_status": "loaded", "tested_runtime": "loaded", "runtime_matches_desired": diagnostic.runtime.runtime_matches_desired, "tested_provider_id": diagnostic.runtime.tested_provider_id, "tested_revision": diagnostic.runtime.tested_revision, "requires_restart": diagnostic.runtime.requires_restart }
    })).into_response()
}

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
            "tested_provider_id": diagnostic.runtime.tested_provider_id, "tested_revision": diagnostic.runtime.tested_revision, "requires_restart": diagnostic.runtime.requires_restart,
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
    if let Some(revision) = diagnostic.runtime.tested_revision {
        headers.insert(
            "x-provider-tested-revision",
            HeaderValue::from_str(&revision.to_string()).expect("revision header"),
        );
    }
    if let Some(provider_id) = diagnostic.runtime.tested_provider_id {
        headers.insert(
            "x-provider-tested-id",
            HeaderValue::from_str(&provider_id.to_string()).expect("provider id header"),
        );
    }
    headers.insert(
        "x-provider-key",
        HeaderValue::from_str(&diagnostic.provider_key)
            .unwrap_or_else(|_| HeaderValue::from_static("invalid")),
    );
    response
}

pub(super) async fn test_asr_provider(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    if request
        .headers()
        .get(header::CONTENT_TYPE)
        .map(|value| value.as_bytes())
        != Some(b"audio/wav")
    {
        return error(&request, StatusCode::BAD_REQUEST, "invalid_test_input");
    }
    let (parts, body) = request.into_parts();
    let request = Request::from_parts(parts, Body::empty());
    let body = match to_bytes(body, MAX_ASR_TEST_BODY).await {
        Ok(body) => body,
        Err(_) => return error(&request, StatusCode::PAYLOAD_TOO_LARGE, "request_too_large"),
    };
    let (pcm, duration_ms) = match parse_asr_wav(&body) {
        Some(value) => value,
        None => return error(&request, StatusCode::BAD_REQUEST, "invalid_test_input"),
    };
    let started = Instant::now();
    let diagnostic = match state.provider_diagnostics.execute_asr(&key, pcm).await {
        Ok(value) => value,
        Err(error_value) => return request_error_response(&request, error_value),
    };
    let elapsed_ms = started.elapsed().as_millis();
    let mut metrics = serde_json::json!({
        "audio_duration_ms": duration_ms,
        "elapsed_ms": elapsed_ms,
    });
    if duration_ms > 0 {
        metrics["rtf"] =
            serde_json::json!(started.elapsed().as_secs_f64() / (duration_ms as f64 / 1_000.0));
    }
    Json(serde_json::json!({
        "provider_key": diagnostic.provider_key,
        "type": "asr",
        "status": "success",
        "result": { "text": diagnostic.text, "language": diagnostic.language },
        "metrics": metrics,
        "runtime": {
            "runtime_status": "loaded",
            "tested_runtime": "loaded",
            "runtime_matches_desired": diagnostic.runtime.runtime_matches_desired,
            "tested_provider_id": diagnostic.runtime.tested_provider_id, "tested_revision": diagnostic.runtime.tested_revision, "requires_restart": diagnostic.runtime.requires_restart,
        }
    }))
    .into_response()
}

pub(super) async fn test_speaker_provider(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    if request
        .headers()
        .get(header::CONTENT_TYPE)
        .map(|v| v.as_bytes())
        != Some(b"audio/wav")
        || request.headers().contains_key(header::CONTENT_ENCODING)
    {
        return error(&request, StatusCode::BAD_REQUEST, "invalid_test_input");
    }
    let revision = match expected(request.headers()) {
        Ok(revision) => revision,
        Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };
    let (parts, body) = request.into_parts();
    let request = Request::from_parts(parts, Body::empty());
    let body = match to_bytes(body, 524_288).await {
        Ok(body) => body,
        Err(_) => return error(&request, StatusCode::PAYLOAD_TOO_LARGE, "request_too_large"),
    };
    let (pcm, _) = match parse_asr_wav(&body) {
        Some(value) if value.0.sample_rate_hz() == 16_000 && value.1 <= 12_000 => value,
        _ => return error(&request, StatusCode::BAD_REQUEST, "invalid_test_input"),
    };
    match state
        .provider_diagnostics
        .execute_speaker(&key, revision, pcm)
        .await
    {
        Ok(value) => Json(value).into_response(),
        Err(value) => request_error_response(&request, value),
    }
}

fn parse_asr_wav(body: &[u8]) -> Option<(crate::audio::PcmF32Mono, u64)> {
    let mut reader = hound::WavReader::new(Cursor::new(body)).ok()?;
    let spec = reader.spec();
    if spec.channels != 1
        || spec.bits_per_sample != 16
        || spec.sample_format != hound::SampleFormat::Int
    {
        return None;
    }
    let samples = reader
        .samples::<i16>()
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    if samples.len() > usize::try_from(spec.sample_rate).ok()? * MAX_ASR_DURATION_SECONDS as usize {
        return None;
    }
    let duration_ms =
        u64::try_from(samples.len()).ok()?.saturating_mul(1_000) / u64::from(spec.sample_rate);
    Some((
        crate::audio::PcmF32Mono::new(
            samples
                .into_iter()
                .map(|sample| f32::from(sample) / f32::from(i16::MAX))
                .collect(),
            spec.sample_rate,
        ),
        duration_ms,
    ))
}

fn request_error_response(
    request: &Request,
    error_value: ProviderDiagnosticRequestError,
) -> Response {
    match error_value {
        ProviderDiagnosticRequestError::SpeakerManagerRequired => error(
            request,
            StatusCode::SERVICE_UNAVAILABLE,
            "speaker_runtime_manager_required",
        ),
        ProviderDiagnosticRequestError::RevisionConflict => {
            error(request, StatusCode::CONFLICT, "revision_conflict")
        }
        ProviderDiagnosticRequestError::Runtime(runtime_error) => {
            let status = match runtime_error {
                crate::services::provider_runtime::RuntimeError::Busy => {
                    StatusCode::TOO_MANY_REQUESTS
                }
                crate::services::provider_runtime::RuntimeError::Timeout => {
                    StatusCode::GATEWAY_TIMEOUT
                }
                crate::services::provider_runtime::RuntimeError::ArtifactsNotReady => {
                    StatusCode::CONFLICT
                }
                _ => StatusCode::SERVICE_UNAVAILABLE,
            };
            error(request, status, runtime_error.code())
        }
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PrepareRequest {}

pub(super) async fn prepare_provider(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    let (request, _body): (_, PrepareRequest) = match json(request).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    match state.provider_diagnostics.prepare(&key).await {
        Ok(prepared) => {
            let status = if prepared.runtime.desired_state
                == crate::services::provider_runtime::RuntimeState::Ready
            {
                StatusCode::OK
            } else {
                StatusCode::ACCEPTED
            };
            (status, Json(prepared)).into_response()
        }
        Err(ProviderDiagnosticRequestError::Diagnostic(ProviderDiagnosticError::Busy)) => error(
            &request,
            StatusCode::TOO_MANY_REQUESTS,
            "provider_runtime_busy",
        ),
        Err(error_value) => request_error_response(&request, error_value),
    }
}
