use super::*;
use crate::services::task_flow::skills::{self, Precondition, SaveJobSkill, SkillCatalogError};

fn skill_error_response(request: &Request, err: SkillCatalogError) -> Response {
    match err {
        SkillCatalogError::InvalidId => error(request, StatusCode::BAD_REQUEST, "invalid_skill_id"),
        SkillCatalogError::InvalidArguments(code) => error(request, StatusCode::BAD_REQUEST, code),
        SkillCatalogError::PayloadTooLarge => {
            error(request, StatusCode::PAYLOAD_TOO_LARGE, "skill_too_large")
        }
        SkillCatalogError::NotFound => error(request, StatusCode::NOT_FOUND, "not_found"),
        SkillCatalogError::AlreadyExists => error(request, StatusCode::CONFLICT, "already_exists"),
        SkillCatalogError::RevisionConflict => {
            error(request, StatusCode::CONFLICT, "revision_conflict")
        }
        SkillCatalogError::BuiltinCannotBeDeleted => error(
            request,
            StatusCode::BAD_REQUEST,
            "builtin_skill_cannot_be_deleted",
        ),
        SkillCatalogError::Io => error(
            request,
            StatusCode::SERVICE_UNAVAILABLE,
            "storage_unavailable",
        ),
    }
}

fn parse_skill_etag(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(header::IF_MATCH)?.to_str().ok()?.trim();
    let unquoted = raw
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(raw)
        .trim();
    (!unquoted.is_empty()).then(|| unquoted.to_owned())
}

fn parse_save_precondition(headers: &HeaderMap) -> Result<Precondition, &'static str> {
    if let Some(if_none) = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        && if_none == "*"
    {
        return Ok(Precondition::IfNoneMatchStar);
    }
    if let Some(etag) = parse_skill_etag(headers) {
        return Ok(Precondition::IfMatch(etag));
    }
    Err("invalid_if_match")
}

pub(in crate::app::admin) async fn get_skill(
    State(state): State<AppState>,
    Path(id): Path<String>,
    request: Request,
) -> Response {
    match skills::get(&state, &id) {
        Ok(detail) => {
            let etag = format!("\"{}\"", detail.summary.etag);
            (StatusCode::OK, [(header::ETAG, etag)], Json(detail)).into_response()
        }
        Err(err) => skill_error_response(&request, err),
    }
}

pub(in crate::app::admin) async fn save_skill(
    State(state): State<AppState>,
    Path(id): Path<String>,
    request: Request,
) -> Response {
    let precondition = match parse_save_precondition(request.headers()) {
        Ok(p) => p,
        Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };
    let (request, input): (_, SaveJobSkill) = match json(request).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    match skills::save(&state, &id, input, precondition).await {
        Ok((detail, created)) => {
            let status = if created {
                StatusCode::CREATED
            } else {
                StatusCode::OK
            };
            let etag = format!("\"{}\"", detail.summary.etag);
            (status, [(header::ETAG, etag)], Json(detail)).into_response()
        }
        Err(err) => skill_error_response(&request, err),
    }
}

pub(in crate::app::admin) async fn delete_skill(
    State(state): State<AppState>,
    Path(id): Path<String>,
    request: Request,
) -> Response {
    let Some(expected_etag) = parse_skill_etag(request.headers()) else {
        return error(&request, StatusCode::BAD_REQUEST, "invalid_if_match");
    };
    match skills::delete(&state, &id, &expected_etag).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => skill_error_response(&request, err),
    }
}
