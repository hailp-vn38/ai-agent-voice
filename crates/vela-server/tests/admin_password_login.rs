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
/// A bearer value the removed static credential used to take; it must now be refused.
const RETIRED_STATIC_BEARER: &str = "retired-static-bearer-0123456789";

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
            "[api]\nenabled = true\njwt_secret = \"REPLACE_WITH_RANDOM_JWT_SECRET_OF_32_CHARS_OR_MORE\"\n",
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
        system_status(&client, &base, RETIRED_STATIC_BEARER).await,
        StatusCode::UNAUTHORIZED,
        "a static bearer is no longer a credential"
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
        format!("[api]\nenabled = true\njwt_secret = \"{secret}\"\ntoken_ttl_secs = {ttl}\n")
    };
    assert!(validated_prod_example(&api(JWT_SECRET, 3600), AUTH_TOKEN).is_ok());
    assert!(validated_prod_example("[api]\nenabled = true\n", AUTH_TOKEN).is_err());
    assert!(validated_prod_example(&api("too-short", 3600), AUTH_TOKEN).is_err());
    // 31 characters but more than 32 bytes: the minimum counts characters.
    assert!(validated_prod_example(&api(&"é".repeat(31), 3600), AUTH_TOKEN).is_err());
    assert!(validated_prod_example(&api(AUTH_TOKEN, 3600), AUTH_TOKEN).is_err());
    assert!(validated_prod_example(&api(JWT_SECRET, 0), AUTH_TOKEN).is_err());
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
async fn forced_change_flag_blocks_every_route_with_no_bypass() {
    let base = serve(&database_url(), JWT_SECRET, 3600).await;
    let client = Client::new();
    let token = login_token(&client, &base, "admin").await;
    assert_eq!(
        system_status(&client, &base, &token).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        system_status(&client, &base, RETIRED_STATIC_BEARER).await,
        StatusCode::UNAUTHORIZED
    );
}

#[test]
fn config_with_the_removed_static_key_fails_naming_jwt_secret() {
    let text = config_text(&database_url(), JWT_SECRET, 3600).replace(
        "jwt_secret =",
        "admin_token = \"old-value-that-is-not-migrated\"\njwt_secret =",
    );
    let error = parse(&text).unwrap_err();
    assert!(error.contains("admin_token"), "{error}");
    assert!(error.contains("jwt_secret"), "{error}");
}

#[tokio::test]
async fn password_change_writes_one_audit_event_without_secrets() {
    let base = serve(&database_url(), JWT_SECRET, 3600).await;
    let client = Client::new();
    let token = login_token(&client, &base, "admin").await;
    let new_password = "fresh-admin-pass";
    let changed = change_password(&client, &base, &token, "admin", new_password).await;
    assert_eq!(changed.status(), StatusCode::OK);
    let rotated = changed.json::<Value>().await.unwrap()["token"]
        .as_str()
        .unwrap()
        .to_owned();

    let page: Value = client
        .get(format!("{base}/system/audit-logs"))
        .bearer_auth(&rotated)
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
    assert_eq!(
        status_body(&client, &base).await["password_login_enabled"],
        json!(true)
    );
}

#[tokio::test]
async fn disabling_password_login_is_refused_while_the_forced_change_flag_is_set() {
    let base = serve(&database_url(), JWT_SECRET, 3600).await;
    let client = Client::new();
    let token = login_token(&client, &base, "admin").await;

    let response = set_password_login(&client, &base, Some(&token), false, "admin").await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(error_code(response).await, "password_change_required");
    assert_eq!(
        status_body(&client, &base).await["password_login_enabled"],
        json!(true)
    );
}

