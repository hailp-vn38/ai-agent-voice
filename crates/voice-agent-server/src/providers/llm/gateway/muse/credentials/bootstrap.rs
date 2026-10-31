use reqwest::header::{CONTENT_TYPE, COOKIE, ORIGIN, REFERER, USER_AGENT};
use serde_json::{Value, json};

use super::super::pool::UpstreamFailureKind;
use super::{BootstrappedMuseSession, DEFAULT_USER_AGENT, build_wss_url, merge_set_cookies};

/// Bootstraps a fresh Hatch VM WebSocket URL from a Muse session cookie via:
/// 1. REST API: `GET {base}/api/session?vm_state=live` -> `POST {base}/api/hatch/token`
/// 2. SSR / RSC HTML stream: `GET {base}/`
pub(crate) async fn fetch_wss_from_cookie_http(
    http: &reqwest::Client,
    cookie_header: &str,
    base_url: &str,
) -> Result<BootstrappedMuseSession, (UpstreamFailureKind, String)> {
    let base = base_url.trim_end_matches('/');
    let mut current_cookie = cookie_header.to_string();

    // Tier 1: REST API (/api/session?vm_state=live -> /api/hatch/token)
    let session_resp = http
        .get(format!("{base}/api/session?vm_state=live"))
        .header(COOKIE, &current_cookie)
        .header(USER_AGENT, DEFAULT_USER_AGENT)
        .header(REFERER, format!("{base}/"))
        .send()
        .await;

    if let Ok(resp) = session_resp {
        let status = resp.status().as_u16();
        current_cookie = merge_set_cookies(&current_cookie, resp.headers());
        if status == 401 || status == 403 {
            return Err((
                UpstreamFailureKind::AuthInvalid,
                format!("Muse session unauthorized (HTTP {status})"),
            ));
        }
        if resp.status().is_success()
            && let Ok(sess_data) = resp.json::<Value>().await
        {
            let vm_id = sess_data
                .get("vmId")
                .or_else(|| sess_data.get("vm_id"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToOwned::to_owned);
            let endpoint_url = sess_data
                .get("endpointUrl")
                .or_else(|| sess_data.get("endpoint_url"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToOwned::to_owned);
            let discovered_email = sess_data
                .get("email")
                .and_then(Value::as_str)
                .or_else(|| {
                    sess_data
                        .get("user")
                        .and_then(|u| u.get("email"))
                        .and_then(Value::as_str)
                })
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToOwned::to_owned);

            if let (Some(vm_id), Some(endpoint_url)) = (vm_id.clone(), endpoint_url) {
                let token_resp = http
                    .post(format!("{base}/api/hatch/token"))
                    .header(COOKIE, &current_cookie)
                    .header(USER_AGENT, DEFAULT_USER_AGENT)
                    .header(ORIGIN, base)
                    .header(REFERER, format!("{base}/"))
                    .header(CONTENT_TYPE, "application/json")
                    .json(&json!({
                        "vmAddress": endpoint_url,
                        "vmName": vm_id,
                    }))
                    .send()
                    .await;

                if let Ok(t_resp) = token_resp {
                    current_cookie = merge_set_cookies(&current_cookie, t_resp.headers());
                    if t_resp.status().is_success()
                        && let Ok(token_data) = t_resp.json::<Value>().await
                        && let Some(auth_token) = token_data
                            .get("token")
                            .or_else(|| token_data.get("authToken"))
                            .and_then(Value::as_str)
                            .filter(|s| !s.is_empty())
                    {
                        let notary_token = token_data
                            .get("notary_token")
                            .or_else(|| token_data.get("noiseNotaryToken"))
                            .or_else(|| token_data.get("notaryToken"))
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        let wss_url = build_wss_url(&vm_id, auth_token, notary_token);
                        return Ok(BootstrappedMuseSession {
                            wss_url,
                            cookie_header: current_cookie,
                            vm_id: Some(vm_id),
                            discovered_email,
                        });
                    }
                }
            }
        }
    }

    // Tier 2: SSR HTML / Next.js RSC Flight Stream
    let page_resp = http
        .get(format!("{base}/"))
        .header(COOKIE, &current_cookie)
        .header(USER_AGENT, DEFAULT_USER_AGENT)
        .send()
        .await
        .map_err(|e| (UpstreamFailureKind::Transient, format!("GET {base}/ failed: {e}")))?;

    let page_status = page_resp.status().as_u16();
    current_cookie = merge_set_cookies(&current_cookie, page_resp.headers());
    if page_status == 401 || page_status == 403 {
        return Err((
            UpstreamFailureKind::AuthInvalid,
            format!("GET {base}/ unauthorized (HTTP {page_status})"),
        ));
    }
    let html = page_resp
        .text()
        .await
        .map_err(|e| (UpstreamFailureKind::Transient, format!("failed reading {base}/ body: {e}")))?;

    if let Some((wss_url, vm_id)) = extract_wss_from_ssr_html(&html) {
        return Ok(BootstrappedMuseSession {
            wss_url,
            cookie_header: current_cookie,
            vm_id: Some(vm_id),
            discovered_email: None,
        });
    }

    Err((
        UpstreamFailureKind::Transient,
        "Failed to bootstrap Muse WSS URL from cookie via REST API or SSR HTML".to_string(),
    ))
}

pub(crate) fn extract_wss_from_ssr_html(html: &str) -> Option<(String, String)> {
    let unescaped = html.replace("\\\"", "\"");
    let vm_id = extract_json_str_field(&unescaped, "vmName")
        .or_else(|| extract_json_str_field(&unescaped, "vmId"))
        .or_else(|| extract_json_str_field(&unescaped, "vm_id"))?;
    let auth_token = extract_json_str_field(&unescaped, "authToken")?;
    let notary_token = extract_json_str_field(&unescaped, "noiseNotaryToken").unwrap_or_default();
    Some((build_wss_url(&vm_id, &auth_token, &notary_token), vm_id))
}

fn extract_json_str_field(text: &str, key: &str) -> Option<String> {
    let pattern = format!("\"{key}\":\"");
    let start = text.find(&pattern)? + pattern.len();
    let rest = &text[start..];
    let end = rest.find('"')?;
    let val = &rest[..end];
    if val.is_empty() {
        None
    } else {
        Some(val.to_string())
    }
}
