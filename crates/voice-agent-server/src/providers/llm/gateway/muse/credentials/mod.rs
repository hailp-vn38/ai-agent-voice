mod bootstrap;

use std::collections::BTreeMap;
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use reqwest::header::{HeaderMap, SET_COOKIE};
use serde_json::Value;
use uuid::Uuid;

pub(crate) use self::bootstrap::{
    fetch_wss_from_cookie_http, parse_session_json, request_hatch_token_with_retry,
};
use super::config::{MuseAccountCredential, format_cookie_header, parse_cookie_header};

pub(crate) const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36";

/// Minimum safety buffer (5 minutes) before a Hatch VM WebSocket URL is considered expiring.
pub(crate) const WSS_EXPIRY_BUFFER_SECS: u64 = 300;

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ParsedMuseCredential {
    pub wss_url: Option<String>,
    pub cookie_header: Option<String>,
    pub gateway_init_json: Option<String>,
}

impl fmt::Debug for ParsedMuseCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ParsedMuseCredential")
            .field("wss_url", &self.wss_url.as_ref().map(|_| "[REDACTED]"))
            .field(
                "cookie_header",
                &self.cookie_header.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "gateway_init_json",
                &self.gateway_init_json.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

#[derive(Clone)]
pub(crate) struct BootstrappedMuseSession {
    pub wss_url: String,
    pub cookie_header: String,
    pub vm_id: Option<String>,
    pub discovered_email: Option<String>,
}

impl BootstrappedMuseSession {
    pub(crate) fn apply_to(
        self,
        cred: &mut MuseAccountCredential,
        discovered_email: &mut Option<String>,
    ) -> String {
        for (k, v) in parse_cookie_header(&self.cookie_header) {
            cred.cookies.insert(k, v);
        }
        if self.vm_id.is_some() {
            cred.vm_id = self.vm_id;
        }
        if discovered_email.is_none() {
            *discovered_email = self.discovered_email;
        }
        cred.wss_expires = get_wss_expiry_unix_secs(&self.wss_url);
        cred.wss_url = Some(self.wss_url.clone());
        self.wss_url
    }
}

impl std::fmt::Debug for BootstrappedMuseSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BootstrappedMuseSession")
            .field("wss_url", &"[REDACTED]")
            .field("cookie_header", &"[REDACTED]")
            .field("vm_id", &self.vm_id)
            .field("discovered_email", &self.discovered_email)
            .finish()
    }
}

impl ParsedMuseCredential {
    pub fn parse(raw: &str) -> Result<Self, String> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err("Muse credential cannot be empty".to_string());
        }

        // 1. Direct __HATCH_GATEWAY_INIT__ JSON object: {"vmName":"...","authToken":"...","noiseNotaryToken":"..."}
        if trimmed.starts_with('{')
            && let Ok(val) = serde_json::from_str::<Value>(trimmed)
            && (val.get("authToken").is_some() || val.get("token").is_some())
            && (val.get("vmName").is_some()
                || val.get("vmId").is_some()
                || val.get("vm_id").is_some())
        {
            let wss = build_wss_url_from_init_json(&val)?;
            return Ok(Self {
                wss_url: Some(wss),
                cookie_header: None,
                gateway_init_json: Some(trimmed.to_string()),
            });
        }

        // 2. Hybrid ("wss://...\n<cookies>"), Direct WSS, or Cookie header/JSON array
        let mut wss_url = None;
        let mut cookie_lines = Vec::new();
        for line in trimmed.lines() {
            let l = line.trim();
            if l.is_empty() {
                continue;
            }
            if wss_url.is_none() && (l.starts_with("wss://") || l.starts_with("ws://")) {
                wss_url = Some(l.to_string());
            } else {
                cookie_lines.push(l);
            }
        }

        let cookie_raw = cookie_lines.join("\n");
        let cookie_header = if cookie_raw.is_empty() {
            None
        } else {
            Some(normalize_cookie_header(&cookie_raw)?)
        };

        if wss_url.is_none() && cookie_header.is_none() {
            return Err(
                "Invalid Muse credential: expected wss:// URL, __HATCH_GATEWAY_INIT__ JSON, or cookie containing hatch_sess"
                    .to_string(),
            );
        }

        Ok(Self {
            wss_url,
            cookie_header,
            gateway_init_json: None,
        })
    }

    #[cfg(test)]
    pub fn to_hybrid_string(&self) -> String {
        match (&self.wss_url, &self.cookie_header) {
            (Some(w), Some(c)) => format!("{w}\n{c}"),
            (Some(w), None) => w.clone(),
            (None, Some(c)) => c.clone(),
            (None, None) => String::new(),
        }
    }
}

