use super::*;
use crate::database::backup::{
    BackupError, BackupFormat, BackupImportOptions, BackupScope, ExportBackupRequest,
    ImportBackupRequest, ImportMode, InspectBackupRequest, MAX_BACKUP_BODY_BYTES,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ExportBackupQuery {
    #[serde(default)]
    format: Option<String>,
    #[serde(default)]
    scope: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ImportBackupQuery {
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    clear_unreadable_credentials: Option<bool>,
}

fn parse_import_options(query: ImportBackupQuery) -> Result<BackupImportOptions, &'static str> {
    let mode = match query.mode.as_deref() {
        Some(raw) => ImportMode::parse(raw).ok_or("invalid_query")?,
        None => ImportMode::Merge,
    };
    let scope = match query.scope.as_deref() {
        Some(raw) => BackupScope::parse(raw).ok_or("invalid_query")?,
        None => BackupScope::Full,
    };
    let clear_unreadable_credentials = query.clear_unreadable_credentials.unwrap_or(true);
    Ok(BackupImportOptions {
        mode,
        scope,
        clear_unreadable_credentials,
    })
}

fn map_backup_error(request: &Request, err: BackupError) -> Response {
    match err {
        BackupError::EmptyPayload => error(request, StatusCode::BAD_REQUEST, "backup_payload_empty"),
        BackupError::PayloadTooLarge => {
            error(request, StatusCode::PAYLOAD_TOO_LARGE, "backup_payload_too_large")
        }
        BackupError::InvalidFormat => {
            error(request, StatusCode::BAD_REQUEST, "backup_invalid_format")
        }
        BackupError::ChecksumMismatch => {
            error(request, StatusCode::BAD_REQUEST, "backup_checksum_mismatch")
        }
        BackupError::UnsupportedVersion => {
            error(request, StatusCode::BAD_REQUEST, "backup_unsupported_version")
        }
        BackupError::UnreadableCredentials => error(
            request,
            StatusCode::UNPROCESSABLE_ENTITY,
            "backup_unreadable_credentials",
        ),
        BackupError::InvalidReference(_) => {
            error(request, StatusCode::UNPROCESSABLE_ENTITY, "backup_reference_invalid")
        }
        BackupError::Busy => error(request, StatusCode::SERVICE_UNAVAILABLE, "database_busy"),
        BackupError::Unavailable => error(
            request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        ),
    }
}

pub(in crate::app::admin) async fn export_backup(
    State(state): State<AppState>,
    request: Request,
) -> Response {
    let Some(db) = state.database.as_ref() else {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        );
    };
    let Query(query) = match Query::<ExportBackupQuery>::try_from_uri(request.uri()) {
        Ok(q) => q,
        Err(_) => return error(&request, StatusCode::BAD_REQUEST, "invalid_query"),
    };
    let format = match query.format.as_deref() {
        Some(raw) => match BackupFormat::parse(raw) {
            Some(f) => f,
            None => return error(&request, StatusCode::BAD_REQUEST, "invalid_query"),
        },
        None => BackupFormat::Json,
    };
    let scope = match query.scope.as_deref() {
        Some(raw) => match BackupScope::parse(raw) {
            Some(s) => s,
            None => return error(&request, StatusCode::BAD_REQUEST, "invalid_query"),
        },
        None => BackupScope::Full,
    };
    let companion_settings = state.load_backup_companion_settings(scope);
    match db
        .export_backup(ExportBackupRequest {
            format,
            scope,
            companion_settings,
        })
        .await
    {
        Ok(exported) => {
            let mut response = Response::new(Body::from(exported.bytes));
            *response.status_mut() = StatusCode::OK;
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static(exported.content_type),
            );
            if let Ok(disposition) = HeaderValue::from_str(&format!(
                "attachment; filename=\"{}\"",
                exported.filename
            )) {
                response
                    .headers_mut()
                    .insert(header::CONTENT_DISPOSITION, disposition);
            }
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
        }
        Err(err) => map_backup_error(&request, err),
    }
}

async fn extract_import_payload(
    request: Request,
) -> Result<(Request, String, BackupImportOptions, axum::body::Bytes), Response> {
    let Query(query) = match Query::<ImportBackupQuery>::try_from_uri(request.uri()) {
        Ok(q) => q,
        Err(_) => return Err(error(&request, StatusCode::BAD_REQUEST, "invalid_query")),
    };
    let options = match parse_import_options(query) {
        Ok(opts) => opts,
        Err(code) => return Err(error(&request, StatusCode::BAD_REQUEST, code)),
    };
    let req_id = id(&request).to_owned();
    let (parts, body) = request.into_parts();
    let req_head = Request::from_parts(parts, Body::empty());
    let bytes = match to_bytes(body, MAX_BACKUP_BODY_BYTES).await {
        Ok(b) => b,
        Err(_) => {
            return Err(error(
                &req_head,
                StatusCode::PAYLOAD_TOO_LARGE,
                "backup_payload_too_large",
            ));
        }
    };
    Ok((req_head, req_id, options, bytes))
}

pub(in crate::app::admin) async fn inspect_backup(
    State(state): State<AppState>,
    request: Request,
) -> Response {
    let Some(db) = state.database.as_ref() else {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        );
    };
    let (req_head, _req_id, options, bytes) = match extract_import_payload(request).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    match db
        .inspect_backup(InspectBackupRequest {
            payload: &bytes,
            options,
            secret_resolver: state.secret_resolver.as_ref(),
        })
        .await
    {
        Ok(report) => {
            let mut response = Json(report).into_response();
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
        }
        Err(err) => map_backup_error(&req_head, err),
    }
}

pub(in crate::app::admin) async fn import_backup(
    State(state): State<AppState>,
    request: Request,
) -> Response {
    let Some(db) = state.database.as_ref() else {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        );
    };
    let (req_head, req_id, options, bytes) = match extract_import_payload(request).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    match db
        .import_backup(ImportBackupRequest {
            request_id: &req_id,
            payload: &bytes,
            options,
            secret_resolver: state.secret_resolver.as_ref(),
        })
        .await
    {
        Ok(mut outcome) => {
            state.apply_backup_import_outcome(&mut outcome).await;
            let mut response = Json(outcome.report).into_response();
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
        }
        Err(err) => map_backup_error(&req_head, err),
    }
}
