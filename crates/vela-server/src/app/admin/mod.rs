//! Bounded, separately authenticated Admin HTTP surface.

mod agents;
mod background;
mod common;
mod deletion;
mod devices;
mod history;
mod knowledge;
mod llm_gateway;
mod mcp;
mod middleware;
mod notifications;
mod providers;
mod router;
mod speakers;
mod system;
mod templates;

pub use common::Patch;
pub(super) use router::router;

use super::AppState;
use crate::database::{external_mcp_policy, provider_config};
use axum::{
    Json,
    body::{Body, to_bytes},
    extract::{Path, Query, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use common::{
    MAX_ASR_TEST_BODY, MAX_BODY, MAX_JOB_CREATE_BODY, MAX_QUICK_CAPTURE_BODY,
    MAX_SPEAKER_SAMPLE_BODY, PAGE_MAX, PageQuery, database, error, expected, json, json_with_limit,
    now, page_bounds, security, sql_error, valid_identity, valid_key, valid_metadata, valid_text,
    write_error,
};
use middleware::{authenticate, id, request_id, transport};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;
