use std::{collections::BTreeMap, sync::Arc};

use url::Url;

use crate::{
    config::DatabaseConfig,
    database::{Database, credentials::CredentialCipher},
};

use super::{
    FakeOAuthVendorDriver, GatewayAccountStatus, GatewayBinding, GatewayCallOutcome, GatewayError,
    GatewayVendor, LlmGatewayHub, OAuthCallbackSubmission, OAuthLoginMode, OAuthPollStatus,
    OAuthVendorDriver, StartOAuthLoginRequest, UpdateGatewayAccountPatch,
};

async fn test_hub_with_fakes() -> (
    Arc<LlmGatewayHub>,
    BTreeMap<GatewayVendor, Arc<FakeOAuthVendorDriver>>,
    Database,
) {
    let db = Database::connect(&DatabaseConfig {
        url: "sqlite::memory:".into(),
        max_connections: 1,
        ..Default::default()
    })
    .await
    .expect("in-memory sqlite connect");

    let cipher = Arc::new(CredentialCipher::new(1, &[0x42; 32]).expect("cipher"));
    let mut fake_map = BTreeMap::new();
    let mut dyn_map: BTreeMap<GatewayVendor, Arc<dyn OAuthVendorDriver>> = BTreeMap::new();
    for vendor in GatewayVendor::ALL {
        let fake = Arc::new(FakeOAuthVendorDriver::new(vendor));
        fake_map.insert(vendor, Arc::clone(&fake));
        dyn_map.insert(vendor, fake);
    }

    let hub = Arc::new(LlmGatewayHub::with_drivers(
        Some(db.pool().clone()),
        cipher,
        dyn_map,
    ));
    (hub, fake_map, db)
}

#[tokio::test]
async fn llm_gateway_pkce_login_and_secret_resolver_roundtrip() {
    let (hub, _fakes, _temp) = test_hub_with_fakes().await;

    let challenge = hub
        .start_oauth_login(StartOAuthLoginRequest {
            vendor: GatewayVendor::Anthropic,
            label: "Claude Team #1".to_owned(),
            redirect_uri: None,
        })
        .await
        .expect("start_oauth_login");

    assert_eq!(challenge.mode, OAuthLoginMode::Pkce);
    let auth_url_str = challenge.authorize_url.as_deref().expect("authorize_url");
    let auth_url = Url::parse(auth_url_str).expect("valid url");
    let state = auth_url
        .query_pairs()
        .find_map(|(k, v)| (k == "state").then(|| v.into_owned()))
        .expect("state in query");
    let challenge_s256 = auth_url
        .query_pairs()
        .find_map(|(k, v)| (k == "code_challenge").then(|| v.into_owned()))
        .expect("code_challenge in query");
    assert!(!challenge_s256.is_empty());

    // Complete via pasted full redirect URL
    let callback_url = format!(
        "https://console.anthropic.com/oauth/code/callback?code=auth_code_99&state={state}"
    );
    let view = hub
        .complete_oauth_login(
            &challenge.session_id,
            OAuthCallbackSubmission {
                callback_url_or_code: callback_url,
                state: None,
            },
        )
        .await
        .expect("complete_oauth_login");

    assert_eq!(view.vendor, GatewayVendor::Anthropic);
    assert_eq!(view.label, "Claude Team #1");
    assert_eq!(view.account_email.as_deref(), Some("anthropic@example.com"));
    assert_eq!(view.status, GatewayAccountStatus::Active);
    assert_eq!(
        view.credential_metadata
            .get("status")
            .and_then(|v| v.as_str()),
        Some("active")
    );

    // Reload snapshot from SQLite and verify 0ms lease acquisition
    hub.reload_snapshot().await.expect("reload_snapshot");
    let lease = hub
        .acquire_lease(&GatewayBinding::Account(view.account_key.clone()))
        .expect("acquire_lease");
    assert_eq!(lease.vendor, GatewayVendor::Anthropic);
    assert_eq!(lease.access_token.expose(), "tok_anthropic_auth_code_99");
    assert!(
        lease
            .extra_headers
            .contains(&("anthropic-beta".to_owned(), "oauth-2025-04-20".to_owned()))
    );
}

