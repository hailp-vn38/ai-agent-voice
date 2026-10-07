//! Bounded, separately authenticated Admin HTTP surface.  It deliberately exposes only
//! Agent and Device desired configuration in this rollout.
use super::AppState;
use crate::database::secrets::SecretRef;
use crate::database::{external_mcp_policy, provider_config};
use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{Path, Query, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{get, put},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use sqlx::{Executor, FromRow, Sqlite, SqlitePool};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

const MAX_BODY: usize = 256 * 1024;
pub(super) const MAX_ASR_TEST_BODY: usize = 5 * 1024 * 1024;
/// Voice samples are raw PCM16 WAV: 12s mono 16 kHz is 384 KiB, so the 256 KiB
/// JSON cap would reject every valid clip. The route itself still enforces
/// `speaker.max_audio_body_bytes` (512 KiB) and the 12s duration cap.
pub(super) const MAX_SPEAKER_SAMPLE_BODY: usize = 512 * 1024;
const PAGE_DEFAULT: u32 = 50;
const PAGE_MAX: u32 = 200;

pub(super) fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/system", get(get_system))
        .route("/agents", get(list_agents).post(create_agent))
        .route(
            "/agents/{key}",
            get(get_agent).patch(patch_agent).delete(delete_agent),
        )
        .route(
            "/agents/{key}/default-template/{template_key}",
            put(set_default_template),
        )
        .route("/agents/{key}/templates", get(list_agent_templates))
        .route(
            "/agents/{key}/templates/{template_key}",
            put(assign_template).delete(unlink_agent_template),
        )
        .route("/templates", get(list_templates).post(create_template))
        .route(
            "/templates/{key}",
            get(get_template)
                .patch(patch_template)
                .delete(delete_template),
        )
        .route("/templates/{key}/agents", get(list_template_agents))
        .route("/templates/{key}/providers", get(list_template_providers))
        .route(
            "/templates/{key}/providers/{provider_type}",
            put(bind_template_provider).delete(unlink_template_provider),
        )
        .route("/providers", get(list_providers).post(create_provider))
        .route(
            "/providers/{key}",
            get(get_provider)
                .patch(patch_provider)
                .delete(delete_provider),
        )
        .route(
            "/providers/{key}/test/speaker",
            axum::routing::post(provider_tests::test_speaker_provider),
        )
        .route("/providers/{key}/templates", get(list_provider_templates))
        .route(
            "/providers/{key}/prepare",
            axum::routing::post(provider_tests::prepare_provider),
        )
        .route(
            "/providers/{key}/test/llm",
            axum::routing::post(test_llm_provider),
        )
        .route(
            "/providers/{key}/test/tts",
            axum::routing::post(test_tts_provider),
        )
        .route(
            "/providers/{key}/test/asr",
            axum::routing::post(test_asr_provider),
        )
        .route(
            "/providers/{key}/test/vad",
            axum::routing::post(test_vad_provider),
        )
        .route(
            "/providers/{key}/capabilities",
            get(get_provider_capabilities),
        )
        .route("/provider-adapters", get(list_provider_adapters))
        .route("/provider-adapters/{adapter}", get(get_provider_adapter))
        .route(
            "/provider-adapters/{adapter}/capabilities/discover",
            axum::routing::post(discover_provider_capabilities),
        )
        .route(
            "/mcp-servers",
            get(list_mcp_servers).post(create_mcp_server),
        )
        .route(
            "/mcp-servers/{key}",
            get(get_mcp_server)
                .patch(patch_mcp_server)
                .delete(delete_mcp_server),
        )
        .route("/agents/{key}/mcp-bindings", get(list_agent_mcp_bindings))
        .route(
            "/agents/{key}/mcp-bindings/{server_key}",
            put(put_agent_mcp_binding).delete(unlink_agent_mcp_binding),
        )
        .route(
            "/agents/{key}/tool-allowlist",
            get(tool_allowlist::list).put(tool_allowlist::review),
        )
        .route(
            "/agents/{key}/device-tool-allowlist",
            get(device_tools::list).put(device_tools::review),
        )
        .route(
            "/agents/{key}/device-tool-recovery",
            axum::routing::post(device_tools::start_recovery),
        )
        .route("/devices", get(list_devices).post(create_device))
        .route(
            "/device-enrollments/claim",
            axum::routing::post(claim_enrollment),
        )
        .route(
            "/devices/{device_id}",
            get(get_device).patch(patch_device).delete(delete_device),
        )
        .route("/history", get(list_history))
        .route("/history/purge", axum::routing::post(purge_history))
        .route("/speaker-recognition", get(speakers::summary))
        .route("/speakers", get(speakers::list).post(speakers::create))
        .route(
            "/speakers/{key}",
            get(speakers::get)
                .patch(speakers::patch)
                .delete(speakers::delete),
        )
        .route(
            "/speakers/{key}/enrollments",
            axum::routing::post(speakers::create_draft),
        )
        .route(
            "/speakers/{key}/enrollments/{id}",
            get(speakers::get_draft).delete(speakers::cancel_draft),
        )
        .route(
            "/speakers/{key}/enrollments/{id}/samples/{slot}",
            put(speakers::put_sample).delete(speakers::delete_sample),
        )
        .route(
            "/speakers/{key}/enrollments/{id}/validate",
            axum::routing::post(speakers::validate_holdout),
        )
        .route(
            "/speakers/{key}/enrollments/{id}/finalize",
            axum::routing::post(speakers::finalize),
        )
        .route(
            "/agents/{key}/speaker-policy",
            get(speaker_policy::get_policy).put(speaker_policy::put_policy),
        )
        .route(
            "/agents/{key}/speakers",
            get(speaker_policy::list_agent_speakers),
        )
        .route(
            "/agents/{key}/speakers/{speaker_key}",
            put(speaker_policy::put_agent_speaker).delete(speaker_policy::delete_agent_speaker),
        )
        .route(
            "/speakers/{key}/bindings",
            get(speaker_policy::list_speaker_bindings),
        )
        .route(
            "/speaker-recognition/reload",
            axum::routing::post(speaker_calibration::reload),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            authenticate,
        ))
        .layer(axum::middleware::from_fn(transport))
        .layer(axum::middleware::from_fn(request_id))
        .with_state(state)
}