pub(crate) fn build_wss_url(vm_id: &str, auth_token: &str, notary_token: &str) -> String {
    let req_id = Uuid::new_v4().to_string();
    let mut url = url::Url::parse("wss://hatch.metaaivm.com/v1/noise")
        .expect("static wss://hatch.metaaivm.com/v1/noise URL is valid");
    url.query_pairs_mut()
        .append_pair("vm_id", vm_id)
        .append_pair("auth_token", auth_token)
        .append_pair("app_id", "hatch-web")
        .append_pair("request_id", &req_id)
        .append_pair("notary_token", notary_token);
    url.into()
}

pub(crate) fn build_wss_url_from_init_json(val: &Value) -> Result<String, String> {
    let vm_id = val
        .get("vmName")
        .or_else(|| val.get("vmId"))
        .or_else(|| val.get("vm_id"))
        .and_then(Value::as_str)
        .ok_or_else(|| "missing vmName/vmId in gateway init JSON".to_string())?;
    let auth_token = val
        .get("authToken")
        .or_else(|| val.get("token"))
        .and_then(Value::as_str)
        .ok_or_else(|| "missing authToken in gateway init JSON".to_string())?;
    let notary_token = val
        .get("noiseNotaryToken")
        .or_else(|| val.get("notaryToken"))
        .or_else(|| val.get("notary_token"))
        .and_then(Value::as_str)
        .unwrap_or("");
    Ok(build_wss_url(vm_id, auth_token, notary_token))
}

pub(crate) fn extract_vm_id_from_wss_url(wss_url: &str) -> Option<String> {
    let parsed = url::Url::parse(wss_url).ok()?;
    for (k, v) in parsed.query_pairs() {
        if k == "vm_id" && !v.trim().is_empty() {
            return Some(v.trim().to_string());
        }
    }
    None
}

/// Normalizes a cookie input (Cookie-Editor JSON array, object map, or `k=v; k2=v2` header)
/// and requires at least `hatch_sess`.
pub(crate) fn normalize_cookie_header(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    let mut map = BTreeMap::new();

    if trimmed.starts_with('[') {
        let items: Vec<Value> = serde_json::from_str(trimmed)
            .map_err(|e| format!("invalid cookie JSON array: {e}"))?;
        for item in items {
            if let (Some(name), Some(val)) = (
                item.get("name").and_then(Value::as_str),
                item.get("value").and_then(Value::as_str),
            ) && !name.trim().is_empty()
                && !val.trim().is_empty()
            {
                map.insert(name.trim().to_string(), val.trim().to_string());
            }
        }
    } else if trimmed.starts_with('{') {
        let obj: Value = serde_json::from_str(trimmed)
            .map_err(|e| format!("invalid cookie JSON object: {e}"))?;
        let target = obj.get("cookies").unwrap_or(&obj);
        if let Some(m) = target.as_object() {
            for (k, v) in m {
                if let Some(s) = v.as_str()
                    && !k.trim().is_empty()
                    && !s.trim().is_empty()
                {
                    map.insert(k.trim().to_string(), s.trim().to_string());
                }
            }
        }
    } else {
        map = parse_cookie_header(trimmed);
    }

    if !map.contains_key("hatch_sess") {
        return Err("Muse cookie must contain at least `hatch_sess`".to_string());
    }

    Ok(format_cookie_header(&map))
}

