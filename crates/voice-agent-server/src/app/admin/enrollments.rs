//! Admin-side, authenticated consumption of an activation code.
use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClaimEnrollment {
    code: String,
    agent_key: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    template_key: Option<String>,
}

pub(super) async fn claim(State(state): State<AppState>, request: Request) -> Response {
    if !state.config.database.devices.enrollment.enabled {
        return error(&request, StatusCode::NOT_FOUND, "not_found");
    }
    let (request, body): (_, ClaimEnrollment) = match json(request).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    if !valid_code(&body.code)
        || !valid_key(&body.agent_key)
        || body
            .template_key
            .as_ref()
            .is_some_and(|key| !valid_key(key))
        || body
            .name
            .as_ref()
            .is_some_and(|name| !valid_text(name, 128, true))
    {
        return error(&request, StatusCode::BAD_REQUEST, "validation_failed");
    }
    if !state.admission_gate().is_open() {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "server_shutting_down",
        );
    }
    let database = match database(&state) {
        Ok(pool) => pool,
        Err(_) => {
            return error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            );
        }
    };
    match database
        .claim_device_enrollment(
            crate::database::device_enrollments::AdminEnrollmentClaim {
                code: &body.code,
                agent_key: &body.agent_key,
                name: body.name.as_deref(),
                template_key: body.template_key.as_deref(),
            },
            id(&request),
        )
        .await
    {
        Ok(device) => (StatusCode::CREATED, Json(device)).into_response(),
        Err(cause) => write_error(&request, cause),
    }
}

fn valid_code(value: &str) -> bool {
    value.len() == 6 && value.bytes().all(|byte| byte.is_ascii_digit())
}
