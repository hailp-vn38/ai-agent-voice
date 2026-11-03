use super::*;

#[tokio::test]
async fn llm_gateway_muse_auto_refresh_multi_tier_fallback_and_watchdog_recovery() {
    use axum::{
        Router,
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::{get, post},
    };
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::net::TcpListener;

    use super::super::vendors::muse::MuseOAuthDriver;
    use crate::providers::llm::gateway::muse::config::parse_accounts;

    // Mode 0: /api/session returns 200 with nested `vms: [{"id": "vm_nested_01"}]` (no top-level endpointUrl)
    //         and /api/hatch/token mints a fresh wss token with timeout:1893456000.
    // Mode 1: /api/session returns 403 (Cloudflare block), but SSR HTML `GET /` succeeds with RSC flight stream.
    // Mode 2: both /api/session and `GET /` return 503 (transient outage).
    let mode = Arc::new(AtomicUsize::new(0));
    let mode_session = Arc::clone(&mode);
    let mode_root = Arc::clone(&mode);

    let app = Router::new()
        .route(
            "/api/session",
            get(move |headers: HeaderMap| {
                let m = mode_session.load(Ordering::SeqCst);
                async move {
                    let cookie = headers
                        .get("cookie")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("");
                    assert!(cookie.contains("hatch_sess="));
                    match m {
                        0 => (
                            [(
                                axum::http::header::SET_COOKIE,
                                "hatch_sess=sess_rotated_tier1; Path=/; HttpOnly; Secure",
                            )],
                            axum::Json(json!({
                                "vms": [{ "id": "vm_nested_01" }],
                                "user": { "email": "muse-tier1@muse.ai" }
                            })),
                        )
                            .into_response(),
                        1 => (StatusCode::FORBIDDEN, "Cloudflare challenge").into_response(),
                        _ => (StatusCode::SERVICE_UNAVAILABLE, "upstream warming up").into_response(),
                    }
                }
            }),
        )
        .route(
            "/api/hatch/token",
            post(|| async move {
                axum::Json(json!({
                    "token": "tok_tier1_minted",
                    "notary_token": "v1.timeout:1893456000.sig"
                }))
            }),
        )
        .route(
            "/api/hatch/vm/wake",
            post(|| async move { axum::Json(json!({"ok": true})) }),
        )
        .route(
            "/",
            get(move || {
                let m = mode_root.load(Ordering::SeqCst);
                async move {
                    if m == 1 {
                        (
                            [(
                                axum::http::header::SET_COOKIE,
                                "hatch_sess=sess_rotated_ssr; Path=/; HttpOnly; Secure",
                            )],
                            r#"<html><script>self.__next_f.push([1,"{\"vmName\":\"vm_ssr_02\",\"authToken\":\"tok_ssr_minted\",\"noiseNotaryToken\":\"v1.timeout:1893499000.sig\"}"])</script></html>"#,
                        )
                            .into_response()
                    } else {
                        (StatusCode::SERVICE_UNAVAILABLE, "SSR down").into_response()
                    }
                }
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
    .expect("sqlite memory");
    let cipher: Arc<dyn crate::database::secrets::SecretResolver> =
        Arc::new(CredentialCipher::new(1, &[0x42; 32]).expect("cipher"));
    let mut drivers: BTreeMap<GatewayVendor, Arc<dyn OAuthVendorDriver>> = BTreeMap::new();
    drivers.insert(
        GatewayVendor::Muse,
        Arc::new(MuseOAuthDriver::with_test_endpoints(
            format!("http://{addr}/api/session"),
            format!("http://{addr}/api/hatch/vm/wake"),
        )),
    );
    let hub = Arc::new(LlmGatewayHub::with_drivers(
        Some(db.pool().clone()),
        cipher,
        drivers,
    ));

    // Seed a Muse account with an ALREADY-EXPIRED wss:// URL + valid hatch_sess cookie (hybrid credential)
    let expired_hybrid = "wss://hatch.metaaivm.com/v1/noise?vm_id=vm_old_expired&auth_token=tok_old&app_id=hatch-web&request_id=req1&notary_token=v1.timeout:1700000000.sig\nhatch_sess=sess_init; hatch_gw=gw_init; hatch_vml=vml_init; hatch_native_auth_device=dev_init";
    let acc = hub
        .import_cookie_account(ImportCookieAccountRequest {
            vendor: GatewayVendor::Muse,
            label: "Muse Hybrid Auto-Renew".to_owned(),
            cookies: expired_hybrid.to_owned(),
        })
        .await
        .expect("complete login");

    // 1. Verify expired wss_url was replaced by Tier 1 (/api/session nested vms[0] -> fallback endpointUrl -> /api/hatch/token)
    assert_eq!(acc.expires_at, Some(1_893_456_000));
    let lease1 = hub
        .acquire_lease(&GatewayBinding::Account(acc.account_key.clone()))
        .expect("lease 1");
    let creds1 = parse_accounts(lease1.access_token.expose());
    assert_eq!(creds1[0].vm_id.as_deref(), Some("vm_nested_01"));
    assert!(
        creds1[0]
            .wss_url
            .as_deref()
            .unwrap_or("")
            .contains("auth_token=tok_tier1_minted"),
        "expected fresh Tier-1 wss_url, got {:?}",
        creds1[0].wss_url
    );
    assert_eq!(
        creds1[0].cookies.get("hatch_sess").map(String::as_str),
        Some("sess_rotated_tier1")
    );

    // 2. Switch to Mode 1: /api/session returns 403 (Cloudflare block), SSR HTML `GET /` returns fresh RSC stream token.
    //    `refresh_account_now` must NOT fail with `oauth_exchange_failed: muse_session_unauthorized`!
    mode.store(1, Ordering::SeqCst);
    let refreshed_ssr = hub
        .refresh_account_now(&acc.account_key)
        .await
        .expect("refresh_account_now should fall back to SSR HTML when /api/session returns 403");
    assert_eq!(refreshed_ssr.status, GatewayAccountStatus::Active);
    assert_eq!(refreshed_ssr.expires_at, Some(1_893_499_000));
    let lease2 = hub
        .acquire_lease(&GatewayBinding::Account(acc.account_key.clone()))
        .expect("lease 2");
    let creds2 = parse_accounts(lease2.access_token.expose());
    assert_eq!(creds2[0].vm_id.as_deref(), Some("vm_ssr_02"));
    assert!(
        creds2[0]
            .wss_url
            .as_deref()
            .unwrap_or("")
            .contains("auth_token=tok_ssr_minted")
    );
    assert_eq!(
        creds2[0].cookies.get("hatch_sess").map(String::as_str),
        Some("sess_rotated_ssr")
    );

    // 3. Simulate an account stuck in SQLite with `status = 'reauth_required'` and `last_error = 'oauth_exchange_failed'`
    //    (e.g. from a prior outage). When `proactive_refresh_expiring` runs (watchdog), it must auto-recover the account!
    sqlx::query(
        "UPDATE llm_gateway_accounts SET status = 'reauth_required', last_error = 'oauth_exchange_failed', expires_at = 1700000000 WHERE account_key = ?",
    )
    .bind(&acc.account_key)
    .execute(db.pool())
    .await
    .expect("force reauth_required in sqlite");
    hub.reload_snapshot().await.expect("reload_snapshot");
    assert_eq!(
        hub.acquire_lease(&GatewayBinding::Account(acc.account_key.clone()))
            .unwrap_err(),
        GatewayError::AccountReauthRequired
    );

    let recovered_count = hub
        .proactive_refresh_expiring(300)
        .await
        .expect("watchdog proactive_refresh_expiring");
    assert_eq!(
        recovered_count, 1,
        "watchdog must auto-recover Muse account from oauth_exchange_failed"
    );

    let accounts_after = hub
        .list_accounts(Some(GatewayVendor::Muse))
        .await
        .expect("list accounts");
    let recovered_view = accounts_after
        .iter()
        .find(|a| a.account_key == acc.account_key)
        .expect("recovered view");
    assert_eq!(recovered_view.status, GatewayAccountStatus::Active);
    assert_eq!(recovered_view.last_error, None);
    assert!(
        hub.acquire_lease(&GatewayBinding::Account(acc.account_key.clone()))
            .is_ok()
    );
}
