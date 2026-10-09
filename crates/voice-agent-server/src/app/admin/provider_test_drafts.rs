//! HTTP codecs only; source, credential and runtime ownership belong to ProviderTestRunner.
use super::*;
use crate::services::provider_test::{
    ProviderTestDraft, ProviderTestError, ProviderTestInput, ProviderTestOutput, ProviderTestRunner,
};
use axum::extract::{FromRequest, Multipart};
use std::time::Instant;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TextInput {
    text: String,
    voice: Option<String>,
    language: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftRequest {
    provider: ProviderTestDraft,
    input: TextInput,
}

fn runner(state: &AppState) -> ProviderTestRunner<'_> {
    ProviderTestRunner {
        diagnostics: &state.provider_diagnostics,
        database: state.database.as_deref(),
        secrets: state.secret_resolver.clone(),
    }
}
fn failure(request: &Request, cause: ProviderTestError) -> Response {
    match cause {
        ProviderTestError::Config => {
            error(request, StatusCode::BAD_REQUEST, "provider_config_invalid")
        }
        ProviderTestError::Credential => {
            error(request, StatusCode::BAD_REQUEST, "credential_invalid")
        }
        ProviderTestError::Diagnostic(cause) => {
            provider_tests::request_error_response(request, cause)
        }
    }
}
fn response(
    output: ProviderTestOutput,
    kind: &str,
    started: Instant,
    duration: Option<u64>,
) -> Response {
    match output {
        ProviderTestOutput::Wav(wav) => {
            let mut response = wav.into_response();
            response
                .headers_mut()
                .insert(header::CONTENT_TYPE, HeaderValue::from_static("audio/wav"));
            response
                .headers_mut()
                .insert("x-provider-test-source", HeaderValue::from_static("draft"));
            response.headers_mut().insert(
                "x-provider-test-elapsed-ms",
                HeaderValue::from_str(&started.elapsed().as_millis().to_string())
                    .expect("elapsed header"),
            );
            response
        }
        ProviderTestOutput::Text { text, language } => {
            let mut metrics = serde_json::json!({"elapsed_ms":started.elapsed().as_millis()});
            if let Some(duration) = duration {
                metrics["audio_duration_ms"] = serde_json::json!(duration);
                if duration > 0 {
                    metrics["rtf"] = serde_json::json!(
                        started.elapsed().as_secs_f64() * 1000.0 / duration as f64
                    );
                }
            }
            let mut result = serde_json::json!({"text":text});
            if let Some(language) = language {
                result["language"] = serde_json::json!(language);
            }
            Json(serde_json::json!({"test_source":"draft","type":kind,"status":"success","result":result,"metrics":metrics,"runtime":{"test_runtime_ready":true,"persisted_runtime_modified":false}})).into_response()
        }
    }
}
pub(super) async fn llm(State(state): State<AppState>, request: Request) -> Response {
    let (request, body): (_, DraftRequest) = match json(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    if body.input.voice.is_some() || body.input.language.is_some() {
        return error(&request, StatusCode::BAD_REQUEST, "invalid_test_input");
    }
    let started = Instant::now();
    match runner(&state)
        .run(body.provider, ProviderTestInput::Llm(body.input.text))
        .await
    {
        Ok(v) => response(v, "llm", started, None),
        Err(e) => failure(&request, e),
    }
}
pub(super) async fn tts(State(state): State<AppState>, request: Request) -> Response {
    let (request, body): (_, DraftRequest) = match json(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    let started = Instant::now();
    let input = crate::providers::TtsDiagnosticRequest {
        text: body.input.text,
        voice: body.input.voice,
        language: body.input.language,
    };
    match runner(&state)
        .run(body.provider, ProviderTestInput::Tts(input))
        .await
    {
        Ok(v) => response(v, "tts", started, None),
        Err(e) => failure(&request, e),
    }
}
pub(super) async fn asr(State(state): State<AppState>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let request = Request::from_parts(parts.clone(), Body::empty());
    let mut multipart =
        match Multipart::from_request(Request::from_parts(parts, body), &state).await {
            Ok(v) => v,
            Err(_) => return error(&request, StatusCode::BAD_REQUEST, "invalid_test_input"),
        };
    let mut provider = None;
    let mut audio = None;
    loop {
        let mut field = match multipart.next_field().await {
            Ok(Some(v)) => v,
            Ok(None) => break,
            Err(e) => return error(&request, e.status(), "invalid_test_input"),
        };
        let name = field.name().unwrap_or("").to_owned();
        let cap = match name.as_str() {
            "provider" if provider.is_none() => provider_config::MAX_PROVIDER_CONFIG_BYTES + 8192,
            "audio" if audio.is_none() && field.content_type() == Some("audio/wav") => {
                MAX_ASR_TEST_BODY
            }
            _ => return error(&request, StatusCode::BAD_REQUEST, "invalid_test_input"),
        };
        let mut bytes = Vec::new();
        loop {
            match field.chunk().await {
                Ok(Some(chunk)) if bytes.len().saturating_add(chunk.len()) <= cap => {
                    bytes.extend_from_slice(&chunk)
                }
                Ok(Some(_)) => {
                    return error(&request, StatusCode::PAYLOAD_TOO_LARGE, "request_too_large");
                }
                Ok(None) => break,
                Err(e) => return error(&request, e.status(), "invalid_test_input"),
            }
        }
        if name == "provider" {
            provider = serde_json::from_slice::<ProviderTestDraft>(&bytes).ok();
            if provider.is_none() {
                return error(&request, StatusCode::BAD_REQUEST, "invalid_test_input");
            }
        } else {
            audio = provider_tests::parse_asr_wav(&bytes);
            if audio.is_none() {
                return error(&request, StatusCode::BAD_REQUEST, "invalid_test_input");
            }
        }
    }
    let (Some(provider), Some((pcm, duration))) = (provider, audio) else {
        return error(&request, StatusCode::BAD_REQUEST, "invalid_test_input");
    };
    let started = Instant::now();
    match runner(&state)
        .run(provider, ProviderTestInput::Asr(pcm))
        .await
    {
        Ok(v) => response(v, "asr", started, Some(duration)),
        Err(e) => failure(&request, e),
    }
}
