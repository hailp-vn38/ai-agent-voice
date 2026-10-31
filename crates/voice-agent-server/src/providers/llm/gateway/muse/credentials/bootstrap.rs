use reqwest::header::{CONTENT_TYPE, COOKIE, ORIGIN, REFERER, USER_AGENT};
use serde_json::{Value, json};

use super::super::pool::UpstreamFailureKind;
use super::{BootstrappedMuseSession, DEFAULT_USER_AGENT, build_wss_url, merge_set_cookies};

pub(crate) fn parse_session_json(
    sess_data: &Value,
) -> (Option<String>, Option<String>, Option<String>) {
    let first_vm = sess_data
        .get("vms")
        .and_then(Value::as_array)
        .and_then(|arr| arr.first());

    let vm_id = sess_data
        .get("vmId")
        .or_else(|| sess_data.get("vm_id"))
        .or_else(|| sess_data.get("vm_name"))
        .or_else(|| sess_data.get("vmName"))
        .and_then(Value::as_str)
        .or_else(|| {
            first_vm.and_then(|v0| {
                v0.get("vm_id")
                    .or_else(|| v0.get("vmId"))
                    .or_else(|| v0.get("id"))
                    .and_then(Value::as_str)
            })
        })
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);

    let endpoint_url = sess_data
        .get("endpointUrl")
        .or_else(|| sess_data.get("endpoint_url"))
        .and_then(Value::as_str)
        .or_else(|| {
            first_vm.and_then(|v0| {
                v0.get("endpointUrl")
                    .or_else(|| v0.get("endpoint_url"))
                    .and_then(Value::as_str)
            })
        })
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| vm_id.as_ref().map(|id| format!("wss://{id}.metaaivm.com/")));

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

    (vm_id, endpoint_url, discovered_email)
}

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
        .header(ORIGIN, base)
        .header(REFERER, format!("{base}/"))
        .send()
        .await;

    if let Ok(resp) = session_resp {
        let status = resp.status().as_u16();
        current_cookie = merge_set_cookies(&current_cookie, resp.headers());
        if status == 401 {
            return Err((
                UpstreamFailureKind::AuthInvalid,
                format!("Muse session unauthorized (HTTP {status})"),
            ));
        }
        if resp.status().is_success()
            && let Ok(sess_data) = resp.json::<Value>().await
        {
            let (vm_id, endpoint_url, discovered_email) = parse_session_json(&sess_data);

            if let (Some(vm_id), Some(endpoint_url)) = (vm_id.clone(), endpoint_url) {
                for t_attempt in 0..2 {
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
                    if t_attempt == 0 && !cfg!(test) {
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
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
    if page_status == 401 {
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
    let unescaped = html.replace("\\\"", "\"").replace("\\/", "/");
    let auth_token = extract_json_str_field(&unescaped, "authToken")
        .or_else(|| extract_json_str_field(&unescaped, "auth_token"))
        .filter(|s| s != "null")?;
    let vm_id = extract_json_str_field(&unescaped, "vmName")
        .or_else(|| extract_json_str_field(&unescaped, "vmId"))
        .or_else(|| extract_json_str_field(&unescaped, "vm_id"))
        .or_else(|| {
            let gw = extract_json_str_field(&unescaped, "gatewayUrl")?;
            let parsed = url::Url::parse(&gw).ok()?;
            parsed
                .host_str()?
                .split('.')
                .next()
                .filter(|s| !s.is_empty())
                .map(ToOwned::to_owned)
        })?;
    let notary_token = extract_json_str_field(&unescaped, "noiseNotaryToken")
        .or_else(|| extract_json_str_field(&unescaped, "notary_token"))
        .filter(|s| s != "null")
        .unwrap_or_default();
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
