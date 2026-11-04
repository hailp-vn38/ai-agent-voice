use super::*;

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
    assert!(lease.extra_headers.contains(&(
        "Copilot-Integration-Id".to_owned(),
        "vscode-chat".to_owned()
    )));
}

#[test]
fn github_copilot_inference_headers_omit_github_rest_api_version() {
    use super::super::{
        types::OAuthTokenBundle, vendors::github_copilot::GithubCopilotOAuthDriver,
    };
    let bundle = OAuthTokenBundle {
        access_token: "tid=copilot".to_owned(),
        refresh_token: None,
        upstream_oauth_token: Some("ghu_test".to_owned()),
        account_context_id: None,
    };
    let headers = GithubCopilotOAuthDriver::new().extra_headers_for(&bundle);
    // api.githubcopilot.com rejects the GitHub REST version with `400 invalid apiVersion`.
    assert!(
        headers
            .iter()
            .all(|(name, _)| !name.eq_ignore_ascii_case("x-github-api-version"))
    );
    assert!(headers.contains(&(
        "Copilot-Integration-Id".to_owned(),
        "vscode-chat".to_owned()
    )));
}

#[tokio::test]
async fn github_copilot_device_flow_slow_down_throttle_and_post_exchange_fallback() {
    use crate::services::llm_gateway::vendors::github_copilot::GithubCopilotOAuthDriver;
    use axum::{
        Router,
        http::StatusCode,
        response::IntoResponse,
        routing::{get, post},
    };
    use serde_json::json;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use tokio::net::TcpListener;

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
    assert_eq!(
        initial_lease.access_token.expose(),
        "ghu_live_oauth_token_abc"
    );

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
async fn openai_codex_driver_enforces_fixed_redirect_and_9router_authorize_params() {
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use serde_json::json;

    let drivers = crate::services::llm_gateway::vendors::default_vendor_drivers();
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

#[tokio::test]
async fn llm_gateway_muse_cookie_paste_rejects_login_flow_and_renews_session() {
    use axum::{
        Router,
        http::HeaderMap,
        response::IntoResponse,
        routing::{get, post},
    };
    use serde_json::json;
    use tokio::net::TcpListener;

    use crate::providers::llm::gateway::muse::config::parse_accounts;
    use crate::services::llm_gateway::vendors::muse::MuseOAuthDriver;

    let db = Database::connect(&DatabaseConfig {
        url: "sqlite::memory:".into(),
        max_connections: 1,
        ..Default::default()
    })
    .await
    .expect("sqlite memory");
    let cipher: Arc<dyn crate::database::secrets::SecretResolver> =
        Arc::new(CredentialCipher::new(1, &[0x42; 32]).expect("cipher"));

    // 1. Muse has no browser login: the OAuth session flow refuses it outright.
    assert_eq!(
        GatewayVendor::Muse.login_mode(),
        OAuthLoginMode::CookiePaste
    );
    let mut drivers: BTreeMap<GatewayVendor, Arc<dyn OAuthVendorDriver>> = BTreeMap::new();
    drivers.insert(GatewayVendor::Muse, Arc::new(MuseOAuthDriver::new()));
    let offline_hub =
        LlmGatewayHub::with_drivers(Some(db.pool().clone()), Arc::clone(&cipher), drivers);
    assert!(matches!(
        offline_hub
            .start_oauth_login(StartOAuthLoginRequest {
                vendor: GatewayVendor::Muse,
                label: "Muse Browser Login".to_owned(),
                redirect_uri: None,
            })
            .await,
        Err(GatewayError::ValidationFailed)
    ));
    assert!(matches!(
        offline_hub
            .import_cookie_account(ImportCookieAccountRequest {
                vendor: GatewayVendor::Muse,
                label: "Muse Missing Session".to_owned(),
                cookies: "theme=dark; dpr=2".to_owned(),
            })
            .await,
        Err(GatewayError::OAuthExchangeFailed(_))
    ));

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
        Arc::clone(&cipher),
        online_drivers,
    ));

    let pasted_acc = online_hub
        .import_cookie_account(ImportCookieAccountRequest {
            vendor: GatewayVendor::Muse,
            label: "Muse Manual Paste".to_owned(),
            cookies: "Cookie: hatch_sess=sess_initial; hatch_gw=gw_initial; hatch_vml=vml_initial; hatch_native_auth_device=dev_initial".to_owned(),
        })
        .await
        .expect("import_cookie_account with raw Cookie header containing spaces");

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
        parsed_creds[0]
            .cookies
            .get("hatch_sess")
            .map(String::as_str),
        Some("sess_rotated_99")
    );
    assert_eq!(parsed_creds[0].vm_id.as_deref(), Some("vm_muse_777"));

    // 3. Test pasting full browser export JSON `{"url":"https://muse.ai","cookies":[...]}`
    //    with realistic multi-kilobyte `hatch_gw` / `hatch_vml` values (> 5 KiB raw JSON)
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
        .import_cookie_account(ImportCookieAccountRequest {
            vendor: GatewayVendor::Muse,
            label: "Muse Browser Export JSON".to_owned(),
            cookies: browser_export_payload,
        })
        .await
        .expect("import_cookie_account with browser export JSON");

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

    // 4. Cookies upstream reports as unauthorized are rejected instead of saved as a dead account.
    let unauthorized_app = Router::new().route(
        "/api/session",
        get(|| async { axum::http::StatusCode::UNAUTHORIZED }),
    );
    let unauthorized_listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let unauthorized_addr = unauthorized_listener.local_addr().expect("addr");
    tokio::spawn(async move {
        let _ = axum::serve(unauthorized_listener, unauthorized_app).await;
    });
    let mut unauthorized_drivers: BTreeMap<GatewayVendor, Arc<dyn OAuthVendorDriver>> =
        BTreeMap::new();
    unauthorized_drivers.insert(
        GatewayVendor::Muse,
        Arc::new(MuseOAuthDriver::with_test_endpoints(
            format!("http://{unauthorized_addr}/api/session"),
            format!("http://{unauthorized_addr}/api/hatch/vm/wake"),
        )),
    );
    let unauthorized_hub =
        LlmGatewayHub::with_drivers(Some(db.pool().clone()), cipher, unauthorized_drivers);
    let accounts_before = online_hub
        .list_accounts(Some(GatewayVendor::Muse))
        .await
        .expect("list before")
        .len();
    let rejected = unauthorized_hub
        .import_cookie_account(ImportCookieAccountRequest {
            vendor: GatewayVendor::Muse,
            label: "Muse Expired Cookie".to_owned(),
            cookies: "hatch_sess=expired; hatch_gw=g; hatch_vml=v; hatch_native_auth_device=d"
                .to_owned(),
        })
        .await;
    match rejected {
        Err(GatewayError::OAuthExchangeFailed(detail)) => {
            assert_eq!(detail, "muse_cookie_unauthorized");
        }
        Err(other) => panic!("expected unauthorized cookie rejection, got {other:?}"),
        Ok(_) => panic!("unauthorized cookie must not create an account"),
    }
    assert_eq!(
        online_hub
            .list_accounts(Some(GatewayVendor::Muse))
            .await
            .expect("list after")
            .len(),
        accounts_before
    );
}

