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
        .write_wiki_file(
            "concepts/khong-khi-lanh.md",
            "---\ntitle: \"Không khí lạnh\"\n---\n\n# Không khí lạnh\n\nDự báo mưa rào và dông rải rác tại Đông Bắc Bộ và Thanh Hóa.\n",
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

    // 4. Vietnamese non-diacritic query with d/đ ('Dong Bac Bo') matches 'Đông Bắc Bộ' and extracts exact line
    let viet_resp = client
        .get(format!(
            "{base}/knowledge/pages?q=Dong%20Bac%20Bo&space=wiki&mode=lex"
        ))
        .bearer_auth("knowledge-test-token")
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let viet_items = viet_resp["items"].as_array().unwrap();
    assert!(!viet_items.is_empty());
    assert_eq!(viet_items[0]["path"], "wiki/concepts/khong-khi-lanh.md");
    assert_eq!(viet_items[0]["match_line"], 7);
    assert!(
        viet_items[0]["excerpt"]
            .as_str()
            .unwrap_or("")
            .contains("Đông Bắc Bộ")
    );

    // 5. Nonsense query filters out random vector hash noise in both hybrid and vec modes
    for mode in ["hybrid", "vec", "lex"] {
        let noise_resp = client
            .get(format!(
                "{base}/knowledge/pages?q=zzzzzzzzqqqqqqqq&space=wiki&mode={mode}"
            ))
            .bearer_auth("knowledge-test-token")
            .send()
            .await
            .unwrap()
            .json::<Value>()
            .await
            .unwrap();
        assert_eq!(
            noise_resp["items"].as_array().unwrap().len(),
            0,
            "expected 0 noise items for mode={mode}"
        );
    }

    // 6. System catalog pages (wiki/index.md, wiki/log.md) rank below domain content pages
    let ranked_resp = client
        .get(format!(
            "{base}/knowledge/pages?q=khong%20khi%20lanh&space=wiki&mode=hybrid"
        ))
        .bearer_auth("knowledge-test-token")
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let ranked_items = ranked_resp["items"].as_array().unwrap();
    assert!(!ranked_items.is_empty());
    assert_eq!(ranked_items[0]["path"], "wiki/concepts/khong-khi-lanh.md");

    handle.abort();
    let _ = std::fs::remove_dir_all(temp_root);
}

fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let ty = entry.file_type().unwrap();
        let dst_path = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &dst_path);
        } else if ty.is_file() {
            std::fs::copy(entry.path(), dst_path).unwrap();
        }
    }
}

