//! Admin Password login through the public Admin HTTP boundary: default seed, login, status,
//! limiter, token rejection cases, restart behaviour and config validation.

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::{Client, StatusCode};
use serde_json::{Value, json};
use std::{fs, sync::Arc, time::Duration};
use vela_server::{
    app::bootstrap_with_providers_and_secret_resolver, config::AppConfig,
    database::secrets::EnvSecretResolver, providers::ProviderSet,
};

const JWT_SECRET: &str = "admin-signing-secret-0123456789abcdef";
const OTHER_SECRET: &str = "a-different-signing-secret-0123456789";
const AUTH_TOKEN: &str = "voice-ota-token-not-the-same-0123456789";
const ADMIN_TOKEN: &str = "admin-test-token";

fn database_url() -> String {
    format!(
        "sqlite://{}",
        std::env::temp_dir()
            .join(format!("vela-admin-login-{}", uuid::Uuid::new_v4()))
            .join("vela.db")
            .display()
    )
}

fn config_text(database_uri: &str, jwt_secret: &str, ttl_secs: u64) -> String {
    format!(
        r#"
[server]
bind = "127.0.0.1:0"
public_ws_url = "ws://127.0.0.1:0/voice/v1/"
[provider_defaults]
vad = "test"
asr = "test"
llm = "test"
tts = "test"
[database]
url = "{database_uri}"
[auth]
token = "{AUTH_TOKEN}"
[api]
enabled = true
admin_token = "{ADMIN_TOKEN}"
jwt_secret = "{jwt_secret}"
token_ttl_secs = {ttl_secs}
"#
    )
}

fn parse(text: &str) -> Result<AppConfig, String> {
    let path = std::env::temp_dir().join(format!("vela-admin-login-{}.toml", uuid::Uuid::new_v4()));
    fs::write(&path, text).unwrap();
    let parsed = AppConfig::parse_and_resolve(&path).map_err(|error| error.to_string());
    fs::remove_file(path).unwrap();
    parsed
}

/// The production example is a configuration that validates; only its Admin and auth values vary.
fn validated_prod_example(api_block: &str, voice_token: &str) -> Result<AppConfig, String> {
    let text = include_str!("../../../docker/config.prod.example.toml")
        .replace(
            "[api]\nenabled = true\nadmin_token = \"REPLACE_WITH_ADMIN_TOKEN\"\n",
            api_block,
        )
        .replace(
            "token = \"REPLACE_WITH_VOICE_TOKEN\"",
            &format!("token = \"{voice_token}\""),
        );
    let config: AppConfig = toml::from_str(&text).map_err(|error| error.to_string())?;
    config.validate().map_err(|error| error.to_string())?;
    Ok(config)
}

async fn serve(database_uri: &str, jwt_secret: &str, ttl_secs: u64) -> String {
    let config = parse(&config_text(database_uri, jwt_secret, ttl_secs)).unwrap();
    let router = bootstrap_with_providers_and_secret_resolver(
        config,
        Arc::new(ProviderSet::unavailable()),
        Arc::new(EnvSecretResolver),
    )
    .await
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    format!("http://{address}/api/admin")
}

async fn login(client: &Client, base: &str, password: &str) -> reqwest::Response {
    client
        .post(format!("{base}/auth/login"))
        .json(&json!({ "password": password }))
        .send()
        .await
        .unwrap()
}

async fn login_token(client: &Client, base: &str, password: &str) -> String {
    let response = login(client, base, password).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    body["token"].as_str().unwrap().to_owned()
}

/// Signs in with the default password and replaces it, returning a token that passes every route.
async fn rotated_token(client: &Client, base: &str) -> String {
    let token = login_token(client, base, "admin").await;
    let response = change_password(client, base, &token, "admin", "fresh-admin-pass").await;
    assert_eq!(response.status(), StatusCode::OK);
    response.json::<Value>().await.unwrap()["token"]
        .as_str()
        .unwrap()
        .to_owned()
}

async fn system_status(client: &Client, base: &str, bearer: &str) -> StatusCode {
    client
        .get(format!("{base}/system"))
        .bearer_auth(bearer)
        .send()
        .await
        .unwrap()
        .status()
}

fn forge(header: &str, body: &Value, signature: &str) -> String {
    format!(
        "{}.{}.{}",
        URL_SAFE_NO_PAD.encode(header),
        URL_SAFE_NO_PAD.encode(body.to_string()),
        signature
    )
}

