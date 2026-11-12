//! Shared Admin Web Token helper for tests.
//!
//! Included by path from integration test crates (`tests/*.rs`) and from in-crate `#[cfg(test)]`
//! modules, so there is exactly one implementation. Depends only on `reqwest` and `serde_json`.
#![allow(dead_code)]

use reqwest::Client;
use serde_json::{Value, json};

/// The password the harness sets on every fresh Admin database.
pub const HARNESS_ADMIN_PASSWORD: &str = "harness-admin-password";

/// PBKDF2-HMAC-SHA256 hash of the default Admin Password `admin` in the server's stored format,
/// with a fixed test salt. Test-only: lets fixtures that build via `router_with_state` (which skips
/// `admin::auth::prepare`) seed the same default password that a fresh deployment gets.
pub const DEFAULT_ADMIN_PASSWORD_HASH: &str =
    "pbkdf2_sha256$600000$dmVsYS10ZXN0LXNhbHQtMQ==$8i7x1mKuOT1GN7AKUR9U0epYooK/yr+vVNbZt6763iM=";

/// Seeds the default Admin Password into `admin_auth` the way `prepare` does on a fresh database.
/// Call before building the router; the in-memory auth status keeps its defaults.
pub async fn seed_default_admin_password(pool: &sqlx::SqlitePool) {
    sqlx::query(
        "UPDATE admin_auth
         SET password_login_enabled = 1, password_hash = ?, must_change_password = 1, updated_at = 1
         WHERE id = 1 AND password_hash = ''",
    )
    .bind(DEFAULT_ADMIN_PASSWORD_HASH)
    .execute(pool)
    .await
    .unwrap();
}

/// Logs in with the default Admin Password, rotates it to `HARNESS_ADMIN_PASSWORD`, and returns a
/// usable Admin Web Token. Tests that call protected Admin routes use this instead of `admin_token`.
/// Call it once per freshly started server: the default password is gone after the first call.
pub async fn admin_web_token(client: &Client, base: &str) -> String {
    let login: Value = client
        .post(format!("{base}/api/admin/auth/login"))
        .json(&json!({ "password": "admin" }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
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
