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

    assert_eq!(system_status(&client, &base, token).await, StatusCode::OK);
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
    let token = login_token(&client, &base, "admin").await;
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
    let token = login_token(&client, &base, "admin").await;

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
