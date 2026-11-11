//! Shared Admin Web Token helper for tests.
//!
//! Included by path from integration test crates (`tests/*.rs`) and from in-crate `#[cfg(test)]`
//! modules, so there is exactly one implementation. Depends only on `reqwest` and `serde_json`.
#![allow(dead_code)]

use reqwest::Client;
use serde_json::{Value, json};

/// The password the harness sets on every fresh Admin database.
pub const HARNESS_ADMIN_PASSWORD: &str = "harness-admin-password";

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
