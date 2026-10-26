use crate::{
    app::{AppState, router_with_state},
    config::AppConfig,
    providers::ProviderSet,
};
use llm_wiki::WikiEngine;
use reqwest::StatusCode;
use serde_json::Value;
use std::sync::Arc;

async fn fixture_server() -> (
    String,
    std::path::PathBuf,
    tokio::task::JoinHandle<()>,
) {
    let temp_root =
        std::env::temp_dir().join(format!("knowledge-search-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&temp_root).unwrap();
    let config: AppConfig = toml::from_str(
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
admin_token = "knowledge-test-token"
"#,
    )
    .unwrap();
    let engine = Arc::new(WikiEngine::open(temp_root.join("vault")).unwrap());
    engine
        .write_wiki_file(
            "concepts/hybrid-search.md",
            "---\ntitle: \"Hybrid Search Architecture\"\n---\n\n# Hybrid Search Architecture\n\nReciprocal rank fusion combines lexical BM25 and dense vectors.\n",
        )
        .unwrap();
    engine
        .write_source_file(
            "notes/field-observation.md",
            "# Field Observation\n\nAcoustic barge-in latency was measured at 42 milliseconds in the lab.\n",
        )
        .unwrap();
    let _ = engine.embed_pending(true);

    let mut state = AppState::from_provider_set_with_database(
        config,
        Arc::new(ProviderSet::unavailable()),
        None,
    );
    state.wiki_engine = Some(engine);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = router_with_state(state);
    let handle = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (format!("http://{addr}/api/admin"), temp_root, handle)
}

#[tokio::test]
async fn knowledge_search_scopes_by_browser_space_and_returns_normalized_paths_and_match_lines() {
    let (base, temp_root, handle) = fixture_server().await;
    let client = reqwest::Client::new();

    // 1. Default / wiki space finds Wiki Page and returns space="wiki", path="wiki/...", match_line > 0
    let wiki_resp = client
        .get(format!(
            "{base}/knowledge/pages?q=Reciprocal%20rank%20fusion&space=wiki"
        ))
        .bearer_auth("knowledge-test-token")
        .send()
        .await
        .unwrap();
    assert_eq!(wiki_resp.status(), StatusCode::OK);
    let wiki_body: Value = wiki_resp.json().await.unwrap();
    let wiki_items = wiki_body["items"].as_array().unwrap();
    assert!(!wiki_items.is_empty());
    assert!(wiki_items.iter().all(|item| item["space"] == "wiki"));
    assert_eq!(wiki_items[0]["id"], "wiki/concepts/hybrid-search.md");
    assert_eq!(wiki_items[0]["space"], "wiki");
    assert_eq!(wiki_items[0]["path"], "wiki/concepts/hybrid-search.md");
    assert_eq!(wiki_items[0]["title"], "Hybrid Search Architecture");
    assert!(wiki_items[0]["match_line"].as_u64().unwrap_or(0) >= 1);
    assert!(
        wiki_items[0]["excerpt"]
            .as_str()
            .unwrap_or("")
            .contains("Reciprocal rank fusion")
    );

    // Raw Working Note should not appear when searching space=wiki
    let raw_in_wiki = client
        .get(format!(
            "{base}/knowledge/pages?q=42%20milliseconds&space=wiki&mode=lex"
        ))
        .bearer_auth("knowledge-test-token")
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(raw_in_wiki["items"].as_array().unwrap().len(), 0);

    // 2. space=raw searches Raw Working Notes in sources/ and normalizes path to "raw/..."
    let raw_resp = client
        .get(format!(
            "{base}/knowledge/pages?q=42%20milliseconds&space=raw&mode=lex"
        ))
        .bearer_auth("knowledge-test-token")
        .send()
        .await
        .unwrap();
    assert_eq!(raw_resp.status(), StatusCode::OK);
    let raw_body: Value = raw_resp.json().await.unwrap();
    let raw_items = raw_body["items"].as_array().unwrap();
    assert_eq!(raw_items.len(), 1);
    assert_eq!(raw_items[0]["space"], "raw");
    assert_eq!(raw_items[0]["path"], "raw/notes/field-observation.md");
    assert_eq!(raw_items[0]["title"], "Field Observation");
    assert!(raw_items[0]["match_line"].as_u64().unwrap_or(0) >= 1);
    assert!(
        raw_items[0]["excerpt"]
            .as_str()
            .unwrap_or("")
            .contains("42 milliseconds")
    );

    // 3. Invalid space or mode is rejected with 400 Bad Request
    let bad_space = client
        .get(format!("{base}/knowledge/pages?q=test&space=unknown"))
        .bearer_auth("knowledge-test-token")
        .send()
        .await
        .unwrap();
    assert_eq!(bad_space.status(), StatusCode::BAD_REQUEST);

    let bad_mode = client
        .get(format!("{base}/knowledge/pages?q=test&mode=invalid"))
        .bearer_auth("knowledge-test-token")
        .send()
        .await
        .unwrap();
    assert_eq!(bad_mode.status(), StatusCode::BAD_REQUEST);

    let bad_mode_without_q = client
        .get(format!("{base}/knowledge/pages?mode=invalid"))
        .bearer_auth("knowledge-test-token")
        .send()
        .await
        .unwrap();
    assert_eq!(bad_mode_without_q.status(), StatusCode::BAD_REQUEST);

    handle.abort();
    let _ = std::fs::remove_dir_all(temp_root);
}