#[tokio::test]
async fn llm_gateway_device_code_flow_github_copilot() {
    let (hub, fakes, _temp) = test_hub_with_fakes().await;
    let copilot_fake = fakes.get(&GatewayVendor::GithubCopilot).expect("copilot");

    let challenge = hub
        .start_oauth_login(StartOAuthLoginRequest {
            vendor: GatewayVendor::GithubCopilot,
            label: "Copilot Work".to_owned(),
            redirect_uri: None,
        })
        .await
        .expect("start device flow");

    assert_eq!(challenge.mode, OAuthLoginMode::DeviceCode);
    assert_eq!(challenge.user_code.as_deref(), Some("ABCD-1234"));
    assert_eq!(
        challenge.verification_uri.as_deref(),
        Some("https://github.com/login/device")
    );

    // First poll: Pending
    let status1 = hub
        .poll_oauth_login(&challenge.session_id)
        .await
        .expect("poll 1");
    assert!(matches!(status1, OAuthPollStatus::Pending { .. }));

    // User authorizes on GitHub -> second poll: Completed
    copilot_fake.set_device_ready(true);
    let status2 = hub
        .poll_oauth_login(&challenge.session_id)
        .await
        .expect("poll 2");
    let OAuthPollStatus::Completed { account } = status2 else {
        panic!("expected Completed, got {status2:?}");
    };
    assert_eq!(account.vendor, GatewayVendor::GithubCopilot);

    let lease = hub
        .acquire_lease(&GatewayBinding::Pool(GatewayVendor::GithubCopilot))
        .expect("pool lease");
    assert_eq!(lease.access_token.expose(), "tid_copilot_bearer_token_01");
    assert!(
        lease
            .extra_headers
            .contains(&("Copilot-Integration-Id".to_owned(), "vscode-chat".to_owned()))
    );
}

#[tokio::test]
async fn llm_gateway_proactive_refresh_and_zero_retry_cooldown_rotation() {
    let (hub, fakes, _temp) = test_hub_with_fakes().await;
    let anthropic_fake = fakes.get(&GatewayVendor::Anthropic).expect("anthropic");

    // Set near-expiry timestamp so proactive refresh triggers
    anthropic_fake.set_next_expires_at(Some(100));

    let ch1 = hub
        .start_oauth_login(StartOAuthLoginRequest {
            vendor: GatewayVendor::Anthropic,
            label: "Claude #1".to_owned(),
            redirect_uri: None,
        })
        .await
        .expect("ch1");
    let acc1 = hub
        .complete_oauth_login(
            &ch1.session_id,
            OAuthCallbackSubmission {
                callback_url_or_code: "code_acc_1".to_owned(),
                state: None,
            },
        )
        .await
        .expect("acc1");

    let ch2 = hub
        .start_oauth_login(StartOAuthLoginRequest {
            vendor: GatewayVendor::Anthropic,
            label: "Claude #2".to_owned(),
            redirect_uri: None,
        })
        .await
        .expect("ch2");
    let acc2 = hub
        .complete_oauth_login(
            &ch2.session_id,
            OAuthCallbackSubmission {
                callback_url_or_code: "code_acc_2".to_owned(),
                state: None,
            },
        )
        .await
        .expect("acc2");

    // Fire 4 concurrent proactive refreshes for acc1 -> single-flight lock ensures refresh_token runs once
    let mut handles = Vec::new();
    for _ in 0..4 {
        let h = Arc::clone(&hub);
        handles.push(tokio::spawn(
            async move { h.proactive_refresh_expiring(300).await },
        ));
    }
    for handle in handles {
        let _ = handle.await.expect("join");
    }
    // Both acc1 and acc2 were expiring, so each should be refreshed exactly once (total = 2)
    assert_eq!(anthropic_fake.refresh_call_count(), 2);

    // Round-robin across the 2 active accounts in Pool(Anthropic)
    let pool_binding = GatewayBinding::Pool(GatewayVendor::Anthropic);
    let lease_a = hub.acquire_lease(&pool_binding).expect("lease a");
    let lease_b = hub.acquire_lease(&pool_binding).expect("lease b");
    assert_ne!(lease_a.account_key, lease_b.account_key);

    // Report 429 RateLimited on acc1 -> next pool acquisitions must skip acc1 and route to acc2
    hub.report_outcome(
        &acc1.account_key,
        GatewayCallOutcome::RateLimited {
            retry_after_secs: Some(60),
        },
    );
    assert_eq!(
        hub.acquire_lease(&GatewayBinding::Account(acc1.account_key.clone()))
            .unwrap_err(),
        GatewayError::AccountCoolingDown
    );
    for _ in 0..3 {
        let next_lease = hub.acquire_lease(&pool_binding).expect("pool lease");
        assert_eq!(next_lease.account_key, acc2.account_key);
    }

    // Clear cooldown on acc1 -> acc1 becomes available again
    let cleared = hub
        .clear_cooldown(&acc1.account_key)
        .await
        .expect("clear_cooldown");
    assert_eq!(cleared.cooldown_remaining_secs, 0);

    // Disable acc2 via update_account -> only acc1 remains active
    let updated = hub
        .update_account(
            &acc2.account_key,
            UpdateGatewayAccountPatch {
                label: Some("Claude #2 Paused".to_owned()),
                enabled: Some(false),
                priority: Some(200),
            },
        )
        .await
        .expect("update_account");
    assert!(!updated.enabled);
    assert_eq!(updated.status, GatewayAccountStatus::Disabled);
}