#[tokio::test]
async fn disabled_password_login_admits_requests_without_credentials_until_reenabled() {
    let base = serve(&database_url(), JWT_SECRET, 3600).await;
    let client = Client::new();
    let token = rotated_token(&client, &base).await;

    let off = set_password_login(&client, &base, Some(&token), false, "fresh-admin-pass").await;
    assert_eq!(off.status(), StatusCode::OK);
    assert_eq!(
        off.json::<Value>().await.unwrap()["password_login_enabled"],
        json!(false)
    );
    assert_eq!(
        status_body(&client, &base).await["password_login_enabled"],
        json!(false)
    );

    let open = client.get(format!("{base}/system")).send().await.unwrap();
    assert_eq!(open.status(), StatusCode::OK);
    let login_response = login(&client, &base, "fresh-admin-pass").await;
    assert_eq!(login_response.status(), StatusCode::CONFLICT);
    assert_eq!(error_code(login_response).await, "password_login_disabled");

    let wrong = set_password_login(&client, &base, None, true, "not-the-password").await;
    assert_eq!(wrong.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let on = set_password_login(&client, &base, None, true, "fresh-admin-pass").await;
    assert_eq!(on.status(), StatusCode::OK);
    assert_eq!(
        status_body(&client, &base).await["password_login_enabled"],
        json!(true)
    );

    let anonymous = client.get(format!("{base}/system")).send().await.unwrap();
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
    let relogin = login_token(&client, &base, "fresh-admin-pass").await;
    assert_eq!(
        system_status(&client, &base, &relogin).await,
        StatusCode::OK
    );
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
    assert_eq!(
        duplicated(&token).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );

    assert_eq!(
        set_password_login(&client, &base, Some(&token), false, "fresh-admin-pass")
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        duplicated(&token).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn toggle_failures_have_their_own_budget_and_do_not_lock_login() {
    let base = serve(&database_url(), JWT_SECRET, 3600).await;
    let client = Client::new();
    let token = rotated_token(&client, &base).await;

    for _ in 0..5 {
        let wrong =
            set_password_login(&client, &base, Some(&token), false, "not-the-password").await;
        assert_eq!(wrong.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
    let blocked = set_password_login(&client, &base, Some(&token), false, "fresh-admin-pass").await;
    assert_eq!(blocked.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(error_code(blocked).await, "too_many_attempts");
    assert_eq!(
        login(&client, &base, "fresh-admin-pass").await.status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn handler_refuses_disabling_login_while_forced_change_is_set_even_without_a_bearer() {
    let uri = database_url();
    let _first = serve(&uri, JWT_SECRET, 3600).await;
    // Login off with the forced-change flag still on: the middleware admits anyone, so only the
    // handler can refuse the toggle.
    let database = vela_server::database::Database::connect(&database_config(&uri))
        .await
        .unwrap();
    sqlx::query(
        "UPDATE admin_auth SET password_login_enabled = 0, must_change_password = 1 WHERE id = 1",
    )
    .execute(database.pool())
    .await
    .unwrap();
    drop(database);
    let base = serve(&uri, JWT_SECRET, 3600).await;
    let client = Client::new();

    let response = set_password_login(&client, &base, None, false, "admin").await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(error_code(response).await, "password_change_required");
}

#[tokio::test]
async fn change_password_wrong_current_is_throttled_after_five_failures() {
    let base = serve(&database_url(), JWT_SECRET, 3600).await;
    let client = Client::new();
    let token = rotated_token(&client, &base).await;

    for _ in 0..5 {
        let wrong =
            change_password(&client, &base, &token, "not-the-password", "another-pass-1").await;
        assert_eq!(wrong.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
    let blocked =
        change_password(&client, &base, &token, "fresh-admin-pass", "another-pass-1").await;
    assert_eq!(blocked.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(error_code(blocked).await, "too_many_attempts");
    // Login keeps its own budget.
    assert_eq!(
        login(&client, &base, "fresh-admin-pass").await.status(),
        StatusCode::OK
    );
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
        .bearer_auth(&fresh)
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
    assert_eq!(
        rows.len(),
        3,
        "one event for each of change, logout and toggle"
    );
    let logout = rows
        .iter()
        .find(|row| row["action"] == json!("logout_everywhere"))
        .unwrap();
    assert_eq!(logout["prior_revision"], json!(2));
    assert_eq!(logout["new_revision"], json!(3));
    let toggle = rows
        .iter()
        .find(|row| row["action"] == json!("password_login_set"))
        .unwrap();
    assert_eq!(toggle["outcome"], json!("success"));
    for row in rows {
        assert!(!row.to_string().contains("fresh-admin-pass"));
    }
}

fn database_config(database_uri: &str) -> vela_server::config::DatabaseConfig {
    parse(&config_text(database_uri, JWT_SECRET, 3600))
        .unwrap()
        .database
}

#[tokio::test]
async fn reset_returns_default_password_and_forces_change_after_restart() {
    let uri = database_url();
    let base = serve(&uri, JWT_SECRET, 3600).await;
    let client = Client::new();
    let old_token = rotated_token(&client, &base).await;
    assert_eq!(
        system_status(&client, &base, &old_token).await,
        StatusCode::OK
    );

    vela_server::app::reset_admin_password(&database_config(&uri))
        .await
        .unwrap();

    let restarted = serve(&uri, JWT_SECRET, 3600).await;
    assert_eq!(
        system_status(&client, &restarted, &old_token).await,
        StatusCode::UNAUTHORIZED,
        "a token minted before the reset is revoked once the server restarts"
    );
    let old_password = login(&client, &restarted, "fresh-admin-pass").await;
    assert_eq!(old_password.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(error_code(old_password).await, "invalid_password");

    let response = login(&client, &restarted, "admin").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["must_change_password"], json!(true));
    let token = body["token"].as_str().unwrap();
    assert_eq!(
        system_status(&client, &restarted, token).await,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn reset_reenables_password_login_when_it_was_switched_off() {
    let uri = database_url();
    let base = serve(&uri, JWT_SECRET, 3600).await;
    let client = Client::new();
    let _ = login_token(&client, &base, "admin").await;
    let pool = sqlx::SqlitePool::connect(&uri).await.unwrap();
    sqlx::query("UPDATE admin_auth SET password_login_enabled = 0 WHERE id = 1")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    vela_server::app::reset_admin_password(&database_config(&uri))
        .await
        .unwrap();

    let restarted = serve(&uri, JWT_SECRET, 3600).await;
    let status: Value = client
        .get(format!("{restarted}/auth/status"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        status,
        json!({ "password_login_enabled": true, "must_change_password": true })
    );
}

#[tokio::test]
async fn reset_command_prints_restart_notice_and_no_password() {
    let uri = database_url();
    let config_path =
        std::env::temp_dir().join(format!("vela-reset-{}.toml", uuid::Uuid::new_v4()));
    fs::write(&config_path, config_text(&uri, JWT_SECRET, 3600)).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_vela-server"))
        .arg("reset-admin-password")
        .env("VELA_CONFIG", &config_path)
        .output()
        .unwrap();
    fs::remove_file(&config_path).unwrap();
    assert!(output.status.success(), "reset command failed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.to_lowercase().contains("restart"),
        "the command must say a running server needs a restart: {stdout}"
    );
    assert!(!stdout.contains("fresh-admin-pass"));
}

#[tokio::test]
async fn reset_writes_one_audit_event_without_secrets() {
    let uri = database_url();
    let base = serve(&uri, JWT_SECRET, 3600).await;
    let client = Client::new();
    let _ = rotated_token(&client, &base).await;

    vela_server::app::reset_admin_password(&database_config(&uri))
        .await
        .unwrap();

    let restarted = serve(&uri, JWT_SECRET, 3600).await;
    let after_reset = rotated_token(&client, &restarted).await;
    let page: Value = client
        .get(format!("{restarted}/system/audit-logs"))
        .bearer_auth(&after_reset)
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
        .filter(|row| row["action"] == json!("password_reset"))
        .collect();
    assert_eq!(rows.len(), 1, "exactly one audit event per reset");
    assert_eq!(rows[0]["resource_type"], json!("admin_auth"));
    assert_eq!(rows[0]["outcome"], json!("success"));
    assert_eq!(rows[0]["prior_revision"], json!(2));
    assert_eq!(rows[0]["new_revision"], json!(3));
    let serialized = rows[0].to_string();
    assert!(!serialized.contains("fresh-admin-pass"));
    assert!(!serialized.contains("pbkdf2"));
}

async fn export_bytes(client: &Client, base: &str, bearer: &str, format: &str) -> Vec<u8> {
    let response = client
        .get(format!(
            "{base}/system/backup/export?format={format}&scope=full"
        ))
        .bearer_auth(bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    response.bytes().await.unwrap().to_vec()
}

async fn import_status(client: &Client, base: &str, bearer: &str, payload: Vec<u8>) -> StatusCode {
    client
        .post(format!(
            "{base}/system/backup/import?mode=merge&scope=full&clear_unreadable_credentials=true"
        ))
        .bearer_auth(bearer)
        .body(payload)
        .send()
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn backup_import_keeps_target_password_login_and_token() {
    let client = Client::new();
    let source = serve(&database_url(), JWT_SECRET, 3600).await;
    let source_token = rotated_token(&client, &source).await;
    let bundle = export_bytes(&client, &source, &source_token, "json").await;
    let snapshot = export_bytes(&client, &source, &source_token, "sqlite").await;
    let bundle_text = String::from_utf8_lossy(&bundle);
    assert!(!bundle_text.contains("pbkdf2"));
    assert!(!bundle_text.contains("password_login"));

    let target = serve(&database_url(), JWT_SECRET, 3600).await;
    let target_token = login_token(&client, &target, "admin").await;
    let changed = change_password(
        &client,
        &target,
        &target_token,
        "admin",
        "target-admin-pass",
    )
    .await;
    assert_eq!(changed.status(), StatusCode::OK);
    let target_token = changed.json::<Value>().await.unwrap()["token"]
        .as_str()
        .unwrap()
        .to_owned();

    for payload in [bundle, snapshot] {
        assert_eq!(
            import_status(&client, &target, &target_token, payload).await,
            StatusCode::OK
        );
        assert_eq!(
            system_status(&client, &target, &target_token).await,
            StatusCode::OK,
            "the token minted on the target stays valid after import"
        );
        assert_eq!(
            login(&client, &target, "target-admin-pass").await.status(),
            StatusCode::OK,
            "the target keeps its own Admin Password"
        );
        assert_eq!(
            login(&client, &target, "fresh-admin-pass").await.status(),
            StatusCode::UNAUTHORIZED,
            "the source Admin Password is not imported"
        );
    }
}

#[tokio::test]
async fn concurrent_wrong_logins_cannot_exceed_the_failure_budget() {
    let base = serve(&database_url(), JWT_SECRET, 3600).await;
    let client = Client::new();

    let attempts: Vec<_> = (0..12)
        .map(|_| {
            let client = client.clone();
            let base = base.clone();
            tokio::spawn(async move { login(&client, &base, "not-the-password").await.status() })
        })
        .collect();
    let mut evaluated = 0;
    for attempt in attempts {
        match attempt.await.unwrap() {
            StatusCode::UNAUTHORIZED => evaluated += 1,
            StatusCode::TOO_MANY_REQUESTS => {}
            other => panic!("unexpected status {other}"),
        }
    }
    assert_eq!(
        evaluated, 5,
        "only the budget of five guesses is ever checked"
    );
}

#[tokio::test]
async fn reset_without_restart_still_forces_the_password_change() {
    let uri = database_url();
    let base = serve(&uri, JWT_SECRET, 3600).await;
    let client = Client::new();
    let _ = rotated_token(&client, &base).await;

    vela_server::app::reset_admin_password(&database_config(&uri))
        .await
        .unwrap();

    let response = login(&client, &base, "admin").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["must_change_password"], json!(true));
    let token = body["token"].as_str().unwrap();
    assert_eq!(
        system_status(&client, &base, token).await,
        StatusCode::FORBIDDEN,
        "the default password never yields an unrestricted token"
    );
}

#[tokio::test]
async fn failed_logins_write_no_audit_event() {
    let base = serve(&database_url(), JWT_SECRET, 3600).await;
    let client = Client::new();
    let token = rotated_token(&client, &base).await;
    for _ in 0..2 {
        assert_eq!(
            login(&client, &base, "not-the-password").await.status(),
            StatusCode::UNAUTHORIZED
        );
    }

    let page: Value = client
        .get(format!("{base}/system/audit-logs"))
        .bearer_auth(&token)
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
    assert_eq!(rows.len(), 1, "only the password change is audited");
    assert_eq!(rows[0]["action"], json!("password_change"));
}