#[tokio::test]
async fn fresh_database_logs_in_with_default_password_and_token_is_accepted() {
    let base = serve(&database_url(), JWT_SECRET, 3600).await;
    let client = Client::new();

    let response = login(&client, &base, "admin").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["must_change_password"], json!(true));
    assert!(body["expires_at"].as_i64().unwrap() > 0);
    let token = body["token"].as_str().unwrap();

    assert_eq!(
        system_status(&client, &base, token).await,
        StatusCode::FORBIDDEN,
        "a default-password token is limited to the password change until it is replaced"
    );
    assert_eq!(
        system_status(&client, &base, ADMIN_TOKEN).await,
        StatusCode::OK,
        "admin_token keeps working during the expand step"
    );
}

#[tokio::test]
async fn status_is_public_and_exposes_only_two_flags() {
    let base = serve(&database_url(), JWT_SECRET, 3600).await;
    let body: Value = Client::new()
        .get(format!("{base}/auth/status"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        body,
        json!({ "password_login_enabled": true, "must_change_password": true })
    );
}

#[tokio::test]
async fn wrong_password_is_401_then_limiter_blocks_after_five_failures() {
    let base = serve(&database_url(), JWT_SECRET, 3600).await;
    let client = Client::new();

    let wrong = login(&client, &base, "not-the-password").await;
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
    let body: Value = wrong.json().await.unwrap();
    assert_eq!(body["error"]["code"], json!("invalid_password"));
    assert!(!body.to_string().contains("not-the-password"));

    for _ in 0..4 {
        assert_eq!(
            login(&client, &base, "still-wrong").await.status(),
            StatusCode::UNAUTHORIZED
        );
    }
    let locked = login(&client, &base, "admin").await;
    assert_eq!(locked.status(), StatusCode::TOO_MANY_REQUESTS);
    let retry_after: u64 = locked
        .headers()
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
        .expect("Retry-After header");
    assert!((1..=60).contains(&retry_after));
    let body: Value = locked.json().await.unwrap();
    assert_eq!(body["error"]["code"], json!("too_many_attempts"));
}

#[tokio::test]
async fn expired_token_is_rejected() {
    let base = serve(&database_url(), JWT_SECRET, 1).await;
    let client = Client::new();
    let token = rotated_token(&client, &base).await;
    assert_eq!(system_status(&client, &base, &token).await, StatusCode::OK);
    tokio::time::sleep(Duration::from_millis(2100)).await;
    assert_eq!(
        system_status(&client, &base, &token).await,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn altered_foreign_and_unsigned_tokens_are_rejected() {
    let database = database_url();
    let base = serve(&database, JWT_SECRET, 3600).await;
    let client = Client::new();
    let token = login_token(&client, &base, "admin").await;

    let mut flipped = token.clone().into_bytes();
    let last = flipped.len() - 1;
    flipped[last] = if flipped[last] == b'A' { b'B' } else { b'A' };
    let flipped = String::from_utf8(flipped).unwrap();
    assert_eq!(
        system_status(&client, &base, &flipped).await,
        StatusCode::UNAUTHORIZED
    );

    let other = serve(&database_url(), OTHER_SECRET, 3600).await;
    let foreign = login_token(&client, &other, "admin").await;
    assert_eq!(
        system_status(&client, &base, &foreign).await,
        StatusCode::UNAUTHORIZED
    );

    let now = 4_000_000_000i64;
    let claims = json!({ "iat": now - 10, "exp": now + 3600, "rev": 1 });
    let none = forge(r#"{"alg":"none","typ":"JWT"}"#, &claims, "");
    assert_eq!(
        system_status(&client, &base, &none).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        system_status(&client, &base, "not-a-token").await,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn restart_on_same_database_keeps_tokens_and_new_secret_invalidates_them() {
    let database = database_url();
    let base = serve(&database, JWT_SECRET, 3600).await;
    let client = Client::new();
    let token = rotated_token(&client, &base).await;

    let restarted = serve(&database, JWT_SECRET, 3600).await;
    assert_eq!(
        system_status(&client, &restarted, &token).await,
        StatusCode::OK
    );

    let rotated = serve(&database, OTHER_SECRET, 3600).await;
    assert_eq!(
        system_status(&client, &rotated, &token).await,
        StatusCode::UNAUTHORIZED
    );
}

#[test]
fn config_without_valid_signing_secret_fails_validation() {
    let api = |secret: &str, ttl: u64| {
        format!(
            "[api]\nenabled = true\nadmin_token = \"{ADMIN_TOKEN}\"\njwt_secret = \"{secret}\"\ntoken_ttl_secs = {ttl}\n"
        )
    };
    assert!(validated_prod_example(&api(JWT_SECRET, 3600), AUTH_TOKEN).is_ok());
    assert!(
        validated_prod_example("[api]\nenabled = true\nadmin_token = \"x\"\n", AUTH_TOKEN).is_ok()
    );
    assert!(validated_prod_example(&api("too-short", 3600), AUTH_TOKEN).is_err());
    assert!(validated_prod_example(&api(AUTH_TOKEN, 3600), AUTH_TOKEN).is_err());
    assert!(validated_prod_example(&api(JWT_SECRET, 0), AUTH_TOKEN).is_err());
}

#[tokio::test]
async fn routes_are_absent_without_signing_secret() {
    let config = parse(&config_text(&database_url(), "", 3600)).unwrap();
    let router = bootstrap_with_providers_and_secret_resolver(
        config,
        Arc::new(ProviderSet::unavailable()),
        Arc::new(EnvSecretResolver),
    )
    .await
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let response = Client::new()
        .post(format!("http://{address}/api/admin/auth/login"))
        .json(&json!({ "password": "admin" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

async fn change_password(
    client: &Client,
    base: &str,
    bearer: &str,
    current: &str,
    new: &str,
) -> reqwest::Response {
    client
        .post(format!("{base}/auth/password"))
        .bearer_auth(bearer)
        .json(&json!({ "current_password": current, "new_password": new }))
        .send()
        .await
        .unwrap()
}

async fn error_code(response: reqwest::Response) -> String {
    let body: Value = response.json().await.unwrap();
    body["error"]["code"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn default_password_token_is_limited_to_password_change_and_status() {
    let base = serve(&database_url(), JWT_SECRET, 3600).await;
    let client = Client::new();
    let token = login_token(&client, &base, "admin").await;

    let blocked = client
        .get(format!("{base}/system"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(blocked.status(), StatusCode::FORBIDDEN);
    assert_eq!(error_code(blocked).await, "password_change_required");

    let status: Value = client
        .get(format!("{base}/auth/status"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["must_change_password"], json!(true));
}

#[tokio::test]
async fn changing_password_returns_a_usable_token_and_clears_the_flag() {
    let base = serve(&database_url(), JWT_SECRET, 3600).await;
    let client = Client::new();
    let old_token = login_token(&client, &base, "admin").await;

    let response = change_password(&client, &base, &old_token, "admin", "fresh-admin-pass").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["must_change_password"], json!(false));
    let new_token = body["token"].as_str().unwrap().to_owned();

    assert_eq!(
        system_status(&client, &base, &new_token).await,
        StatusCode::OK
    );
    assert_eq!(
        system_status(&client, &base, &old_token).await,
        StatusCode::UNAUTHORIZED,
        "a token minted before the change is revoked"
    );
    let status: Value = client
        .get(format!("{base}/auth/status"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["must_change_password"], json!(false));

    let relogin = login(&client, &base, "admin").await;
    assert_eq!(relogin.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(error_code(relogin).await, "invalid_password");
    assert_eq!(
        login(&client, &base, "fresh-admin-pass").await.status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn password_change_rejects_wrong_current_and_weak_new_passwords() {
    let base = serve(&database_url(), JWT_SECRET, 3600).await;
    let client = Client::new();
    let token = login_token(&client, &base, "admin").await;

    let wrong_current = change_password(&client, &base, &token, "not-it", "fresh-admin-pass").await;
    assert_eq!(wrong_current.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(error_code(wrong_current).await, "invalid_current_password");

    let default_again = change_password(&client, &base, &token, "admin", "admin").await;
    assert_eq!(default_again.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(error_code(default_again).await, "password_is_default");

    let too_short = change_password(&client, &base, &token, "admin", "short").await;
    assert_eq!(too_short.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(error_code(too_short).await, "password_too_short");

    assert_eq!(
        system_status(&client, &base, &token).await,
        StatusCode::FORBIDDEN,
        "rejected changes leave the token and the forced-change flag untouched"
    );

    let changed = change_password(&client, &base, &token, "admin", "fresh-admin-pass").await;
    let current_token = changed.json::<Value>().await.unwrap()["token"]
        .as_str()
        .unwrap()
        .to_owned();
    let unchanged = change_password(
        &client,
        &base,
        &current_token,
        "fresh-admin-pass",
        "fresh-admin-pass",
    )
    .await;
    assert_eq!(unchanged.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(error_code(unchanged).await, "password_unchanged");
}

#[tokio::test]
async fn admin_token_is_not_blocked_by_the_forced_change_flag() {
    let base = serve(&database_url(), JWT_SECRET, 3600).await;
    let client = Client::new();
    let _ = login_token(&client, &base, "admin").await;
    assert_eq!(
        system_status(&client, &base, ADMIN_TOKEN).await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn password_change_writes_one_audit_event_without_secrets() {
    let base = serve(&database_url(), JWT_SECRET, 3600).await;
    let client = Client::new();
    let token = login_token(&client, &base, "admin").await;
    let new_password = "fresh-admin-pass";
    assert_eq!(
        change_password(&client, &base, &token, "admin", new_password)
            .await
            .status(),
        StatusCode::OK
    );

    let page: Value = client
        .get(format!("{base}/system/audit-logs"))
        .bearer_auth(ADMIN_TOKEN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let rows: Vec<&Value> = page["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["resource_type"] == json!("admin_auth"))
        .collect();
    assert_eq!(rows.len(), 1, "exactly one audit event per change");
    assert_eq!(rows[0]["action"], json!("password_change"));
    assert_eq!(rows[0]["outcome"], json!("success"));
    assert_eq!(rows[0]["prior_revision"], json!(1));
    assert_eq!(rows[0]["new_revision"], json!(2));
    let serialized = rows[0].to_string();
    assert!(!serialized.contains(new_password));
    assert!(!serialized.contains("admin\""));
}

#[tokio::test]
async fn password_change_survives_restart_on_the_same_database() {
    let uri = database_url();
    let base = serve(&uri, JWT_SECRET, 3600).await;
    let client = Client::new();
    let token = login_token(&client, &base, "admin").await;
    assert_eq!(
        change_password(&client, &base, &token, "admin", "fresh-admin-pass")
            .await
            .status(),
        StatusCode::OK
    );

    let restarted = serve(&uri, JWT_SECRET, 3600).await;
    assert_eq!(
        login(&client, &restarted, "admin").await.status(),
        StatusCode::UNAUTHORIZED
    );
    let body: Value = login(&client, &restarted, "fresh-admin-pass")
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(body["must_change_password"], json!(false));
}

async fn logout_everywhere(client: &Client, base: &str, bearer: &str) -> reqwest::Response {
    client
        .post(format!("{base}/auth/logout-all"))
        .bearer_auth(bearer)
        .send()
        .await
        .unwrap()
}

async fn set_password_login(
    client: &Client,
    base: &str,
    bearer: Option<&str>,
    enabled: bool,
    current: &str,
) -> reqwest::Response {
    let mut request = client
        .post(format!("{base}/auth/password-login"))
        .json(&json!({ "enabled": enabled, "current_password": current }));
    if let Some(bearer) = bearer {
        request = request.bearer_auth(bearer);
    }
    request.send().await.unwrap()
}

async fn status_body(client: &Client, base: &str) -> Value {
    client
        .get(format!("{base}/auth/status"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

#[tokio::test]
async fn logout_everywhere_revokes_earlier_tokens_and_returns_a_usable_one() {
    let base = serve(&database_url(), JWT_SECRET, 3600).await;
    let client = Client::new();
    let old = rotated_token(&client, &base).await;
    let other_browser = login_token(&client, &base, "fresh-admin-pass").await;

    let response = logout_everywhere(&client, &base, &old).await;
    assert_eq!(response.status(), StatusCode::OK);
    let fresh = response.json::<Value>().await.unwrap()["token"]
        .as_str()
        .unwrap()
        .to_owned();

    assert_eq!(
        system_status(&client, &base, &old).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        system_status(&client, &base, &other_browser).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(system_status(&client, &base, &fresh).await, StatusCode::OK);
}

#[tokio::test]
async fn disabling_password_login_requires_the_current_password() {
    let base = serve(&database_url(), JWT_SECRET, 3600).await;
    let client = Client::new();
    let token = rotated_token(&client, &base).await;

    let wrong = set_password_login(&client, &base, Some(&token), false, "not-the-password").await;
    assert_eq!(wrong.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(error_code(wrong).await, "invalid_current_password");
    assert_eq!(status_body(&client, &base).await["password_login_enabled"], json!(true));
}

#[tokio::test]
async fn disabling_password_login_is_refused_while_the_forced_change_flag_is_set() {
    let base = serve(&database_url(), JWT_SECRET, 3600).await;
    let client = Client::new();
    let token = login_token(&client, &base, "admin").await;

    let response = set_password_login(&client, &base, Some(&token), false, "admin").await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(error_code(response).await, "password_change_required");
    assert_eq!(status_body(&client, &base).await["password_login_enabled"], json!(true));
}

#[tokio::test]
async fn disabled_password_login_admits_requests_without_credentials_until_reenabled() {
    let base = serve(&database_url(), JWT_SECRET, 3600).await;
    let client = Client::new();
    let token = rotated_token(&client, &base).await;

    let off = set_password_login(&client, &base, Some(&token), false, "fresh-admin-pass").await;
    assert_eq!(off.status(), StatusCode::OK);
    assert_eq!(off.json::<Value>().await.unwrap()["password_login_enabled"], json!(false));
    assert_eq!(status_body(&client, &base).await["password_login_enabled"], json!(false));

    let open = client.get(format!("{base}/system")).send().await.unwrap();
    assert_eq!(open.status(), StatusCode::OK);
    let login_response = login(&client, &base, "fresh-admin-pass").await;
    assert_eq!(login_response.status(), StatusCode::CONFLICT);
    assert_eq!(error_code(login_response).await, "password_login_disabled");

    let wrong = set_password_login(&client, &base, None, true, "not-the-password").await;
    assert_eq!(wrong.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let on = set_password_login(&client, &base, None, true, "fresh-admin-pass").await;
    assert_eq!(on.status(), StatusCode::OK);
    assert_eq!(status_body(&client, &base).await["password_login_enabled"], json!(true));

    let anonymous = client.get(format!("{base}/system")).send().await.unwrap();
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
    let relogin = login_token(&client, &base, "fresh-admin-pass").await;
    assert_eq!(system_status(&client, &base, &relogin).await, StatusCode::OK);
}

#[tokio::test]
async fn duplicate_authorization_headers_are_rejected_in_both_login_states() {
    let base = serve(&database_url(), JWT_SECRET, 3600).await;
    let client = Client::new();
    let token = rotated_token(&client, &base).await;
    let duplicated = |bearer: &str| {
        client
            .get(format!("{base}/system"))
            .header(reqwest::header::AUTHORIZATION, format!("Bearer {bearer}"))
            .header(reqwest::header::AUTHORIZATION, format!("Bearer {bearer}"))
            .send()
    };
    assert_eq!(duplicated(&token).await.unwrap().status(), StatusCode::UNAUTHORIZED);

    assert_eq!(
        set_password_login(&client, &base, Some(&token), false, "fresh-admin-pass")
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(duplicated(&token).await.unwrap().status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn disabling_password_login_shares_the_login_failure_budget() {
    let base = serve(&database_url(), JWT_SECRET, 3600).await;
    let client = Client::new();
    let token = rotated_token(&client, &base).await;

    for _ in 0..5 {
        let wrong = set_password_login(&client, &base, Some(&token), false, "not-the-password").await;
        assert_eq!(wrong.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
    let blocked = set_password_login(&client, &base, Some(&token), false, "fresh-admin-pass").await;
    assert_eq!(blocked.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(error_code(blocked).await, "too_many_attempts");
}

#[tokio::test]
async fn logout_and_toggle_each_write_one_audit_event_without_secrets() {
    let base = serve(&database_url(), JWT_SECRET, 3600).await;
    let client = Client::new();
    let token = rotated_token(&client, &base).await;
    let fresh = logout_everywhere(&client, &base, &token).await;
    let fresh = fresh.json::<Value>().await.unwrap()["token"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        set_password_login(&client, &base, Some(&fresh), false, "fresh-admin-pass")
            .await
            .status(),
        StatusCode::OK
    );

    let page: Value = client
        .get(format!("{base}/system/audit-logs"))
        .bearer_auth(ADMIN_TOKEN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let rows: Vec<&Value> = page["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["resource_type"] == json!("admin_auth"))
        .collect();
    assert_eq!(rows.len(), 3, "one event for each of change, logout and toggle");
    let logout = rows.iter().find(|row| row["action"] == json!("logout_everywhere")).unwrap();
    assert_eq!(logout["prior_revision"], json!(2));
    assert_eq!(logout["new_revision"], json!(3));
    let toggle = rows.iter().find(|row| row["action"] == json!("password_login_set")).unwrap();
    assert_eq!(toggle["outcome"], json!("success"));
    for row in rows {
        assert!(!row.to_string().contains("fresh-admin-pass"));
    }
}
