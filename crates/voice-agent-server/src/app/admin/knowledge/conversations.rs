use super::*;
use axum::extract::Path;
use serde::Deserialize;

fn no_store(mut response: Response) -> Response {
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RenameConversationInput {
    title: String,
}

pub(in crate::app::admin) async fn knowledge_conversations(
    State(state): State<AppState>,
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
    match crate::database::wiki_conversations::list_conversations(database).await {
        Ok(items) => no_store(Json(serde_json::json!({ "items": items })).into_response()),
        Err(_) => no_store(error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        )),
    }
}

pub(in crate::app::admin) async fn rename_knowledge_conversation(
    State(state): State<AppState>,
    Path(conversation_id): Path<String>,
    request: Request,
) -> Response {
    let (request, input_value): (_, serde_json::Value) = match json(request).await {
        Ok(parsed) => parsed,
        Err(response) => return no_store(response),
    };
    let input: RenameConversationInput = match serde_json::from_value(input_value) {
        Ok(input) => input,
        Err(_) => return no_store(error(&request, StatusCode::BAD_REQUEST, "invalid_input")),
    };
    let title = input.title.trim();
    if !(1..=120).contains(&title.chars().count()) {
        return no_store(error(&request, StatusCode::BAD_REQUEST, "invalid_input"));
    }
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
    match crate::database::wiki_conversations::rename_conversation(database, &conversation_id, title).await {
        Ok(true) => no_store(StatusCode::NO_CONTENT.into_response()),
        Ok(false) => no_store(error(
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

pub(in crate::app::admin) async fn delete_knowledge_conversation(
    State(state): State<AppState>,
    Path(conversation_id): Path<String>,
    request: Request,
) -> Response {
    let request_id = id(&request).to_owned();
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
    match crate::database::wiki_conversations::delete_conversation(database, &conversation_id, &request_id)
        .await
    {
        Ok(()) => no_store(StatusCode::NO_CONTENT.into_response()),
        Err(crate::database::wiki_conversations::DeleteConversationError::NotFound) => {
            no_store(error(
                &request,
                StatusCode::NOT_FOUND,
                "wiki_conversation_not_found",
            ))
        }
        Err(crate::database::wiki_conversations::DeleteConversationError::Busy) => no_store(error(
            &request,
            StatusCode::CONFLICT,
            "wiki_conversation_busy",
        )),
        Err(crate::database::wiki_conversations::DeleteConversationError::Sql(error_value)) => {
            no_store(sql_error(&request, &error_value))
        }
    }
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

#[cfg(test)]
mod tests {
    use crate::{
        app::{AppState, router_with_state},
        config::{AppConfig, DatabaseConfig},
        database::Database,
        providers::ProviderSet,
    };
    use reqwest::StatusCode;
    use serde_json::{Value, json};
    use std::sync::Arc;

    async fn fixture() -> (String, Database, std::path::PathBuf, tokio::task::JoinHandle<()>) {
        let root = std::env::temp_dir().join(format!(
            "wiki-conversations-routes-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let config = DatabaseConfig {
            url: format!("sqlite://{}", root.join("test.db").display()),
            ..DatabaseConfig::default()
        };
        let database = Database::connect(&config).await.unwrap();
        let mut app_config: AppConfig = toml::from_str(
            r#"
[server]
bind = "127.0.0.1:0"
public_ws_url = "ws://127.0.0.1:0/voice/v1/"
[provider_defaults]
vad = "test"
asr = "test"
llm = "test"
tts = "test"
[api]
enabled = true
admin_token = "wiki-conversations-test-token"
"#,
        )
        .unwrap();
        app_config.database = config;
        let state = AppState::from_provider_set_with_database(
            app_config,
            Arc::new(ProviderSet::unavailable()),
            Some(database.clone()),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, router_with_state(state)).await.unwrap();
        });
        (
            format!("http://{address}/api/admin"),
            database,
            root,
            task,
        )
    }

    async fn seed_conversation(database: &Database, id: &str, title: &str, updated_at: i64) {
        sqlx::query("INSERT INTO wiki_conversations (id, title, created_at_ms, updated_at_ms) VALUES (?, ?, ?, ?)")
            .bind(id)
            .bind(title)
            .bind(updated_at)
            .bind(updated_at)
            .execute(database.pool())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn conversation_routes_list_rename_and_delete_without_exposing_message_content() {
        let (base, database, root, server) = fixture().await;
        seed_conversation(&database, "older", "Older research", 10).await;
        seed_conversation(&database, "latest", "Latest research", 20).await;
        sqlx::query("INSERT INTO wiki_conversation_messages (id, conversation_id, seq, role, content, status, created_at_ms, updated_at_ms) VALUES ('older-running', 'older', 1, 'assistant', '', 'running', 10, 10)")
            .execute(database.pool())
            .await
            .unwrap();
        sqlx::query("INSERT INTO wiki_conversation_messages (id, conversation_id, seq, role, content, status, created_at_ms, updated_at_ms) VALUES ('latest-user', 'latest', 1, 'user', 'private question text', 'done', 20, 20)")
            .execute(database.pool())
            .await
            .unwrap();

        let client = reqwest::Client::new();
        let list = client
            .get(format!("{base}/knowledge/conversations"))
            .bearer_auth("wiki-conversations-test-token")
            .send()
            .await
            .unwrap();
        assert_eq!(list.status(), StatusCode::OK);
        assert_eq!(list.headers()["cache-control"], "no-store");
        let body: Value = list.json().await.unwrap();
        assert_eq!(body["items"][0]["id"], "latest");
        assert_eq!(body["items"][0]["title"], "Latest research");
        assert_eq!(body["items"][0]["message_count"], 1);
        assert_eq!(body["items"][0]["running"], false);
        assert_eq!(body["items"][1]["id"], "older");
        assert_eq!(body["items"][1]["message_count"], 1);
        assert_eq!(body["items"][1]["running"], true);
        assert!(!body.to_string().contains("private question text"));

        let missing_auth = client
            .get(format!("{base}/knowledge/conversations"))
            .send()
            .await
            .unwrap();
        assert_eq!(missing_auth.status(), StatusCode::UNAUTHORIZED);

        let renamed = client
            .patch(format!("{base}/knowledge/conversations/latest"))
            .bearer_auth("wiki-conversations-test-token")
            .json(&json!({ "title": "  Field notes  " }))
            .send()
            .await
            .unwrap();
        assert_eq!(renamed.status(), StatusCode::NO_CONTENT);
        assert_eq!(renamed.headers()["cache-control"], "no-store");
        let invalid_title = client
            .patch(format!("{base}/knowledge/conversations/latest"))
            .bearer_auth("wiki-conversations-test-token")
            .json(&json!({ "title": " " }))
            .send()
            .await
            .unwrap();
        assert_eq!(invalid_title.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            invalid_title.json::<Value>().await.unwrap()["error"]["code"],
            "invalid_input"
        );
        let too_long_title = client
            .patch(format!("{base}/knowledge/conversations/latest"))
            .bearer_auth("wiki-conversations-test-token")
            .json(&json!({ "title": "é".repeat(121) }))
            .send()
            .await
            .unwrap();
        assert_eq!(too_long_title.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            too_long_title.json::<Value>().await.unwrap()["error"]["code"],
            "invalid_input"
        );

        let missing_patch_auth = client
            .patch(format!("{base}/knowledge/conversations/latest"))
            .json(&json!({ "title": "Renamed" }))
            .send()
            .await
            .unwrap();
        assert_eq!(missing_patch_auth.status(), StatusCode::UNAUTHORIZED);

        let deleted = client
            .delete(format!("{base}/knowledge/conversations/latest"))
            .bearer_auth("wiki-conversations-test-token")
            .send()
            .await
            .unwrap();
        assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
        assert_eq!(deleted.headers()["cache-control"], "no-store");
        let deleted_message: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM wiki_conversation_messages WHERE conversation_id = 'latest'",
        )
        .fetch_one(database.pool())
        .await
        .unwrap();
        assert_eq!(deleted_message, 0);
        let audit: (String, String, Option<i64>) = sqlx::query_as(
            "SELECT resource_type, action, resource_id FROM admin_audit_events ORDER BY id DESC LIMIT 1",
        )
        .fetch_one(database.pool())
        .await
        .unwrap();
        assert_eq!(audit, ("wiki_conversation".into(), "delete".into(), None));
        let missing = client
            .delete(format!("{base}/knowledge/conversations/latest"))
            .bearer_auth("wiki-conversations-test-token")
            .send()
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
        let missing_delete_auth = client
            .delete(format!("{base}/knowledge/conversations/older"))
            .send()
            .await
            .unwrap();
        assert_eq!(missing_delete_auth.status(), StatusCode::UNAUTHORIZED);
        server.abort();
        drop(database);
        let _ = std::fs::remove_dir_all(root);
    }
}