mod agents;
mod deletion;
mod device_tools;
mod devices;
mod enrollments;
mod history;
mod mcp_servers;
mod provider_adapters;
mod provider_tests;
mod providers;
mod speakers;
mod system;
pub(crate) use speakers::cleanup_expired_drafts as cleanup_expired_speaker_drafts;
mod speaker_calibration;
mod speaker_policy;
mod templates;
mod tool_allowlist;

use agents::{create_agent, get_agent, list_agents, patch_agent};
use deletion::{delete_agent, delete_device, delete_mcp_server, delete_provider, delete_template};
use devices::{create_device, get_device, list_devices, patch_device};
use enrollments::claim as claim_enrollment;
use history::{list_history, purge_history};
use mcp_servers::{
    create_mcp_server, get_mcp_server, list_agent_mcp_bindings, list_mcp_servers, patch_mcp_server,
    put_agent_mcp_binding, unlink_agent_mcp_binding,
};
use provider_adapters::{
    discover_provider_capabilities, get_provider_adapter, get_provider_capabilities,
    list_provider_adapters,
};
use provider_tests::{test_asr_provider, test_llm_provider, test_tts_provider, test_vad_provider};
use providers::{
    create_provider, get_provider, list_provider_templates, list_providers, patch_provider,
};
use system::get_system;
use templates::{
    assign_template, bind_template_provider, create_template, get_template, list_agent_templates,
    list_template_agents, list_template_providers, list_templates, patch_template,
    set_default_template, unlink_agent_template, unlink_template_provider,
};

async fn transport(request: Request, next: Next) -> Response {
    if request
        .headers()
        .get(header::CONTENT_ENCODING)
        .is_some_and(|value| value != "identity")
    {
        return error(
            &request,
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_content_encoding",
        );
    }
    let is_mutation = matches!(
        request.method(),
        &http::Method::POST | &http::Method::PATCH | &http::Method::PUT
    );
    if is_mutation {
        let max_body = if request.uri().path().ends_with("/test/asr") {
            MAX_ASR_TEST_BODY
        } else if request.uri().path().contains("/enrollments/")
            && request.uri().path().contains("/samples/")
        {
            MAX_SPEAKER_SAMPLE_BODY
        } else {
            MAX_BODY
        };
        if let Some(length) = request
            .headers()
            .get(header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<usize>().ok())
            && length > max_body
        {
            return error(&request, StatusCode::PAYLOAD_TOO_LARGE, "request_too_large");
        }
    }
    next.run(request).await
}

async fn request_id(mut request: Request, next: Next) -> Response {
    let id = Uuid::new_v4().to_string();
    request.extensions_mut().insert(RequestId(id.clone()));
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        "x-request-id",
        HeaderValue::from_str(&id).expect("uuid header"),
    );
    response
}

