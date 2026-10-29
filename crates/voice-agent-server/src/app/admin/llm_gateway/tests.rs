use crate::{
    app::{AppState, new_lifecycle, router_with_state},
    config::AppConfig,
    database::{Database, credentials::CredentialCipher, secrets::SecretResolver},
    providers::ProviderSet,
    services::llm_gateway::{
        FakeOAuthVendorDriver, GatewayVendor, LlmGatewayHub, OAuthVendorDriver,
    },
};
use reqwest::StatusCode;
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};

async fn spawn_gateway_admin_server() -> (
    String,
    Arc<LlmGatewayHub>,
    std::path::PathBuf,
    tokio::task::JoinHandle<()>,
) {
    let temp_dir =
        std::env::temp_dir().join(format!("llm-gateway-admin-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let db_path = temp_dir.join("gateway.db");
    let config_str = format!(
        r#"
[server]
bind = "127.0.0.1:0"
public_ws_url = "ws://127.0.0.1:0/voice/v1/"
[database]
url = "sqlite://{}?mode=rwc"
[provider_defaults]
vad = "test"
asr = "test"
llm = "test"
tts = "test"
[api]
enabled = true
admin_token = "gateway-admin-token"
"#,
        db_path.display()
    );
    let config: AppConfig = toml::from_str(&config_str).unwrap();
    let database = Database::connect(&config.database).await.unwrap();
    let pool = database.pool().clone();
    let resolver: Arc<dyn SecretResolver> = Arc::new(CredentialCipher::new(1, &[9u8; 32]).unwrap());
    let lifecycle = new_lifecycle(&config);
    let mut state = AppState::from_provider_set_with_database_resolver_and_shutdown(
        config,
        Arc::new(ProviderSet::unavailable()),
        Some(database),
        Arc::clone(&resolver),
        lifecycle,
    );
    let mut drivers: BTreeMap<GatewayVendor, Arc<dyn OAuthVendorDriver>> = BTreeMap::new();
    for vendor in GatewayVendor::ALL {
        drivers.insert(vendor, Arc::new(FakeOAuthVendorDriver::new(vendor)));
    }
    let hub = Arc::new(LlmGatewayHub::with_drivers(Some(pool), resolver, drivers));
    state.llm_gateway_hub = Arc::clone(&hub);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = router_with_state(state);
    let handle = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (format!("http://{addr}/api/admin"), hub, temp_dir, handle)
}

#[tokio::test]
async fn admin_llm_gateway_routes_and_provider_binding_lifecycle() {
    let (base, _hub, temp_dir, handle) = spawn_gateway_admin_server().await;
    let client = reqwest::Client::new();

    // 1. GET /llm-gateway/vendors returns all 5 supported vendors
    let vendors_resp = client
        .get(format!("{base}/llm-gateway/vendors"))
        .bearer_auth("gateway-admin-token")
        .send()
        .await
        .unwrap();
    assert_eq!(vendors_resp.status(), StatusCode::OK);
    let vendors_body: Value = vendors_resp.json().await.unwrap();
    let vendors = vendors_body["vendors"].as_array().unwrap();
    assert_eq!(vendors.len(), 5);

    // 2. Start OAuth login via POST /llm-gateway/oauth/start and complete via GET /llm-gateway/oauth/callback (browser redirect without Bearer header)
    let start_resp = client
        .post(format!("{base}/llm-gateway/oauth/start"))
        .bearer_auth("gateway-admin-token")
        .json(&json!({
            "vendor": "openai_codex",
            "label": "OpenAI Team #1"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(start_resp.status(), StatusCode::OK);
    let challenge: Value = start_resp.json().await.unwrap();
    let session_id = challenge["session_id"].as_str().unwrap().to_owned();
    let authorize_url = challenge["authorize_url"].as_str().unwrap();
    let parsed_url = url::Url::parse(authorize_url).unwrap();
    let state_param = parsed_url
        .query_pairs()
        .find(|(k, _)| k == "state")
        .map(|(_, v)| v.to_string())
        .unwrap();

    let cb_ok = client
        .get(format!(
            "{base}/llm-gateway/oauth/callback?code=auth-code-123&state={state_param}"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(cb_ok.status(), StatusCode::OK);
    assert!(
        cb_ok
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("text/html")
    );

    let poll_resp = client
        .post(format!(
            "{base}/llm-gateway/oauth/sessions/{session_id}/poll"
        ))
        .bearer_auth("gateway-admin-token")
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(poll_resp.status(), StatusCode::OK);
    let poll_body: Value = poll_resp.json().await.unwrap();
    assert_eq!(poll_body["status"], "completed");
    let account_key = poll_body["account"]["account_key"]
        .as_str()
        .unwrap()
        .to_owned();

    let list_resp = client
        .get(format!("{base}/llm-gateway/accounts?vendor=openai_codex"))
        .bearer_auth("gateway-admin-token")
        .send()
        .await
        .unwrap();
    assert_eq!(list_resp.status(), StatusCode::OK);
    let list_body: Value = list_resp.json().await.unwrap();
    let accounts = list_body["accounts"].as_array().unwrap();
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0]["account_key"], account_key);
    assert_eq!(accounts[0]["label"], "OpenAI Team #1");

    // Invalid vendor query returns 400 invalid_vendor
    let bad_vendor = client
        .get(format!("{base}/llm-gateway/accounts?vendor=unknown_vendor"))
        .bearer_auth("gateway-admin-token")
        .send()
        .await
        .unwrap();
    assert_eq!(bad_vendor.status(), StatusCode::BAD_REQUEST);

    // 3. PATCH /llm-gateway/accounts/{account_key}, refresh, and clear-cooldown
    let patch_resp = client
        .patch(format!("{base}/llm-gateway/accounts/{account_key}"))
        .bearer_auth("gateway-admin-token")
        .json(&json!({
            "label": "OpenAI Team Primary",
            "priority": 25,
            "enabled": true
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(patch_resp.status(), StatusCode::OK);
    let patched_acc: Value = patch_resp.json().await.unwrap();
    assert_eq!(patched_acc["account"]["label"], "OpenAI Team Primary");
    assert_eq!(patched_acc["account"]["priority"], 25);

    let refresh_resp = client
        .post(format!("{base}/llm-gateway/accounts/{account_key}/refresh"))
        .bearer_auth("gateway-admin-token")
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(refresh_resp.status(), StatusCode::OK);

    let clear_resp = client
        .post(format!(
            "{base}/llm-gateway/accounts/{account_key}/clear-cooldown"
        ))
        .bearer_auth("gateway-admin-token")
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(clear_resp.status(), StatusCode::OK);

    // 3c. GET & POST /llm-gateway/accounts/{account_key}/quota and GET /llm-gateway/quotas
    let quota_resp = client
        .get(format!("{base}/llm-gateway/accounts/{account_key}/quota"))
        .bearer_auth("gateway-admin-token")
        .send()
        .await
        .unwrap();
    assert_eq!(quota_resp.status(), StatusCode::OK);
    let quota_body: Value = quota_resp.json().await.unwrap();
    assert_eq!(quota_body["quota"]["account_key"], account_key);
    assert_eq!(quota_body["quota"]["limit_reached"], false);
    assert_eq!(quota_body["quota"]["windows"].as_array().unwrap().len(), 2);

    let list_quotas_resp = client
        .get(format!("{base}/llm-gateway/quotas?vendor=openai_codex"))
        .bearer_auth("gateway-admin-token")
        .send()
        .await
        .unwrap();
    assert_eq!(list_quotas_resp.status(), StatusCode::OK);
    let list_quotas_body: Value = list_quotas_resp.json().await.unwrap();
    assert_eq!(list_quotas_body["quotas"].as_array().unwrap().len(), 1);

    // 4. Create an `openai` provider with Static API Key, then switch to `gateway_binding`
    let create_prov = client
        .post(format!("{base}/providers"))
        .bearer_auth("gateway-admin-token")
        .json(&json!({
            "name": "OpenAI Main",
            "type": "llm",
            "adapter": "openai",
            "config_json": {
                "base_url": "https://api.openai.com/v1",
                "model": "gpt-4.1-mini"
            },
            "api_key": "sk-static-key-9999"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_prov.status(), StatusCode::CREATED);
    let created_prov: Value = create_prov.json().await.unwrap();
    let prov_key = created_prov["key"].as_str().unwrap().to_owned();
    assert!(created_prov["credential"].is_object());
    assert!(created_prov["credential_env"].is_string());

    // Switch provider to OAuth Gateway Account (1-1) -> clears credential & credential_env
    let patch_prov = client
        .patch(format!("{base}/providers/{prov_key}"))
        .bearer_auth("gateway-admin-token")
        .header("if-match", "\"1\"")
        .json(&json!({
            "config_json": {
                "base_url": "https://api.openai.com/v1",
                "model": "gpt-4.1-mini",
                "gateway_binding": format!("account:{account_key}")
            },
            "api_key": null
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(patch_prov.status(), StatusCode::OK);
    let updated_prov: Value = patch_prov.json().await.unwrap();
    assert!(updated_prov["credential"].is_null());
    assert!(updated_prov["credential_env"].is_null());

    // 5. DELETE /llm-gateway/accounts/{account_key} while bound returns 409 Conflict
    let del_conflict = client
        .delete(format!("{base}/llm-gateway/accounts/{account_key}"))
        .bearer_auth("gateway-admin-token")
        .send()
        .await
        .unwrap();
    assert_eq!(del_conflict.status(), StatusCode::CONFLICT);
    let conflict_body: Value = del_conflict.json().await.unwrap();
    assert_eq!(conflict_body["error"]["code"], "gateway_account_in_use");

    // Switch provider to pool binding `pool:openai_codex`, then delete account succeeds
    let patch_pool = client
        .patch(format!("{base}/providers/{prov_key}"))
        .bearer_auth("gateway-admin-token")
        .header("if-match", "\"2\"")
        .json(&json!({
            "config_json": {
                "base_url": "https://api.openai.com/v1",
                "model": "gpt-4.1-mini",
                "gateway_binding": "pool:openai_codex"
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(patch_pool.status(), StatusCode::OK);

    let del_ok = client
        .delete(format!("{base}/llm-gateway/accounts/{account_key}"))
        .bearer_auth("gateway-admin-token")
        .send()
        .await
        .unwrap();
    assert_eq!(del_ok.status(), StatusCode::OK);
    let del_body: Value = del_ok.json().await.unwrap();
    assert_eq!(del_body["deleted"], true);

    // 6. Pool Detail & Pool Model CRUD via HTTP routes
    // (`GET /llm-gateway/pools/openai_codex` already contains the `prov_key` provider from step 5)
    let pool_resp = client
        .get(format!("{base}/llm-gateway/pools/openai_codex"))
        .bearer_auth("gateway-admin-token")
        .send()
        .await
        .unwrap();
    assert_eq!(pool_resp.status(), StatusCode::OK);
    let pool_body: Value = pool_resp.json().await.unwrap();
    assert_eq!(pool_body["pool"]["vendor"], "openai_codex");
    assert_eq!(pool_body["pool"]["models"].as_array().unwrap().len(), 1);
    assert!(
        !pool_body["pool"]["preset_models"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    // Add a new model directly into the Anthropic pool
    let create_model_resp = client
        .post(format!("{base}/llm-gateway/pools/anthropic/models"))
        .bearer_auth("gateway-admin-token")
        .json(&json!({
            "model": "claude-sonnet-4-20250514",
            "display_name": "Claude Sonnet 4 Pool",
            "tags": ["chat", "reasoning", "vision"],
            "max_tokens": 8192,
            "context_limit_tokens": 200000,
            "timeout_ms": 60000,
            "enabled": true
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_model_resp.status(), StatusCode::CREATED);
    let created_model_body: Value = create_model_resp.json().await.unwrap();
    let anthropic_model_key = created_model_body["model"]["provider_key"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        created_model_body["model"]["gateway_binding"],
        "pool:anthropic"
    );
    assert_eq!(
        created_model_body["model"]["model"],
        "claude-sonnet-4-20250514"
    );
    assert_eq!(created_model_body["model"]["max_tokens"], 8192);

    // Edit the model in the Anthropic pool
    let patch_model_resp = client
        .patch(format!(
            "{base}/llm-gateway/pools/anthropic/models/{anthropic_model_key}"
        ))
        .bearer_auth("gateway-admin-token")
        .json(&json!({
            "model": "claude-opus-4-20250514",
            "display_name": "Claude Opus 4 Pool",
            "max_tokens": null,
            "enabled": false
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(patch_model_resp.status(), StatusCode::OK);
    let patched_model_body: Value = patch_model_resp.json().await.unwrap();
    assert_eq!(
        patched_model_body["model"]["model"],
        "claude-opus-4-20250514"
    );
    assert_eq!(
        patched_model_body["model"]["display_name"],
        "Claude Opus 4 Pool"
    );
    assert!(patched_model_body["model"]["max_tokens"].is_null());
    assert_eq!(patched_model_body["model"]["enabled"], false);

    // Re-enable the Anthropic pool model so it can be bound to a template
    let reenable_model_resp = client
        .patch(format!(
            "{base}/llm-gateway/pools/anthropic/models/{anthropic_model_key}"
        ))
        .bearer_auth("gateway-admin-token")
        .json(&json!({
            "enabled": true
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(reenable_model_resp.status(), StatusCode::OK);

    // Bind a template to the Anthropic pool model and verify DELETE returns 409 Conflict (gateway_pool_model_in_use)
    let create_tpl_resp = client
        .post(format!("{base}/templates"))
        .bearer_auth("gateway-admin-token")
        .json(&json!({
            "key": "tpl_gateway_test",
            "name": "Gateway Test Template",
            "language": "en",
            "prompt": "You are a helpful assistant."
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_tpl_resp.status(), StatusCode::CREATED);

    let bind_tpl_resp = client
        .put(format!("{base}/templates/tpl_gateway_test/providers/llm"))
        .bearer_auth("gateway-admin-token")
        .header("if-match", "\"1\"")
        .json(&json!({
            "provider_key": anthropic_model_key
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(bind_tpl_resp.status(), StatusCode::OK);

    let delete_in_use_resp = client
        .delete(format!(
            "{base}/llm-gateway/pools/anthropic/models/{anthropic_model_key}"
        ))
        .bearer_auth("gateway-admin-token")
        .send()
        .await
        .unwrap();
    assert_eq!(delete_in_use_resp.status(), StatusCode::CONFLICT);
    let delete_in_use_body: Value = delete_in_use_resp.json().await.unwrap();
    assert_eq!(
        delete_in_use_body["error"]["code"],
        "gateway_pool_model_in_use"
    );
    assert_eq!(
        delete_in_use_body["error"]["templates"],
        json!(["tpl_gateway_test"])
    );

    let unbind_tpl_resp = client
        .delete(format!("{base}/templates/tpl_gateway_test/providers/llm"))
        .bearer_auth("gateway-admin-token")
        .header("if-match", "\"2\"")
        .send()
        .await
        .unwrap();
    assert_eq!(unbind_tpl_resp.status(), StatusCode::OK);

    // Delete the model from the Anthropic pool after unbinding
    let delete_model_resp = client
        .delete(format!(
            "{base}/llm-gateway/pools/anthropic/models/{anthropic_model_key}"
        ))
        .bearer_auth("gateway-admin-token")
        .send()
        .await
        .unwrap();
    assert_eq!(delete_model_resp.status(), StatusCode::OK);
    let delete_model_body: Value = delete_model_resp.json().await.unwrap();
    assert_eq!(delete_model_body["deleted"], true);

    handle.abort();
    let _ = std::fs::remove_dir_all(&temp_dir);
}