#[tokio::test]
async fn gemini_antigravity_driver_oauth_onboard_user_and_dual_client_refresh() {
    use axum::{
        Router,
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::{get, post},
    };
    use serde_json::{Value, json};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::net::TcpListener;

    use crate::services::llm_gateway::{
        types::OAuthTokenBundle,
        vendors::gemini::{ANTIGRAVITY_USER_AGENT, GeminiOAuthDriver},
    };

    let onboard_calls = Arc::new(AtomicUsize::new(0));
    let onboard_calls_clone = Arc::clone(&onboard_calls);

    let app = Router::new()
        .route(
            "/token",
            post(|body: String| async move {
                let pairs: BTreeMap<String, String> = url::form_urlencoded::parse(body.as_bytes())
                    .into_owned()
                    .collect();
                let grant_type = pairs.get("grant_type").map(String::as_str).unwrap_or("");
                let client_id = pairs.get("client_id").map(String::as_str).unwrap_or("");

                match grant_type {
                    "authorization_code" => {
                        assert_eq!(
                            client_id,
                            "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com"
                        );
                        axum::Json(json!({
                            "access_token": "ya29.antigravity_login_tok_1",
                            "refresh_token": "1//antigravity_ref_1",
                            "expires_in": 3600
                        }))
                        .into_response()
                    }
                    "refresh_token" => {
                        let ref_tok = pairs.get("refresh_token").map(String::as_str).unwrap_or("");
                        if ref_tok == "1//legacy_gemini_cli_ref" {
                            // Reject Antigravity client_id for legacy token so driver falls back to GEMINI_CLI_CLIENT_ID
                            if client_id
                                != "681255809395-oo8ft2oprdrnp9e3aqf6av3hmdib135j.apps.googleusercontent.com"
                            {
                                return (
                                    StatusCode::UNAUTHORIZED,
                                    axum::Json(json!({"error": "unauthorized_client"})),
                                )
                                    .into_response();
                            }
                            return axum::Json(json!({
                                "access_token": "ya29.refreshed_legacy_cli_tok",
                                "expires_in": 3600
                            }))
                            .into_response();
                        }
                        assert_eq!(
                            client_id,
                            "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com"
                        );
                        axum::Json(json!({
                            "access_token": "ya29.refreshed_antigravity_tok",
                            "expires_in": 3600
                        }))
                        .into_response()
                    }
                    _ => StatusCode::BAD_REQUEST.into_response(),
                }
            }),
        )
        .route(
            "/userinfo",
            get(|| async move {
                axum::Json(json!({
                    "email": "lamphuchai03@gmail.com"
                }))
            }),
        )
        .route(
            "/v1internal:loadCodeAssist",
            post(|headers: HeaderMap, body: axum::body::Bytes| async move {
                let ua = headers
                    .get("user-agent")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("");
                assert_eq!(ua, ANTIGRAVITY_USER_AGENT);
                assert!(headers.get("x-goog-api-client").is_none());
                assert!(headers.get("client-metadata").is_none());

                let payload: Value = serde_json::from_slice(&body).expect("loadCodeAssist json");
                assert_eq!(
                    payload
                        .get("metadata")
                        .and_then(|m| m.get("ideType"))
                        .and_then(Value::as_i64),
                    Some(9)
                );

                // Simulate account needing onboardUser to provision cloudaicompanionProject
                axum::Json(json!({
                    "currentTier": { "id": "free-tier", "name": "Antigravity Pro" },
                    "paidTier": { "id": "g1-ultra-tier", "name": "Google AI Ultra" },
                    "allowedTiers": [
                        { "id": "legacy-tier", "isDefault": false },
                        { "id": "free-tier", "isDefault": true }
                    ]
                }))
            }),
        )
        .route(
            "/v1internal:onboardUser",
            post(move |headers: HeaderMap, body: axum::body::Bytes| {
                let counter = Arc::clone(&onboard_calls_clone);
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    let ua = headers
                        .get("user-agent")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("");
                    assert_eq!(ua, ANTIGRAVITY_USER_AGENT);
                    let payload: Value = serde_json::from_slice(&body).expect("onboardUser json");
                    assert_eq!(
                        payload.get("tierId").and_then(Value::as_str),
                        Some("free-tier")
                    );
                    axum::Json(json!({
                        "done": true,
                        "response": {
                            "cloudaicompanionProject": {
                                "id": "proj-antigravity-onboarded-01"
                            }
                        }
                    }))
                }
            }),
        )
        .route(
            "/v1internal:fetchAvailableModels",
            post(|headers: HeaderMap| async move {
                let auth = headers
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("");
                assert_eq!(
                    headers
                        .get("x-client-name")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or(""),
                    "antigravity"
                );
                if auth == "Bearer ya29.forbidden_quota_tok" {
                    return (
                        StatusCode::FORBIDDEN,
                        axum::Json(json!({
                            "error": {
                                "code": 403,
                                "message": "Your current account is not eligible for Antigravity. Verify your account to continue."
                            }
                        })),
                    )
                        .into_response();
                }
                axum::Json(json!({
                    "models": {
                        "gemini-3.8-flash-medium": {
                            "displayName": "Gemini 3.8 Flash (Medium)",
                            "quotaInfo": {
                                "remainingFraction": 0.9,
                                "resetTime": "2026-10-30T12:00:00Z"
                            }
                        }
                    }
                }))
                .into_response()
            }),
        )
        .route(
            "/v1internal:retrieveUserQuotaSummary",
            post(|| async move {
                axum::Json(json!({
                    "groups": [
                        {
                            "displayName": "Gemini Models",
                            "buckets": [
                                {
                                    "bucketId": "gemini-weekly",
                                    "displayName": "Weekly Limit Remaining",
                                    "window": "weekly",
                                    "remainingFraction": 0.8,
                                    "resetTime": "2026-11-06T00:00:00Z"
                                }
                            ]
                        }
                    ]
                }))
            }),
        );

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let driver = GeminiOAuthDriver::with_test_endpoints(&format!("http://{addr}"));

    // 1. Verify authorize URL uses Antigravity client_id and scopes
    let auth_url = driver
        .build_pkce_authorize_url("st_123", "chal_456", "http://localhost:8085/oauth2callback")
        .expect("authorize url");
    assert!(auth_url.contains(
        "client_id=1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com"
    ));
    assert!(auth_url.contains("cclog"));
    assert!(auth_url.contains("experimentsandconfigs"));

    // 2. Verify PKCE code exchange runs loadCodeAssist -> onboardUser fallback and extracts project ID
    let exchanged = driver
        .exchange_pkce_code(
            "auth_code_ag",
            "st_123",
            "verifier_456",
            "http://localhost:8085/oauth2callback",
        )
        .await
        .expect("exchange_pkce_code");
    assert_eq!(
        exchanged.account_email.as_deref(),
        Some("lamphuchai03@gmail.com")
    );
    assert_eq!(
        exchanged.bundle.account_context_id.as_deref(),
        Some("proj-antigravity-onboarded-01")
    );
    assert_eq!(onboard_calls.load(Ordering::SeqCst), 1);

    let extra_headers = driver.extra_headers_for(&exchanged.bundle);
    assert!(extra_headers.contains(&("User-Agent".to_owned(), ANTIGRAVITY_USER_AGENT.to_owned())));

    // 3. Verify dual-client refresh fallback for legacy GEMINI_CLI_CLIENT_ID token + lazy project ID discovery
    let legacy_bundle = OAuthTokenBundle {
        access_token: "ya29.old_expired".to_owned(),
        refresh_token: Some("1//legacy_gemini_cli_ref".to_owned()),
        upstream_oauth_token: None,
        account_context_id: None,
    };
    let refreshed_legacy = driver
        .refresh_token(&legacy_bundle)
        .await
        .expect("refresh legacy Gemini CLI token via fallback client_id");
    assert_eq!(
        refreshed_legacy.bundle.access_token,
        "ya29.refreshed_legacy_cli_tok"
    );
    assert_eq!(
        refreshed_legacy.bundle.account_context_id.as_deref(),
        Some("proj-antigravity-onboarded-01")
    );

    // 4. Verify Antigravity quota uses fetchAvailableModels + retrieveUserQuotaSummary
    let quota_ok = driver
        .fetch_quota(&exchanged.bundle)
        .await
        .expect("fetch_quota 200 OK");
    assert_eq!(quota_ok.plan.as_deref(), Some("Google AI Ultra"));
    assert_eq!(quota_ok.windows.len(), 2);
    assert_eq!(quota_ok.windows[0].name, "gemini-3.8-flash-medium");
    assert_eq!(quota_ok.windows[0].remaining_percentage, 90.0);
    assert_eq!(quota_ok.windows[1].name, "Gemini (Weekly)");
    assert_eq!(quota_ok.windows[1].remaining_percentage, 80.0);

    // 5. Verify HTTP 403 on quota endpoint returns graceful snapshot with message (9router parity, no gemini_quota_http_403 error)
    let forbidden_bundle = OAuthTokenBundle {
        access_token: "ya29.forbidden_quota_tok".to_owned(),
        refresh_token: Some("1//antigravity_ref_1".to_owned()),
        upstream_oauth_token: None,
        account_context_id: Some("proj-antigravity-onboarded-01".to_owned()),
    };
    let quota_403 = driver
        .fetch_quota(&forbidden_bundle)
        .await
        .expect("fetch_quota 403 must not fail with gemini_quota_http_403");
    assert!(quota_403.windows.is_empty());
    assert!(
        quota_403
            .message
            .as_deref()
            .is_some_and(|m| m.contains("not eligible for Antigravity"))
    );
}