async fn authenticate(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let valid = request
        .headers()
        .get_all(header::AUTHORIZATION)
        .iter()
        .count()
        == 1
        && request
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .is_some_and(|token| {
                !token.is_empty()
                    && constant_time_eq(token.as_bytes(), state.config.api.admin_token.as_bytes())
            });
    if valid {
        next.run(request).await
    } else {
        error(&request, StatusCode::UNAUTHORIZED, "unauthorized")
    }
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut diff = left.len() ^ right.len();
    for index in 0..left.len().max(right.len()) {
        diff |= usize::from(*left.get(index).unwrap_or(&0) ^ *right.get(index).unwrap_or(&0));
    }
    diff == 0
}

#[derive(Clone)]
struct RequestId(String);
fn id(request: &Request) -> &str {
    request
        .extensions()
        .get::<RequestId>()
        .map(|v| v.0.as_str())
        .unwrap_or("unknown")
}
fn error(request: &Request, status: StatusCode, code: &'static str) -> Response {
    (
        status,
        Json(serde_json::json!({"error":{"code":code,"request_id":id(request)}})),
    )
        .into_response()
}
#[allow(clippy::result_large_err)] // Axum handlers return the response directly on this boundary.
fn db(state: &AppState) -> Result<&SqlitePool, Response> {
    state.database.as_ref().map(|db| db.pool()).ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error":{"code":"database_unavailable"}})),
        )
            .into_response()
    })
}
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
fn sql_error_kind(error: &sqlx::Error) -> &'static str {
    match error {
        sqlx::Error::PoolTimedOut => "database_unavailable",
        sqlx::Error::Database(error)
            if matches!(
                error.code().as_deref(),
                Some("5" | "6" | "SQLITE_BUSY" | "SQLITE_LOCKED")
            ) =>
        {
            "database_busy"
        }
        _ => "database_unavailable",
    }
}
fn sql_error(request: &Request, sql_error_value: &sqlx::Error) -> Response {
    error(
        request,
        StatusCode::SERVICE_UNAVAILABLE,
        sql_error_kind(sql_error_value),
    )
}
fn mutation_sql_error(request: &Request, sql_error_value: &sqlx::Error) -> Response {
    if sql_error_kind(sql_error_value) == "database_busy" {
        sql_error(request, sql_error_value)
    } else {
        error(request, StatusCode::CONFLICT, "resource_conflict")
    }
}

#[allow(clippy::result_large_err)] // Preserves the request so callers can attach its request ID.
async fn json<T: DeserializeOwned>(request: Request) -> Result<(Request, T), Response> {
    if request
        .headers()
        .get(header::CONTENT_ENCODING)
        .is_some_and(|v| v != "identity")
    {
        return Err(error(
            &request,
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_content_encoding",
        ));
    }
    let content_type = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let mut content_parts = content_type.split(';');
    let media_type_ok = content_parts
        .next()
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("application/json"));
    let parameters_ok = content_parts.all(|parameter| {
        let parameter = parameter.trim();
        !parameter.is_empty()
            && parameter
                .split_once('=')
                .is_some_and(|(name, value)| !name.trim().is_empty() && !value.trim().is_empty())
    });
    if !media_type_ok || !parameters_ok {
        return Err(error(
            &request,
            StatusCode::BAD_REQUEST,
            "invalid_content_type",
        ));
    }
    let (parts, body) = request.into_parts();
    let bytes = match to_bytes(body, MAX_BODY).await {
        Ok(bytes) => bytes,
        Err(_) => {
            let request = Request::from_parts(parts, Body::empty());
            return Err(error(
                &request,
                StatusCode::PAYLOAD_TOO_LARGE,
                "request_too_large",
            ));
        }
    };
    let request = Request::from_parts(parts, Body::empty());
    match serde_json::from_slice(&bytes) {
        Ok(payload) => Ok((request, payload)),
        Err(_) => Err(error(&request, StatusCode::BAD_REQUEST, "invalid_json")),
    }
}

#[derive(Default, Deserialize)]
#[serde(untagged)]
enum Patch<T> {
    Clear(Option<T>),
    #[default]
    Absent,
}
impl<T> Patch<T> {
    fn value(self) -> Option<Option<T>> {
        match self {
            Self::Absent => None,
            Self::Clear(v) => Some(v),
        }
    }
}
#[derive(Deserialize)]
struct PageQuery {
    #[serde(default)]
    page: Option<u32>,
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    sort: Option<String>,
    #[serde(default)]
    page_size: Option<u32>,
}

