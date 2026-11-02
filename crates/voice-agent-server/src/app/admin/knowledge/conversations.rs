use super::*;
use axum::extract::Path;

fn no_store(mut response: Response) -> Response {
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response
}

pub(in crate::app::admin) async fn knowledge_conversation(
    State(state): State<AppState>,
    Path(id): Path<String>,
    request: Request,
) -> Response {
    let database = match database(&state) {
        Ok(database) => database,
        Err(_) => {
            return no_store(error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            ));
        }
    };
    match crate::database::wiki_conversations::get_conversation(database, &id).await {
        Ok(Some(conversation)) => no_store(Json(conversation).into_response()),
        Ok(None) => no_store(error(
            &request,
            StatusCode::NOT_FOUND,
            "wiki_conversation_not_found",
        )),
        Err(_) => no_store(error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        )),
    }
}
