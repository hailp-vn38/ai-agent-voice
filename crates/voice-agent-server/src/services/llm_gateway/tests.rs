use std::{collections::BTreeMap, sync::Arc};

use url::Url;

use crate::{
    config::DatabaseConfig,
    database::{Database, credentials::CredentialCipher},
};

use super::{
    CreatePoolModelRequest, FakeOAuthVendorDriver, GatewayAccountStatus, GatewayBinding,
    GatewayCallOutcome, GatewayError, GatewayVendor, LlmGatewayHub, OAuthCallbackSubmission,
    OAuthLoginMode, OAuthPollStatus, OAuthVendorDriver, StartOAuthLoginRequest,
    UpdateGatewayAccountPatch, UpdatePoolModelPatch,
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

    let cipher: Arc<dyn crate::database::secrets::SecretResolver> =
        Arc::new(CredentialCipher::new(1, &[0x42; 32]).expect("cipher"));
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
async fn github_copilot_device_flow_slow_down_throttle_and_post_exchange_fallback() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use axum::{Router, http::StatusCode, response::IntoResponse, routing::{get, post}};
    use serde_json::json;
    use tokio::net::TcpListener;
    use super::vendors::github_copilot::GithubCopilotOAuthDriver;

    // 1. Verify hub rate-limit guard prevents hammering upstream when slow_down occurs
    let (hub, fakes, _temp) = test_hub_with_fakes().await;
    let copilot_fake = fakes.get(&GatewayVendor::GithubCopilot).expect("copilot");
    copilot_fake.set_next_device_poll_interval(Some(10));

    let ch = hub
        .start_oauth_login(StartOAuthLoginRequest {
            vendor: GatewayVendor::GithubCopilot,
            label: "Copilot Slowdown Test".to_owned(),
            redirect_uri: None,
        })
        .await
        .expect("start device flow");

    // Poll #1 hits driver and gets slow_down (10s)
    let poll1 = hub.poll_oauth_login(&ch.session_id).await.expect("poll 1");
    assert!(matches!(
        poll1,
        OAuthPollStatus::Pending {
            poll_interval_secs: 10
        }
    ));
    assert_eq!(copilot_fake.device_poll_call_count(), 1);

    // Immediate Poll #2 (< 10s later) is intercepted by hub's `next_poll_at` guard without calling driver
    let poll2 = hub.poll_oauth_login(&ch.session_id).await.expect("poll 2");
    assert!(matches!(poll2, OAuthPollStatus::Pending { .. }));
    assert_eq!(copilot_fake.device_poll_call_count(), 1);

    // 2. End-to-end HTTP test for GithubCopilotOAuthDriver:
    //    - Initial `copilot_internal/v2/token` fails (503) -> fallback saves `ghu_...` token (like 9router's postExchange)
    //    - Subsequent `refresh_account_now` succeeds and upgrades `access_token` to `tid=copilot_minted_99`
    let copilot_token_healthy = Arc::new(AtomicBool::new(false));
    let token_poll_count = Arc::new(AtomicUsize::new(0));
    let healthy_clone = Arc::clone(&copilot_token_healthy);
    let poll_count_clone = Arc::clone(&token_poll_count);

    let app = Router::new()
        .route(
            "/login/device/code",
            post(|| async move {
                axum::Json(json!({
                    "device_code": "gh_dev_code_live_01",
                    "user_code": "WXYZ-9876",
                    "verification_uri": "https://github.com/login/device",
                    "expires_in": 900,
                    "interval": 5
                }))
            }),
        )
        .route(
            "/login/oauth/access_token",
            post(move || {
                let poll_count = Arc::clone(&poll_count_clone);
                async move {
                    let n = poll_count.fetch_add(1, Ordering::SeqCst) + 1;
                    if n == 1 {
                        axum::Json(json!({
                            " access_token": null,
                            "access_token": "ghu_live_oauth_token_abc",
                            "token_type": "bearer",
                            "scope": "read:user"
                        }))
                    } else {
                        axum::Json(json!({ "error": "authorization_pending" }))
                    }
                }
            }),
        )
        .route(
            "/copilot_internal/v2/token",
            get(move || {
                let is_healthy = Arc::clone(&healthy_clone);
                async move {
                    if !is_healthy.load(Ordering::SeqCst) {
                        (StatusCode::SERVICE_UNAVAILABLE, "temporary error").into_response()
                    } else {
                        axum::Json(json!({
                            "token": "tid=copilot_minted_99;exp=4102444800",
                            "expires_at": "2100-01-01T00:00:00Z"
                        }))
                        .into_response()
                    }
                }
            }),
        )
        .route(
            "/user",
            get(|| async move {
                axum::Json(json!({
                    "email": null,
                    "login": "PhucHai38",
                    "name": "Phuc Hai"
                }))
            }),
        );

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let db = Database::connect(&DatabaseConfig {
        url: "sqlite::memory:".into(),
        max_connections: 1,
        ..Default::default()
    })
    .await
    .expect("sqlite");
    let cipher: Arc<dyn crate::database::secrets::SecretResolver> =
        Arc::new(CredentialCipher::new(1, &[0x42; 32]).expect("cipher"));
    let mut drivers: BTreeMap<GatewayVendor, Arc<dyn OAuthVendorDriver>> = BTreeMap::new();
    drivers.insert(
        GatewayVendor::GithubCopilot,
        Arc::new(GithubCopilotOAuthDriver::with_test_endpoints(&format!(
            "http://{addr}"
        ))),
    );
    let live_hub = Arc::new(LlmGatewayHub::with_drivers(
        Some(db.pool().clone()),
        cipher,
        drivers,
    ));

    let live_ch = live_hub
        .start_oauth_login(StartOAuthLoginRequest {
            vendor: GatewayVendor::GithubCopilot,
            label: "GitHub Primary".to_owned(),
            redirect_uri: None,
        })
        .await
        .expect("start live device flow");
    assert_eq!(live_ch.user_code.as_deref(), Some("WXYZ-9876"));

    // Poll completes even when `copilot_internal/v2/token` returns 503 during initial login!
    let live_poll = live_hub
        .poll_oauth_login(&live_ch.session_id)
        .await
        .expect("poll live");
    let OAuthPollStatus::Completed { account } = live_poll else {
        panic!("expected Completed with fallback token, got {live_poll:?}");
    };
    assert_eq!(account.account_email.as_deref(), Some("PhucHai38"));

    let initial_lease = live_hub
        .acquire_lease(&GatewayBinding::Account(account.account_key.clone()))
        .expect("initial lease");
    assert_eq!(initial_lease.access_token.expose(), "ghu_live_oauth_token_abc");

    // Once `copilot_internal/v2/token` is reachable, refreshing upgrades `access_token` to `tid=...`
    copilot_token_healthy.store(true, Ordering::SeqCst);
    let refreshed_view = live_hub
        .refresh_account_now(&account.account_key)
        .await
        .expect("refresh_account_now");
    assert_eq!(refreshed_view.expires_at, Some(4_102_444_800));

    let upgraded_lease = live_hub
        .acquire_lease(&GatewayBinding::Account(account.account_key))
        .expect("upgraded lease");
    assert_eq!(
        upgraded_lease.access_token.expose(),
        "tid=copilot_minted_99;exp=4102444800"
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

#[test]
fn vendor_quota_payload_parsers_match_9router_formats() {
    use serde_json::json;
    use super::vendors::{
        anthropic::parse_anthropic_usage_payload, gemini::parse_gemini_quota_payload,
        github_copilot::parse_github_copilot_usage_payload,
        openai_codex::parse_codex_usage_payload,
    };

    // 1. OpenAI Codex (`wham/usage`)
    let codex_json = json!({
        "plan_type": "plus",
        "rate_limit": {
            "limit_reached": false,
            "primary_window": {
                "used_percent": 25.0,
                "reset_at": 1760003600
            },
            "secondary_window": {
                "used_percent": 60.0,
                "reset_at": "2026-10-30T00:00:00Z"
            }
        }
    });
    let codex_quota = parse_codex_usage_payload(&codex_json);
    assert_eq!(codex_quota.plan.as_deref(), Some("plus"));
    assert!(!codex_quota.limit_reached);
    assert_eq!(codex_quota.windows.len(), 2);
    assert_eq!(codex_quota.windows[0].name, "5h");
    assert_eq!(codex_quota.windows[0].remaining_percentage, 75.0);
    assert_eq!(codex_quota.windows[0].reset_at, Some(1760003600));
    assert_eq!(codex_quota.windows[1].name, "Weekly");
    assert_eq!(codex_quota.windows[1].remaining_percentage, 40.0);

    // 2. Anthropic Claude (`/api/oauth/usage?cedar_ember=1`)
    let claude_json = json!({
        "five_hour": {
            "utilization": 42.5,
            "resets_at": "2026-10-28T18:00:00Z"
        },
        "seven_day": {
            "utilization": 100.0,
            "resets_at": "2026-11-01T00:00:00Z"
        },
        "seven_day_opus": {
            "utilization": 80.0,
            "resets_at": "2026-11-01T00:00:00Z"
        }
    });
    let claude_quota = parse_anthropic_usage_payload(&claude_json);
    assert_eq!(claude_quota.plan.as_deref(), Some("Claude Code"));
    assert!(claude_quota.limit_reached);
    assert_eq!(claude_quota.windows.len(), 3);
    assert_eq!(claude_quota.windows[0].name, "Session (5h)");
    assert_eq!(claude_quota.windows[0].remaining_percentage, 57.5);
    assert_eq!(claude_quota.windows[1].name, "Weekly (7d)");
    assert_eq!(claude_quota.windows[1].remaining_percentage, 0.0);

    // 3. Google Gemini (`retrieveUserQuota`)
    let gemini_json = json!({
        "buckets": [
            {
                "modelId": "gemini-2.5-pro",
                "remainingFraction": 0.85,
                "resetTime": "2026-10-29T00:00:00Z"
            },
            {
                "modelId": "gemini-2.5-flash",
                "remainingFraction": 0.5,
                "resetTime": "2026-10-29T00:00:00Z"
            }
        ]
    });
    let gemini_quota = parse_gemini_quota_payload(
        &gemini_json,
        Some("Standard".to_owned()),
        Some("proj-gemini-123".to_owned()),
    );
    assert_eq!(gemini_quota.plan.as_deref(), Some("Standard"));
    assert!(!gemini_quota.limit_reached);
    assert_eq!(gemini_quota.windows.len(), 2);
    assert_eq!(gemini_quota.windows[0].name, "gemini-2.5-pro");
    assert_eq!(gemini_quota.windows[0].used, 150.0);
    assert_eq!(gemini_quota.windows[0].total, 1000.0);
    assert_eq!(gemini_quota.windows[0].remaining_percentage, 85.0);
    assert_eq!(
        gemini_quota.discovered_account_context_id.as_deref(),
        Some("proj-gemini-123")
    );

    // 4. GitHub Copilot (`copilot_internal/user`)
    let copilot_json = json!({
        "copilot_plan": "individual_pro",
        "quota_reset_date": "2026-11-01",
        "quota_snapshots": {
            "chat": {
                "entitlement": 0,
                "remaining": 0,
                "unlimited": true
            },
            "premium_interactions": {
                "entitlement": 300,
                "remaining": 210,
                "unlimited": false
            }
        }
    });
    let copilot_quota = parse_github_copilot_usage_payload(&copilot_json);
    assert_eq!(copilot_quota.plan.as_deref(), Some("individual_pro"));
    assert!(!copilot_quota.limit_reached);
    assert_eq!(copilot_quota.windows.len(), 2);
    assert!(copilot_quota.windows[0].unlimited);
    assert_eq!(copilot_quota.windows[0].remaining_percentage, 100.0);
    assert_eq!(copilot_quota.windows[1].name, "Premium Requests");
    assert_eq!(copilot_quota.windows[1].used, 90.0);
    assert_eq!(copilot_quota.windows[1].total, 300.0);
    assert_eq!(copilot_quota.windows[1].remaining_percentage, 70.0);
}

#[tokio::test]
async fn hub_fetches_caches_and_routes_around_exhausted_account_quotas() {
    let (hub, drivers, _dir) = test_hub_with_fakes().await;
    let codex_fake = drivers.get(&GatewayVendor::OpenaiCodex).unwrap();

    // Create 2 OpenAI Codex accounts in the pool
    let ch1 = hub
        .start_oauth_login(StartOAuthLoginRequest {
            vendor: GatewayVendor::OpenaiCodex,
            label: "Codex A".to_owned(),
            redirect_uri: None,
        })
        .await
        .unwrap();
    let st1 = url::Url::parse(ch1.authorize_url.as_deref().unwrap())
        .unwrap()
        .query_pairs()
        .find(|(k, _)| k == "state")
        .unwrap()
        .1
        .into_owned();
    let (_, acc1) = hub.complete_oauth_by_state(&st1, "code_q1").await.unwrap();

    let ch2 = hub
        .start_oauth_login(StartOAuthLoginRequest {
            vendor: GatewayVendor::OpenaiCodex,
            label: "Codex B".to_owned(),
            redirect_uri: None,
        })
        .await
        .unwrap();
    let st2 = url::Url::parse(ch2.authorize_url.as_deref().unwrap())
        .unwrap()
        .query_pairs()
        .find(|(k, _)| k == "state")
        .unwrap()
        .1
        .into_owned();
    let (_, acc2) = hub.complete_oauth_by_state(&st2, "code_q2").await.unwrap();

    // Mark acc1 as quota limit_reached = true, acc2 as limit_reached = false
    codex_fake.set_next_quota(Some(super::types::VendorQuotaSnapshot {
        plan: Some("plus".to_owned()),
        limit_reached: true,
        windows: Vec::new(),
        message: Some("5h limit reached".to_owned()),
        discovered_account_context_id: None,
    }));
    let q1 = hub
        .get_account_quota(&acc1.account_key, true)
        .await
        .unwrap();
    assert!(q1.limit_reached);
    assert_eq!(codex_fake.quota_call_count(), 1);

    // Cached call (`force = false`) does not hit upstream driver again
    let _ = hub
        .get_account_quota(&acc1.account_key, false)
        .await
        .unwrap();
    assert_eq!(codex_fake.quota_call_count(), 1);

    codex_fake.set_next_quota(None);
    let q2 = hub
        .get_account_quota(&acc2.account_key, true)
        .await
        .unwrap();
    assert!(!q2.limit_reached);
    assert_eq!(q2.windows.len(), 2);

    // Pool routing (`pool:openai_codex`) automatically skips `acc1` (limit_reached) and picks `acc2`
    let lease_a = hub
        .acquire_lease(&GatewayBinding::Pool(GatewayVendor::OpenaiCodex))
        .unwrap();
    let lease_b = hub
        .acquire_lease(&GatewayBinding::Pool(GatewayVendor::OpenaiCodex))
        .unwrap();
    assert_eq!(lease_a.account_key, acc2.account_key);
    assert_eq!(lease_b.account_key, acc2.account_key);
}

#[tokio::test]
async fn llm_gateway_muse_cookie_paste_interactive_cdp_capture_and_session_renewal() {
    use std::sync::Mutex;
    use std::time::Duration;
    use axum::{Router, http::HeaderMap, response::IntoResponse, routing::{get, post}};
    use serde_json::{Value, json};
    use tokio::net::TcpListener;

    use crate::providers::llm::gateway::muse::{
        cdp::{BrowserEngine, BrowserPage, CdpError, CookieParam},
        config::parse_accounts,
    };
    use super::vendors::muse::MuseOAuthDriver;

    struct FakeMuseLoginPage {
        cookies: Arc<Mutex<BTreeMap<String, String>>>,
    }

    #[async_trait::async_trait]
    impl BrowserPage for FakeMuseLoginPage {
        async fn navigate(&self, _url: &str) -> Result<(), CdpError> {
            Ok(())
        }
        async fn current_url(&self) -> Result<String, CdpError> {
            Ok("https://muse.ai/thread/new".to_owned())
        }
        async fn evaluate_json(
            &self,
            _expression: &str,
            _await_promise: bool,
            _timeout: Duration,
        ) -> Result<Value, CdpError> {
            Ok(Value::Null)
        }
        async fn press_enter(&self) -> Result<(), CdpError> {
            Ok(())
        }
        async fn get_cookies(&self, _urls: &[&str]) -> Result<BTreeMap<String, String>, CdpError> {
            Ok(self.cookies.lock().unwrap().clone())
        }
        async fn close(&self) {}
    }

    struct FakeMuseLoginEngine {
        cookies: Arc<Mutex<BTreeMap<String, String>>>,
    }

    #[async_trait::async_trait]
    impl BrowserEngine for FakeMuseLoginEngine {
        async fn open_page(
            &self,
            _context_key: &str,
            _cookies: &[CookieParam],
        ) -> Result<Arc<dyn BrowserPage>, CdpError> {
            Ok(Arc::new(FakeMuseLoginPage {
                cookies: Arc::clone(&self.cookies),
            }))
        }
        async fn dispose_context(&self, _context_key: &str) {}
    }

    // 1. Test interactive CDP login capture via `poll_oauth_login`
    let db = Database::connect(&DatabaseConfig {
        url: "sqlite::memory:".into(),
        max_connections: 1,
        ..Default::default()
    })
    .await
    .expect("sqlite memory");
    let cipher: Arc<dyn crate::database::secrets::SecretResolver> =
        Arc::new(CredentialCipher::new(1, &[0x42; 32]).expect("cipher"));
    let shared_cookies: Arc<Mutex<BTreeMap<String, String>>> = Arc::new(Mutex::new(BTreeMap::new()));
    let fake_login_engine: Arc<dyn BrowserEngine> = Arc::new(FakeMuseLoginEngine {
        cookies: Arc::clone(&shared_cookies),
    });

    let mut drivers: BTreeMap<GatewayVendor, Arc<dyn OAuthVendorDriver>> = BTreeMap::new();
    for vendor in GatewayVendor::ALL {
        if vendor == GatewayVendor::Muse {
            drivers.insert(
                vendor,
                Arc::new(MuseOAuthDriver::with_browser_engine(Arc::clone(
                    &fake_login_engine,
                ))),
            );
        } else {
            drivers.insert(vendor, Arc::new(FakeOAuthVendorDriver::new(vendor)));
        }
    }
    let hub = Arc::new(LlmGatewayHub::with_drivers(
        Some(db.pool().clone()),
        Arc::clone(&cipher),
        drivers,
    ));

    let ch_cdp = hub
        .start_oauth_login(StartOAuthLoginRequest {
            vendor: GatewayVendor::Muse,
            label: "Muse Interactive CDP".to_owned(),
            redirect_uri: None,
        })
        .await
        .expect("start muse login");
    assert_eq!(ch_cdp.mode, OAuthLoginMode::Pkce);
    assert!(
        ch_cdp
            .authorize_url
            .as_deref()
            .unwrap_or("")
            .starts_with("https://muse.ai/thread/new")
    );

    // Poll #1 before user logs into the Chromium window -> Pending
    let poll1 = hub.poll_oauth_login(&ch_cdp.session_id).await.expect("poll 1");
    assert!(matches!(poll1, OAuthPollStatus::Pending { .. }));

    // Simulate user completing login in the Chromium window -> cookies appear in Network.getCookies
    {
        let mut guard = shared_cookies.lock().unwrap();
        guard.insert("hatch_sess".to_owned(), "cdp_sess_1".to_owned());
        guard.insert("hatch_gw".to_owned(), "cdp_gw_1".to_owned());
        guard.insert("hatch_vml".to_owned(), "cdp_vml_1".to_owned());
        guard.insert("hatch_native_auth_device".to_owned(), "cdp_dev_1".to_owned());
    }

    // Poll #2 automatically captures the 4 cookies, seals them into SQLite, and returns Completed!
    let poll2 = hub.poll_oauth_login(&ch_cdp.session_id).await.expect("poll 2");
    let OAuthPollStatus::Completed { account: cdp_acc } = poll2 else {
        panic!("expected Completed from interactive CDP poll, got {poll2:?}");
    };
    assert_eq!(cdp_acc.vendor, GatewayVendor::Muse);
    assert_eq!(cdp_acc.label, "Muse Interactive CDP");

    // 2. Test manual Cookie header paste (with spaces & `Cookie:` prefix) + online session renewal
    let app = Router::new()
        .route(
            "/api/session",
            get(|headers: HeaderMap| async move {
                let cookie = headers
                    .get("cookie")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("");
                assert!(cookie.contains("hatch_sess="));
                (
                    [(
                        axum::http::header::SET_COOKIE,
                        "hatch_sess=sess_rotated_99; Path=/; HttpOnly; Secure",
                    )],
                    axum::Json(json!({
                        "vm_id": "vm_muse_777",
                        "user": { "email": "muse-creator@muse.ai" }
                    })),
                )
                    .into_response()
            }),
        )
        .route(
            "/api/hatch/vm/wake",
            post(|| async move { axum::Json(json!({"ok": true})) }),
        );
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let mut online_drivers: BTreeMap<GatewayVendor, Arc<dyn OAuthVendorDriver>> = BTreeMap::new();
    online_drivers.insert(
        GatewayVendor::Muse,
        Arc::new(MuseOAuthDriver::with_test_endpoints(
            format!("http://{addr}/api/session"),
            format!("http://{addr}/api/hatch/vm/wake"),
        )),
    );
    let online_hub = Arc::new(LlmGatewayHub::with_drivers(
        Some(db.pool().clone()),
        cipher,
        online_drivers,
    ));

    let ch_paste = online_hub
        .start_oauth_login(StartOAuthLoginRequest {
            vendor: GatewayVendor::Muse,
            label: "Muse Manual Paste".to_owned(),
            redirect_uri: None,
        })
        .await
        .expect("start paste login");

    let pasted_acc = online_hub
        .complete_oauth_login(
            &ch_paste.session_id,
            OAuthCallbackSubmission {
                callback_url_or_code: "Cookie: hatch_sess=sess_initial; hatch_gw=gw_initial; hatch_vml=vml_initial; hatch_native_auth_device=dev_initial".to_owned(),
                state: None,
            },
        )
        .await
        .expect("complete_oauth_login with raw Cookie header containing spaces");

    assert_eq!(pasted_acc.vendor, GatewayVendor::Muse);
    assert_eq!(
        pasted_acc.account_email.as_deref(),
        Some("muse-creator@muse.ai")
    );

    // Reload from SQLite and verify rotated `hatch_sess=sess_rotated_99` and `vm_id=vm_muse_777` are sealed & restored
    online_hub.reload_snapshot().await.expect("reload_snapshot");
    let lease = online_hub
        .acquire_lease(&GatewayBinding::Account(pasted_acc.account_key.clone()))
        .expect("acquire muse lease");
    let parsed_creds = parse_accounts(lease.access_token.expose());
    assert_eq!(parsed_creds.len(), 1);
    assert_eq!(
        parsed_creds[0].cookies.get("hatch_sess").map(String::as_str),
        Some("sess_rotated_99")
    );
    assert_eq!(parsed_creds[0].vm_id.as_deref(), Some("vm_muse_777"));

    // 3. Test pasting full browser export JSON `{"url":"https://muse.ai","cookies":[...]}`
    //    with realistic multi-kilobyte `hatch_gw` / `hatch_vml` values (> 5 KiB raw JSON)
    let ch_json_export = online_hub
        .start_oauth_login(StartOAuthLoginRequest {
            vendor: GatewayVendor::Muse,
            label: "Muse Browser Export JSON".to_owned(),
            redirect_uri: None,
        })
        .await
        .expect("start json export login");

    let large_gw = format!("v1%3A{}", "y0hVepRAOkKYuBVg".repeat(140));
    let large_vml = format!("v1%3A{}", "bJ1uZtpKsBWOgtjA".repeat(145));
    let browser_export_payload = serde_json::json!({
        "url": "https://muse.ai",
        "cookies": [
            {"domain":".muse.ai","expirationDate":1825998203.478414,"hostOnly":false,"httpOnly":true,"name":"datr","path":"/","sameSite":"lax","secure":true,"session":false,"storeId":"0","value":"ey3HajK5d-WCgOv2LIOezgTh"},
            {"domain":"muse.ai","expirationDate":1794063196.58286,"hostOnly":true,"httpOnly":true,"name":"hatch_native_auth_device","path":"/","sameSite":"strict","secure":true,"session":false,"storeId":"0","value":"v1%3Adev_export"},
            {"domain":".muse.ai","expirationDate":1823007257.749747,"hostOnly":false,"httpOnly":false,"name":"layout_state","path":"/","sameSite":"unspecified","secure":true,"session":false,"storeId":"0","value":"%7B%22version%22%3A2%7D"},
            {"domain":".muse.ai","expirationDate":1823930037.417935,"hostOnly":false,"httpOnly":false,"name":"theme","path":"/","sameSite":"unspecified","secure":true,"session":false,"storeId":"0","value":"%7B%22chatThemeCache%22%3A%7B%7D%7D"},
            {"domain":".muse.ai","expirationDate":1793726343.711382,"hostOnly":false,"httpOnly":false,"name":"dpr","path":"/","sameSite":"unspecified","secure":true,"session":false,"storeId":"0","value":"2"},
            {"domain":".muse.ai","expirationDate":1793790884.855339,"hostOnly":false,"httpOnly":false,"name":"wd","path":"/","sameSite":"unspecified","secure":true,"session":false,"storeId":"0","value":"640x719"},
            {"domain":".muse.ai","expirationDate":1795778194.858809,"hostOnly":false,"httpOnly":true,"name":"hatch_sess","path":"/","sameSite":"lax","secure":true,"session":false,"storeId":"0","value":"sess_export_initial"},
            {"domain":".muse.ai","expirationDate":1824722194.859351,"hostOnly":false,"httpOnly":true,"name":"hatch_gw","path":"/","sameSite":"lax","secure":true,"session":false,"storeId":"0","value": large_gw},
            {"domain":".muse.ai","expirationDate":1793359056.334252,"hostOnly":false,"httpOnly":true,"name":"hatch_vml","path":"/","sameSite":"lax","secure":true,"session":false,"storeId":"0","value": large_vml}
        ]
    })
    .to_string();

    let json_acc = online_hub
        .complete_oauth_login(
            &ch_json_export.session_id,
            OAuthCallbackSubmission {
                callback_url_or_code: browser_export_payload,
                state: None,
            },
        )
        .await
        .expect("complete_oauth_login with browser export JSON");

    assert_eq!(json_acc.vendor, GatewayVendor::Muse);
    assert_eq!(json_acc.expires_at, Some(1_793_359_056));
    let json_lease = online_hub
        .acquire_lease(&GatewayBinding::Account(json_acc.account_key.clone()))
        .expect("acquire json export lease");
    let json_creds = parse_accounts(json_lease.access_token.expose());
    assert_eq!(json_creds.len(), 1);
    assert!(!json_creds[0].cookies.contains_key("layout_state"));
    assert!(!json_creds[0].cookies.contains_key("theme"));
    assert_eq!(
        json_creds[0].cookies.get("datr").map(String::as_str),
        Some("ey3HajK5d-WCgOv2LIOezgTh")
    );
    assert_eq!(
        json_creds[0].cookies.get("hatch_gw").map(String::as_str),
        Some(large_gw.as_str())
    );
    assert_eq!(
        json_creds[0].cookies.get("hatch_vml").map(String::as_str),
        Some(large_vml.as_str())
    );
}

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
                max_tokens: Some(Some(16384)),
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
    assert_eq!(updated.tags, vec!["chat".to_owned(), "reasoning".to_owned()]);
    assert_eq!(updated.reasoning_effort.as_deref(), Some("medium"));
    assert_eq!(updated.max_tokens, Some(16384));
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
    let tpl_id: i64 = sqlx::query_scalar("SELECT id FROM agent_templates WHERE key = 'tpl_voice_1'")
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