/// The shared page window.  It takes the two bounds rather than a resource query so every list
/// surface can declare only the filters it actually has.
fn page_bounds(page: Option<u32>, page_size: Option<u32>) -> Result<(u32, u32), &'static str> {
    let page = page.unwrap_or(1);
    let page_size = page_size.unwrap_or(PAGE_DEFAULT);
    if page == 0 || page > 10_000 || !(1..=PAGE_MAX).contains(&page_size) {
        Err("invalid_query")
    } else {
        Ok((page, page_size))
    }
}

fn valid_key(value: &str) -> bool {
    value.len() <= 64
        && value.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}
fn valid_text(value: &str, max: usize, allow_empty: bool) -> bool {
    value.len() <= max && (allow_empty || !value.trim().is_empty())
}
fn valid_identity(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && value.bytes().all(|b| b > 0x1f && b != 0x7f)
}
fn valid_metadata(value: &Value) -> bool {
    serde_json::to_vec(value).is_ok_and(|v| v.len() <= 16 * 1024) && json_shape(value, 0, &mut 0)
}
fn json_shape(value: &Value, depth: u8, nodes: &mut u16) -> bool {
    if depth > 8 {
        return false;
    };
    match value {
        Value::Array(items) => items.iter().all(|item| {
            *nodes += 1;
            *nodes <= 256 && json_shape(item, depth + 1, nodes)
        }),
        Value::Object(items) => items.values().all(|item| {
            *nodes += 1;
            *nodes <= 256 && json_shape(item, depth + 1, nodes)
        }),
        _ => true,
    }
}
fn expected(headers: &HeaderMap) -> Result<i64, &'static str> {
    let value = headers
        .get(header::IF_MATCH)
        .and_then(|v| v.to_str().ok())
        .ok_or("invalid_if_match")?;
    value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .and_then(|v| v.parse().ok())
        .filter(|v: &i64| *v > 0)
        .ok_or("invalid_if_match")
}
/// What an administration did, and therefore what its audit row records.
///
/// The outcome and the reason a row failed travel as one value because they are never independent:
/// a mismatched pair would be representable as two free strings and meaningless in practice.
#[derive(Clone, Copy)]
pub(super) enum AuditOutcome {
    Success,
    RevisionConflict,
}

impl AuditOutcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::RevisionConflict => "conflict",
        }
    }

    /// The only reason kind V1 records, and it belongs to exactly one outcome.
    fn error_kind(self) -> Option<&'static str> {
        match self {
            Self::Success => None,
            Self::RevisionConflict => Some("revision_conflict"),
        }
    }
}

/// One audit row: enough to account for an administration without recording any of it.
///
/// `resource_id` is `None` for an operation that stands behind no single resource row — a scoped
/// History Purge names a scope rather than a resource and has no revision pair, so its `action` and
/// `affected_rows` are the only thing that identifies it.  Nothing about the text a purge deleted
/// does: a purge must leave a countable trace, not a second copy of what it removed.
#[allow(clippy::too_many_arguments)] // Each argument maps one-to-one to the immutable audit schema.
async fn audit<'e, E>(
    executor: E,
    request_id: &str,
    resource: &str,
    resource_id: Option<i64>,
    action: &str,
    prior: Option<i64>,
    new: Option<i64>,
    outcome: AuditOutcome,
    affected_rows: u64,
) -> Result<(), sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query("INSERT INTO admin_audit_events (created_at,request_id,resource_type,resource_id,action,prior_revision,new_revision,outcome,error_kind,affected_rows) VALUES (?,?,?,?,?,?,?,?,?,?)")
        .bind(now())
        .bind(request_id)
        .bind(resource)
        .bind(resource_id)
        .bind(action)
        .bind(prior)
        .bind(new)
        .bind(outcome.as_str())
        .bind(outcome.error_kind())
        .bind(affected_rows as i64)
        .execute(executor)
        .await
        .map(|_| ())
}

async fn audit_conflict(
    pool: &SqlitePool,
    request_id: String,
    resource: &str,
    resource_id: i64,
    expected: i64,
) {
    audit_conflict_action(pool, request_id, resource, resource_id, expected, "update").await;
}

async fn audit_conflict_action(
    pool: &SqlitePool,
    request_id: String,
    resource: &str,
    resource_id: i64,
    expected: i64,
    action: &str,
) {
    let _ = audit(
        pool,
        &request_id,
        resource,
        Some(resource_id),
        action,
        Some(expected),
        None,
        AuditOutcome::RevisionConflict,
        1,
    )
    .await;
}
