//! Shared Admin Web Token helper for test binaries that call protected Admin routes.
#![allow(dead_code)]

use reqwest::Client;
use serde_json::{Value, json};

/// The password the harness sets on every fresh Admin database.
pub const HARNESS_ADMIN_PASSWORD: &str = "harness-admin-password";

/// Test-only PBKDF2 hash of the default Admin Password `admin`, in the production stored format.
/// Used only where a harness cannot run the bootstrap that seeds it.
pub const DEFAULT_ADMIN_PASSWORD_HASH_FOR_TESTS: &str =
    "pbkdf2_sha256$600000$AAECAwQFBgcICQoLDA0ODw==$g0LiQ+9SwgYmC2xCxrbp1T5rhqxCi4uOCBbV8XwRX/8=";

/// Seeds the default Admin Password on a database whose server was built without the bootstrap.
/// Mirrors the production seed: only an empty hash is replaced.
pub async fn seed_default_admin_password(pool: &sqlx::SqlitePool) {
    sqlx::query(
        "UPDATE admin_auth SET password_login_enabled = 1, password_hash = ?, must_change_password = 1, updated_at = 0 WHERE id = 1 AND password_hash = ''",
    )
    .bind(DEFAULT_ADMIN_PASSWORD_HASH_FOR_TESTS)
    .execute(pool)
    .await
    .unwrap();
}

/// Logs in with the default Admin Password, rotates it to `HARNESS_ADMIN_PASSWORD`, and returns a
/// usable Admin Web Token. A database that a previous harness server already rotated accepts the
/// rotated password directly. Tests that call protected Admin routes use this instead of a static credential.
pub async fn admin_web_token(client: &Client, base: &str) -> String {
    // A server that already issued a token accepts it again; logging in repeatedly would
    // trip the failed-attempt limiter on the default-password probe below.
    let cached = TOKENS.lock().unwrap().get(base).cloned();
    if let Some(token) = cached {
        let probe = client
            .get(format!("{base}/api/admin/system"))
            .bearer_auth(&token)
            .send()
            .await;
        if probe.is_ok_and(|response| response.status() == reqwest::StatusCode::OK) {
            return token;
        }
    }
    let token = fresh_admin_web_token(client, base).await;
    TOKENS
        .lock()
        .unwrap()
        .insert(base.to_owned(), token.clone());
    token
}

static TOKENS: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, String>>> =
    std::sync::LazyLock::new(Default::default);

async fn fresh_admin_web_token(client: &Client, base: &str) -> String {
    let login = client
        .post(format!("{base}/api/admin/auth/login"))
        .json(&json!({ "password": "admin" }))
        .send()
        .await
        .unwrap();
    if login.status() == reqwest::StatusCode::UNAUTHORIZED {
        let rotated: Value = client
            .post(format!("{base}/api/admin/auth/login"))
            .json(&json!({ "password": HARNESS_ADMIN_PASSWORD }))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        return rotated["token"].as_str().unwrap().to_owned();
    }
    let login: Value = login.error_for_status().unwrap().json().await.unwrap();
    let first_token = login["token"].as_str().unwrap();
    let changed: Value = client
        .post(format!("{base}/api/admin/auth/password"))
        .bearer_auth(first_token)
        .json(&json!({
            "current_password": "admin",
            "new_password": HARNESS_ADMIN_PASSWORD,
        }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    changed["token"].as_str().unwrap().to_owned()
}