#[tokio::test]
async fn openai_codex_driver_enforces_fixed_redirect_and_9router_authorize_params() {
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use serde_json::json;

    let drivers = super::vendors::default_vendor_drivers();
    let codex_driver = drivers.get(&GatewayVendor::OpenaiCodex).expect("codex");

    // 1. Even when frontend passes a custom web app callback redirect_uri, OpenAI Codex forces localhost:1455
    assert!(codex_driver.requires_fixed_redirect_uri());
    let resolved_redirect = codex_driver.resolve_redirect_uri(Some(
        "http://localhost:5173/api/admin/llm-gateway/oauth/callback",
    ));
    assert_eq!(resolved_redirect, "http://localhost:1455/auth/callback");

    // 2. Authorize URL matches 9router Codex params & %20 scope encoding
    let auth_url = codex_driver
        .build_pkce_authorize_url("state_abc", "challenge_xyz", &resolved_redirect)
        .expect("build_pkce_authorize_url");
    assert!(
        auth_url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback"),
        "unexpected redirect_uri in {auth_url}"
    );
    assert!(
        auth_url.contains("scope=openid%20profile%20email%20offline_access"),
        "scope must use %20 encoding in {auth_url}"
    );
    assert!(auth_url.contains("id_token_add_organizations=true"));
    assert!(auth_url.contains("codex_cli_simplified_flow=true"));
    assert!(auth_url.contains("originator=codex_cli_rs"));
    assert!(auth_url.contains("code_challenge=challenge_xyz"));
    assert!(auth_url.contains("code_challenge_method=S256"));
    assert!(auth_url.contains("state=state_abc"));

    // 3. Direct JWT access token paste (eyJ...) extracts nested OpenAI profile email & chatgpt_account_id
    let header_b64 = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#);
    let payload_b64 = URL_SAFE_NO_PAD.encode(
        json!({
            "https://api.openai.com/profile": {
                "email": "codex-user@openai.example"
            },
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acct_9router_codex_01",
                "chatgpt_plan_type": "plus"
            },
            "exp": 4_102_444_800i64
        })
        .to_string()
        .as_bytes(),
    );
    let fake_jwt = format!("{header_b64}.{payload_b64}.sig");
    let exchanged = codex_driver
        .exchange_pkce_code(&fake_jwt, "state_abc", "verifier_xyz", &resolved_redirect)
        .await
        .expect("raw jwt exchange");
    assert_eq!(
        exchanged.account_email.as_deref(),
        Some("codex-user@openai.example")
    );
    assert_eq!(
        exchanged.bundle.account_context_id.as_deref(),
        Some("acct_9router_codex_01")
    );
    assert_eq!(exchanged.expires_at, Some(4_102_444_800));
    let headers = codex_driver.extra_headers_for(&exchanged.bundle);
    assert!(headers.contains(&("originator".to_owned(), "codex_cli_rs".to_owned())));
    assert!(headers.contains(&(
        "ChatGPT-Account-Id".to_owned(),
        "acct_9router_codex_01".to_owned()
    )));
}
