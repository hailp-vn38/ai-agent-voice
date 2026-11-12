use super::harness::*;

#[tokio::test]
async fn catalog_merges_sources_before_filtering_paging_and_never_loads_or_exposes_secrets() {
    let (base, _db, builds, task) = server(true).await;
    let c = client(&base).await;
    let all = get(&c, &format!("{base}/providers?sort=name&page_size=4")).await;
    assert_eq!(all["total"], 7);
    assert_eq!(all["facets"]["vad"], 2);
    assert_eq!(all["total_pages"], 2);
    assert_eq!(all["facets"]["llm"], 2);
    assert_eq!(all["facets"]["tts"], 2);
    let second = get(
        &c,
        &format!("{base}/providers?sort=name&page_size=4&page=2"),
    )
    .await;
    let keys: std::collections::HashSet<_> = all["items"]
        .as_array()
        .unwrap()
        .iter()
        .chain(second["items"].as_array().unwrap())
        .map(|r| r["key"].as_str().unwrap())
        .collect();
    assert_eq!(keys.len(), 7);
    let filtered = get(
        &c,
        &format!("{base}/providers?type=llm&q=primary&enabled=true"),
    )
    .await;
    assert_eq!(filtered["total"], 2);
    assert_eq!(filtered["facets"]["tts"], 1);
    assert_eq!(
        get(&c, &format!("{base}/providers?enabled=false")).await["total"],
        0
    );
    let detail = get(&c, &format!("{base}/providers/config:llm:primary")).await;
    assert_eq!(detail["source"], "config");
    assert_eq!(detail["config_json"], "{}");
    assert_eq!(detail["runtime_status"], "not_loaded");
    assert_eq!(
        get(&c, &format!("{base}/providers/primary")).await["source"],
        "database"
    );
    assert_eq!(builds.load(Ordering::SeqCst), 0);
    for method in [reqwest::Method::PATCH, reqwest::Method::DELETE] {
        let r = c
            .request(method, format!("{base}/providers/config:llm:primary"))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::CONFLICT);
        assert!(r.text().await.unwrap().contains("config_provider_readonly"));
    }
    let before: (i64, i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM providers), (SELECT revision FROM agent_templates WHERE key='template'), (SELECT count(*) FROM template_provider_bindings)").fetch_one(_db.pool()).await.unwrap();
    for payload in [
        json!({"name":"edited"}),
        json!({"config_json":{"model":"changed"}}),
        json!({"enabled":false}),
        json!({"api_key":"CANARY"}),
    ] {
        let response = c
            .patch(format!("{base}/providers/config:llm:primary"))
            .json(&payload)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = response.text().await.unwrap();
        assert!(body.contains("config_provider_readonly"));
        assert!(!body.contains("CANARY"));
    }
    let after: (i64, i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM providers), (SELECT revision FROM agent_templates WHERE key='template'), (SELECT count(*) FROM template_provider_bindings)").fetch_one(_db.pool()).await.unwrap();
    assert_eq!(before, after);
    let r = c
        .post(format!("{base}/providers/config:tts:primary/test/llm"))
        .json(&json!({"input":"hello"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    assert!(r.text().await.unwrap().contains("provider_type_mismatch"));
    let r = c
        .get(format!("{base}/providers/config:llm:primary/capabilities"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::CONFLICT);
    let r = c
        .post(format!("{base}/providers/config:llm:primary/prepare"))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(builds.load(Ordering::SeqCst), 1);
    assert_eq!(
        get(&c, &format!("{base}/providers/config:llm:primary")).await["runtime_status"],
        "loaded"
    );
    task.abort();
}

#[tokio::test]
async fn bindings_round_trip_replace_sources_preserve_revision_audit_and_admission() {
    let (base, db, _builds, task) = server(true).await;
    let c = client(&base).await;
    for (revision, key) in [
        (1, "primary"),
        (2, "config:llm:primary"),
        (3, "primary"),
        (4, "config:llm:primary"),
    ] {
        let r = c
            .put(format!("{base}/templates/template/providers/llm"))
            .header("If-Match", format!("\"{revision}\""))
            .json(&json!({"provider_key":key}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK, "{}", r.text().await.unwrap());
        let relationship = get(&c, &format!("{base}/templates/template/providers")).await;
        assert_eq!(relationship["revision"], revision + 1);
        assert_eq!(relationship["bindings"]["llm"]["provider_key"], key);
        let row:(String,Option<i64>,Option<String>)=sqlx::query_as("SELECT provider_source,provider_id,config_provider_key FROM template_provider_bindings").fetch_one(db.pool()).await.unwrap();
        if key.starts_with("config:") {
            assert_eq!(row, ("config".into(), None, Some("primary".into())));
        } else {
            assert_eq!(row, ("database".into(), Some(1), None));
        }
    }
    let usage = get(
        &c,
        &format!("{base}/providers/config:llm:primary/templates"),
    )
    .await;
    assert_eq!(usage["total"], 1);
    for key in [
        "config:tts:primary",
        "config:llm:missing",
        "config:llm:primary:extra",
    ] {
        let r = c
            .put(format!("{base}/templates/template/providers/llm"))
            .header("If-Match", "\"5\"")
            .json(&json!({"provider_key":key}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    }
    let stale = c
        .put(format!("{base}/templates/template/providers/llm"))
        .header("If-Match", "\"1\"")
        .json(&json!({"provider_key":"primary"}))
        .send()
        .await
        .unwrap();
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    assert_eq!(
        get(&c, &format!("{base}/templates/template/providers")).await["revision"],
        5
    );
    let graph = db
        .admit_device("device", &DatabaseDevicesConfig::default())
        .await
        .unwrap();
    assert_eq!(
        graph.assignments[0].bindings[0].provider_key,
        "config:llm:primary"
    );
    assert!(graph.assignments[0].bindings[0].snapshot.is_none());
    sqlx::query("UPDATE template_provider_bindings SET config_provider_key='removed'")
        .execute(db.pool())
        .await
        .unwrap();
    let missing = get(&c, &format!("{base}/templates/template/providers")).await;
    assert_eq!(
        missing["bindings"]["llm"]["provider_key"],
        "config:llm:removed"
    );
    assert_eq!(missing["bindings"]["llm"]["enabled"], false);
    let r = c
        .delete(format!("{base}/templates/template/providers/llm"))
        .header("If-Match", "\"5\"")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(
        get(&c, &format!("{base}/templates/template/providers")).await["revision"],
        6
    );
    task.abort();
}

#[tokio::test]
async fn legacy_rejects_config_binding_before_mutating_template() {
    let (base, _db, _builds, task) = server(false).await;
    let c = client(&base).await;
    let r = c
        .put(format!("{base}/templates/template/providers/llm"))
        .header("If-Match", "\"1\"")
        .json(&json!({"provider_key":"config:llm:primary"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::CONFLICT);
    assert!(
        r.text()
            .await
            .unwrap()
            .contains("config_provider_requires_managed_runtime")
    );
    assert_eq!(
        get(&c, &format!("{base}/templates/template/providers")).await["revision"],
        1
    );
    task.abort();
}
