//! Admin Web Token helper shared by test binaries that call protected Admin routes.
//! Same contract as `admin_api/harness.rs::admin_web_token`, but idempotent: a database
//! already rotated to `HARNESS_ADMIN_PASSWORD` logs in directly. A fresh database logs in
//! with the default Admin Password, rotates it, and returns the rotated token.
#![allow(dead_code)]

use reqwest::Client;
use serde_json::{Value, json};

/// The password the harness sets on every fresh Admin database.
pub const HARNESS_ADMIN_PASSWORD: &str = "harness-admin-password";

async fn login(client: &Client, base: &str, password: &str) -> Option<String> {
    let response = client
        .post(format!("{base}/api/admin/auth/login"))
        .json(&json!({ "password": password }))
        .send()
        .await
        .unwrap();
    if !response.status().is_success() {
        return None;
    }
    let body: Value = response.json().await.unwrap();
    body["token"].as_str().map(str::to_owned)
}

pub async fn admin_web_token(client: &Client, base: &str) -> String {
    if let Some(token) = login(client, base, HARNESS_ADMIN_PASSWORD).await {
        return token;
    }
    let first_token = login(client, base, "admin")
        .await
        .expect("default Admin Password login");
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

/// PBKDF2-HMAC-SHA256 hash of the default Admin Password `admin`, in the stored format
/// `pbkdf2_sha256$<iterations>$<salt>$<hash>`. Test-only: the bootstrap-less harnesses build their
/// state without `admin::auth::prepare`, so they seed this row themselves.
pub const DEFAULT_ADMIN_PASSWORD_HASH: &str =
    "pbkdf2_sha256$600000$uBmJx/e3RYkR5D1C8u3j1Q==$ETgyNpX8AlaFG1mordZzAdcDHcJ+mc+PVSHkVkXqs6Q=";

/// Seeds the default Admin Password on a database whose state is built without bootstrap.
pub async fn seed_default_admin(pool: &sqlx::SqlitePool) {
    sqlx::query(
        "UPDATE admin_auth SET password_login_enabled = 1, password_hash = ?, must_change_password = 1
         WHERE id = 1 AND password_hash = ''",
    )
    .bind(DEFAULT_ADMIN_PASSWORD_HASH)
    .execute(pool)
    .await
    .unwrap();
}
