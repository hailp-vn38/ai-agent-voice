use super::*;

#[tokio::test]
async fn llm_gateway_pool_detail_and_model_crud_lifecycle() {
    let (hub, _fakes, db) = test_hub_with_fakes().await;

    // 1. Inspect empty pool detail for OpenAI Codex -> includes preset_models
    let detail_initial = hub
        .get_pool_detail(GatewayVendor::OpenaiCodex)
        .await
        .expect("get_pool_detail");
    assert_eq!(detail_initial.vendor, GatewayVendor::OpenaiCodex);
    assert_eq!(
        detail_initial.pool_binding,
        GatewayBinding::Pool(GatewayVendor::OpenaiCodex)
    );
    assert!(detail_initial.models.is_empty());
    assert!(!detail_initial.preset_models.is_empty());
    assert!(
        detail_initial
            .preset_models
            .iter()
            .any(|p| p.model == "gpt-5.3-codex" && p.supports_reasoning_effort)
    );
    for expected_model in [
        "gpt-6-astra",
        "gpt-6.1-sol",
        "gpt-6.1-terra",
        "gpt-6-sol",
        "gpt-6-luna",
        "gpt-5.6-sol",
        "gpt-5.6-terra",
        "gpt-5.6-luna",
        "gpt-5.5",
        "gpt-5.4",
    ] {
        assert!(
            detail_initial
                .preset_models
                .iter()
                .any(|p| p.model == expected_model && p.default_max_tokens == Some(128_000)),
            "missing or unexpected default_max_tokens for OpenAI preset {expected_model}"
        );
    }

    // 2. Add a model to the OpenAI Codex pool using preset defaults
    let created = hub
        .create_pool_model(
            GatewayVendor::OpenaiCodex,
            CreatePoolModelRequest {
                model: "gpt-5.3-codex".to_owned(),
                display_name: None,
                gateway_binding: None,
                base_url: Some("https://custom.openai.example/v1".to_owned()),
                tags: None,
                reasoning_effort: Some("high".to_owned()),
                max_tokens: Some(8192),
                context_limit_tokens: Some(200_000),
                temperature_percent: None,
                timeout_ms: Some(45_000),
                enabled: Some(true),
            },
            "req-create-pool-model",
        )
        .await
        .expect("create_pool_model");

    assert!(created.provider_key.starts_with("llm_"));
    assert_eq!(created.display_name, "GPT-5.3 Codex");
    assert_eq!(created.vendor, GatewayVendor::OpenaiCodex);
    assert_eq!(
        created.gateway_binding,
        GatewayBinding::Pool(GatewayVendor::OpenaiCodex)
    );
    assert_eq!(created.binding_mode, "pool");
    assert_eq!(created.model, "gpt-5.3-codex");
    assert_eq!(
        created.base_url.as_deref(),
        Some("https://custom.openai.example/v1")
    );
    assert_eq!(created.reasoning_effort.as_deref(), Some("high"));
    assert_eq!(created.max_tokens, Some(8192));
    assert_eq!(created.context_limit_tokens, Some(200_000));
    assert_eq!(created.timeout_ms, 45_000);
    assert_eq!(
        created.tags,
        vec![
            "chat".to_owned(),
            "reasoning".to_owned(),
            "vision".to_owned()
        ]
    );
    assert!(created.enabled);

    // 3. Verify list_vendors_detailed reflects model_count = 1
    let vendors = hub.list_vendors_detailed().await;
    let codex_info = vendors
        .iter()
        .find(|v| v.vendor == GatewayVendor::OpenaiCodex)
        .expect("codex info");
    assert_eq!(codex_info.model_count, 1);
    assert_eq!(codex_info.models[0].provider_key, created.provider_key);

    // 4. Edit model parameters in the pool
    let updated = hub
        .update_pool_model(
            GatewayVendor::OpenaiCodex,
            &created.provider_key,
            UpdatePoolModelPatch {
                model: Some("gpt-5.6-terra".to_owned()),
                display_name: Some("Codex Terra Primary".to_owned()),
                base_url: Some(None),
                tags: Some(vec!["chat".to_owned(), "reasoning".to_owned()]),
                reasoning_effort: Some(Some("medium".to_owned())),
                max_tokens: Some(Some(128_000)),
                enabled: Some(false),
                ..Default::default()
            },
            "req-update-pool-model",
        )
        .await
        .expect("update_pool_model");

    assert_eq!(updated.model, "gpt-5.6-terra");
    assert_eq!(updated.display_name, "Codex Terra Primary");
    assert_eq!(updated.base_url, None);
    assert_eq!(
        updated.tags,
        vec!["chat".to_owned(), "reasoning".to_owned()]
    );
    assert_eq!(updated.reasoning_effort.as_deref(), Some("medium"));
    assert_eq!(updated.max_tokens, Some(128_000));
    assert!(!updated.enabled);
    assert_eq!(updated.revision, 2);

    // Cross-vendor guard: cannot update/delete OpenAI Codex model via Anthropic pool endpoint
    let wrong_vendor = hub
        .delete_pool_model(
            GatewayVendor::Anthropic,
            &created.provider_key,
            "req-delete-wrong-vendor",
        )
        .await
        .expect_err("wrong vendor must fail");
    assert_eq!(wrong_vendor, GatewayError::PoolModelNotFound);

    // 5. Bind the provider to an agent_template and verify delete returns PoolModelInUse
    let provider_id: i64 = sqlx::query_scalar("SELECT id FROM providers WHERE key = ?")
        .bind(&created.provider_key)
        .fetch_one(db.pool())
        .await
        .expect("provider id");
    sqlx::query(
        "INSERT INTO agent_templates (key, name, language, prompt, enabled, revision, created_at, updated_at) \
         VALUES ('tpl_voice_1', 'Voice Template 1', 'en', 'You are helpful', 1, 1, 0, 0)",
    )
    .execute(db.pool())
    .await
    .expect("insert template");
    let tpl_id: i64 =
        sqlx::query_scalar("SELECT id FROM agent_templates WHERE key = 'tpl_voice_1'")
            .fetch_one(db.pool())
            .await
            .expect("template id");
    sqlx::query(
        "INSERT INTO template_provider_bindings (template_id, provider_type, provider_source, provider_id, created_at, updated_at) \
         VALUES (?, 'llm', 'database', ?, 0, 0)",
    )
    .bind(tpl_id)
    .bind(provider_id)
    .execute(db.pool())
    .await
    .expect("insert template binding");

    let detail_bound = hub
        .get_pool_detail(GatewayVendor::OpenaiCodex)
        .await
        .expect("detail bound");
    assert_eq!(detail_bound.models[0].template_usage_count, 1);

    let in_use_err = hub
        .delete_pool_model(
            GatewayVendor::OpenaiCodex,
            &created.provider_key,
            "req-delete-in-use",
        )
        .await
        .expect_err("delete bound model should fail");
    assert_eq!(
        in_use_err,
        GatewayError::PoolModelInUse(vec!["tpl_voice_1".to_owned()])
    );

    // Unbind and delete model
    sqlx::query("DELETE FROM template_provider_bindings WHERE template_id = ?")
        .bind(tpl_id)
        .execute(db.pool())
        .await
        .expect("delete binding");

    let deleted = hub
        .delete_pool_model(
            GatewayVendor::OpenaiCodex,
            &created.provider_key,
            "req-delete-pool-model",
        )
        .await
        .expect("delete_pool_model after unbind");
    assert!(deleted);

    let audit_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM admin_audit_events WHERE resource_type = 'provider' AND resource_id = ?",
    )
    .bind(provider_id)
    .fetch_one(db.pool())
    .await
    .expect("audit count");
    assert_eq!(audit_count, 3);

    let detail_after = hub
        .get_pool_detail(GatewayVendor::OpenaiCodex)
        .await
        .expect("detail after delete");
    assert!(detail_after.models.is_empty());
}