#[tokio::test]
async fn knowledge_search_evaluates_real_vault_ranking_and_noise_filtering() {
    let real_vault =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/wiki-vault");
    if !real_vault.join("wiki/index.md").exists() {
        return;
    }

    let temp_root =
        std::env::temp_dir().join(format!("knowledge-real-vault-test-{}", uuid::Uuid::new_v4()));
    let vault_copy = temp_root.join("vault");
    copy_dir_recursive(&real_vault, &vault_copy);

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
    let engine = Arc::new(WikiEngine::open(&vault_copy).unwrap());
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
    let base = format!("http://{addr}/api/admin");
    let client = reqwest::Client::new();

    // 1. Nonsense query returns 0 items across hybrid, vec, and lex
    for mode in ["hybrid", "vec", "lex"] {
        let resp: Value = client
            .get(format!(
                "{base}/knowledge/pages?q=zzzzzzzzqqqqqqqq&space=wiki&mode={mode}&limit=5"
            ))
            .bearer_auth("knowledge-test-token")
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(resp["items"].as_array().unwrap().len(), 0);
    }

    // 2. Raw hybrid search for 'pollen robotics my croc' returns only the matching XGO-Duck raw source without tail noise
    let raw_hybrid: Value = client
        .get(format!(
            "{base}/knowledge/pages?q=pollen%20robotics%20my%20croc&space=raw&mode=hybrid&limit=5"
        ))
        .bearer_auth("knowledge-test-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let raw_items = raw_hybrid["items"].as_array().unwrap();
    assert_eq!(raw_items.len(), 1);
    assert_eq!(
        raw_items[0]["path"],
        "raw/robotics/xgo-duck/01-meet-xgo-duck-a-robot-you-can-build.md"
    );

    // 3. Non-diacritic 'Dong Bac Bo' matches 'Đông Bắc Bộ' via d/đ expansion and extracts prose snippet
    let dong_bac: Value = client
        .get(format!(
            "{base}/knowledge/pages?q=Dong%20Bac%20Bo&space=wiki&mode=lex&limit=5"
        ))
        .bearer_auth("knowledge-test-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let dong_bac_items = dong_bac["items"].as_array().unwrap();
    assert!(!dong_bac_items.is_empty());
    assert_eq!(
        dong_bac_items[0]["path"],
        "wiki/sources/news/thoi-tiet-khong-khi-lanh/don-khong-khi-lanh-mien-bac-giam-may-do.md"
    );
    assert!(
        dong_bac_items[0]["excerpt"]
            .as_str()
            .unwrap_or("")
            .contains("Đông Bắc Bộ")
    );

    // 4. Non-diacritic 'khong khi lanh' in hybrid mode ranks domain pages ahead of wiki/index.md and wiki/log.md
    let kkl: Value = client
        .get(format!(
            "{base}/knowledge/pages?q=khong%20khi%20lanh&space=wiki&mode=hybrid&limit=5"
        ))
        .bearer_auth("knowledge-test-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let kkl_items = kkl["items"].as_array().unwrap();
    assert!(kkl_items.len() >= 3);
    for top in &kkl_items[..3] {
        let p = top["path"].as_str().unwrap_or("");
        assert_ne!(p, "wiki/index.md");
        assert_ne!(p, "wiki/log.md");
    }

    handle.abort();
    let _ = std::fs::remove_dir_all(temp_root);
}

#[tokio::test]
async fn knowledge_graph_filters_dangling_edges_aggregates_lines_and_bounds_local_neighborhood() {
    let (base, temp_root, handle) = fixture_server().await;
    let client = reqwest::Client::new();

    // Create Wiki notes with links to another Wiki note (twice), a distant 2-hop Wiki note, and a Source file.
    for (folder, file_name, title, body) in [
        (
            "wiki/concepts",
            "agent",
            "Agent",
            "Uses [[concepts/hybrid-search|Hybrid 1]] on line 1.\nAlso uses [[concepts/hybrid-search|Hybrid 2]] and [[notes/field-observation]].\n",
        ),
        (
            "wiki/concepts",
            "router",
            "Router",
            "Connects to [[concepts/agent]].\n",
        ),
        (
            "wiki/concepts",
            "isolated",
            "Isolated",
            "Unconnected wiki note.\n",
        ),
    ] {
        let resp = client
            .post(format!("{base}/knowledge/pages"))
            .bearer_auth("knowledge-test-token")
            .json(&serde_json::json!({
                "folder_path": folder,
                "file_name": file_name,
                "title": title,
                "body": body,
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    let local_resp = client
        .get(format!(
            "{base}/knowledge/graph?scope=local&center_id=wiki%2Fconcepts%2Fagent.md&depth=1&limit=500"
        ))
        .bearer_auth("knowledge-test-token")
        .send()
        .await
        .unwrap();
    assert_eq!(local_resp.status(), StatusCode::OK);
    let local_body: Value = local_resp.json().await.unwrap();
    let nodes = local_body["nodes"].as_array().unwrap();
    let node_ids: std::collections::HashSet<&str> =
        nodes.iter().filter_map(|n| n["id"].as_str()).collect();

    // 1-hop around agent includes agent, hybrid-search, and router, but NOT isolated or raw sources.
    assert!(node_ids.contains("wiki/concepts/agent.md"));
    assert!(node_ids.contains("wiki/concepts/hybrid-search.md"));
    assert!(node_ids.contains("wiki/concepts/router.md"));
    assert!(!node_ids.contains("wiki/concepts/isolated.md"));
    assert!(!node_ids.contains("sources/notes/field-observation.md"));

    // Every edge endpoint must exist in returned `nodes` (no dangling target_id to sources/*).
    let edges = local_body["edges"].as_array().unwrap();
    for edge in edges {
        let src = edge["source_id"].as_str().unwrap();
        let dst = edge["target_id"].as_str().unwrap();
        assert!(node_ids.contains(src), "missing edge source {src}");
        assert!(node_ids.contains(dst), "missing edge target {dst}");
    }

    // Repeated links from agent -> hybrid-search are aggregated into a single edge with 2 lines.
    let agent_to_hybrid: Vec<&Value> = edges
        .iter()
        .filter(|e| {
            e["source_id"] == "wiki/concepts/agent.md"
                && e["target_id"] == "wiki/concepts/hybrid-search.md"
        })
        .collect();
    assert_eq!(agent_to_hybrid.len(), 1);
    assert_eq!(agent_to_hybrid[0]["lines"].as_array().unwrap().len(), 2);
    assert_eq!(
        agent_to_hybrid[0]["references"].as_array().unwrap().len(),
        2
    );

    handle.abort();
    let _ = std::fs::remove_dir_all(temp_root);
}

#[tokio::test]
async fn knowledge_chart_read_serves_synthesized_chart_and_rejects_invalid_paths() {
    let (base, temp_root, handle) = fixture_server().await;
    let vault_dir = temp_root.join("vault");
    crate::services::task_flow::charts::execute_chart_write(
        &vault_dir,
        serde_json::json!({
            "chart_path": "charts/ob-retest.chart.json",
            "title": "Bullish OB Retest",
            "candles": [
                {"open": 100.0, "high": 104.0, "low": 99.5, "close": 103.5, "tag": "c1"},
                {"open": 103.5, "high": 105.0, "low": 101.0, "close": 101.5, "tag": "ob"},
                {"open": 101.5, "high": 109.0, "low": 101.2, "close": 108.5, "tag": "bos"}
            ]
        }),
    )
    .expect("write chart to vault");

    let client = reqwest::Client::new();
    let ok_resp = client
        .get(format!("{base}/knowledge/charts/ob-retest.chart.json"))
        .bearer_auth("knowledge-test-token")
        .send()
        .await
        .unwrap();
    assert_eq!(ok_resp.status(), StatusCode::OK);
    let chart_json: Value = ok_resp.json().await.unwrap();
    assert_eq!(chart_json["id"], "ob-retest");
    assert_eq!(chart_json["schemaVersion"], 1);
    assert_eq!(chart_json["candles"].as_array().unwrap().len(), 3);

    let missing_resp = client
        .get(format!("{base}/knowledge/charts/missing.chart.json"))
        .bearer_auth("knowledge-test-token")
        .send()
        .await
        .unwrap();
    assert_eq!(missing_resp.status(), StatusCode::NOT_FOUND);

    let bad_ext_resp = client
        .get(format!("{base}/knowledge/charts/not-a-chart.md"))
        .bearer_auth("knowledge-test-token")
        .send()
        .await
        .unwrap();
    assert_eq!(bad_ext_resp.status(), StatusCode::BAD_REQUEST);

    handle.abort();
    let _ = std::fs::remove_dir_all(temp_root);
}