/// Merges `Set-Cookie` response headers into an existing cookie header while ignoring deletion cookies (`Max-Age=0`).
pub(crate) fn merge_set_cookies(existing_cookie_header: &str, headers: &HeaderMap) -> String {
    let mut map = parse_cookie_header(existing_cookie_header);
    for val in headers.get_all(SET_COOKIE) {
        if let Ok(s) = val.to_str() {
            let lower = s.to_ascii_lowercase();
            if lower.contains("max-age=0") {
                continue;
            }
            if let Some(first_pair) = s.split(';').next()
                && let Some((k, v)) = first_pair.trim().split_once('=')
            {
                let k = k.trim();
                let v = v.trim();
                if !k.is_empty() && !v.is_empty() {
                    map.insert(k.to_string(), v.to_string());
                }
            }
        }
    }
    format_cookie_header(&map)
}

fn extract_timeout_unix_secs(text: &str) -> Option<u64> {
    let lower = text.to_ascii_lowercase();
    let idx = lower.find("timeout:")?;
    let digits: String = text[idx + 8..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if (9..=11).contains(&digits.len()) {
        digits.parse::<u64>().ok()
    } else {
        None
    }
}

fn strip_hatch_jwt_prefix(raw: &str) -> &str {
    if let Some(rest) = raw.strip_prefix('s')
        && let Some((digits, jwt)) = rest.split_once(':')
        && !digits.is_empty()
        && digits.chars().all(|c| c.is_ascii_digit())
    {
        return jwt;
    }
    raw
}

/// Extracts the earliest expiry Unix timestamp (in seconds) from `notary_token` and `auth_token` query params.
pub(crate) fn get_wss_expiry_unix_secs(wss_url: &str) -> Option<u64> {
    let parsed = url::Url::parse(wss_url).ok()?;
    let mut min_exp: Option<u64> = None;

    for (k, v) in parsed.query_pairs() {
        if k == "notary_token" {
            if let Some(ts) = extract_timeout_unix_secs(&v) {
                min_exp = Some(min_exp.map_or(ts, |m| m.min(ts)));
            } else {
                for seg in v.split('.') {
                    if let Ok(bytes) = base64::engine::general_purpose::URL_SAFE_NO_PAD
                        .decode(seg)
                        .or_else(|_| base64::engine::general_purpose::STANDARD.decode(seg))
                    {
                        let text = String::from_utf8_lossy(&bytes);
                        if let Some(ts) = extract_timeout_unix_secs(&text) {
                            min_exp = Some(min_exp.map_or(ts, |m| m.min(ts)));
                        }
                    }
                }
            }
        } else if k == "auth_token" {
            let clean_jwt = strip_hatch_jwt_prefix(&v);
            let parts: Vec<&str> = clean_jwt.split('.').collect();
            if parts.len() == 3
                && let Ok(bytes) = base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .decode(parts[1])
                    .or_else(|_| base64::engine::general_purpose::STANDARD.decode(parts[1]))
                && let Ok(payload) = serde_json::from_slice::<Value>(&bytes)
            {
                if let Some(exp) = payload.get("exp").and_then(Value::as_u64) {
                    min_exp = Some(min_exp.map_or(exp, |m| m.min(exp)));
                } else if let Some(iat) = payload.get("iat").and_then(Value::as_u64) {
                    let exp = iat + 72_000;
                    min_exp = Some(min_exp.map_or(exp, |m| m.min(exp)));
                }
            }
        }
    }
    min_exp
}

pub(crate) fn is_wss_valid(wss_url: &str, buffer_secs: u64) -> bool {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    match get_wss_expiry_unix_secs(wss_url) {
        Some(exp) => exp > now.saturating_add(buffer_secs),
        None => true,
    }
}
