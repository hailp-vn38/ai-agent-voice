use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::credentials::{
    ParsedMuseCredential, extract_vm_id_from_wss_url, get_wss_expiry_unix_secs,
};

pub const SITE_URL: &str = "https://muse.ai";
pub const COOKIE_DOMAIN: &str = ".muse.ai";
pub const NEW_THREAD_URL: &str = "https://muse.ai/thread/new";
pub const SESSION_API_URL: &str = "https://muse.ai/api/session";
pub const VM_WAKE_API_URL: &str = "https://muse.ai/api/hatch/vm/wake";

pub const REQUIRED_COOKIES: &[&str] = &[
    "hatch_sess",
    "hatch_gw",
    "hatch_vml",
    "hatch_native_auth_device",
];

/// Structured credential for each Muse account parsed from `api_key` or OAuth token bundle.
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct MuseAccountCredential {
    #[serde(default = "default_account_label", alias = "id")]
    pub label: String,
    #[serde(default)]
    pub cookies: BTreeMap<String, String>,
    #[serde(default)]
    pub cookie_expires: Option<u64>,
    #[serde(default)]
    pub vm_id: Option<String>,
    #[serde(default)]
    pub wss_url: Option<String>,
    #[serde(default)]
    pub wss_expires: Option<u64>,
}

impl std::fmt::Debug for MuseAccountCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MuseAccountCredential")
            .field("label", &self.label)
            .field("cookies", &"[REDACTED]")
            .field("cookie_expires", &self.cookie_expires)
            .field("vm_id", &self.vm_id)
            .field("wss_url", &self.wss_url.as_ref().map(|_| "[REDACTED]"))
            .field("wss_expires", &self.wss_expires)
            .finish()
    }
}

fn default_account_label() -> String {
    "account-0".to_string()
}

fn sanitize_account_label(raw: &str, idx: usize) -> String {
    let cleaned: String = raw
        .trim()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .take(64)
        .collect();
    if cleaned.is_empty() {
        format!("account-{idx}")
    } else {
        cleaned
    }
}

/// Checks whether a cookie map has the minimum required session cookie (`hatch_sess`).
pub fn has_required_cookies(cookies: &BTreeMap<String, String>) -> bool {
    cookies
        .get("hatch_sess")
        .is_some_and(|v| !v.trim().is_empty())
}

/// Checks whether a [`MuseAccountCredential`] has either a valid `hatch_sess` cookie or a direct Hatch VM `wss://` URL.
pub fn has_valid_muse_credential(cred: &MuseAccountCredential) -> bool {
    has_required_cookies(&cred.cookies)
        || cred
            .wss_url
            .as_deref()
            .is_some_and(|u| u.starts_with("wss://") || u.starts_with("ws://"))
}

pub fn format_cookie_header(cookies: &BTreeMap<String, String>) -> String {
    cookies
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("; ")
}

#[derive(Deserialize)]
struct BrowserCookieEntry {
    name: String,
    value: String,
    #[serde(default)]
    #[allow(dead_code)]
    domain: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    path: Option<String>,
    #[serde(default, rename = "expirationDate")]
    expiration_date: Option<f64>,
    #[serde(default, rename = "hostOnly")]
    #[allow(dead_code)]
    host_only: Option<bool>,
    #[serde(default, rename = "httpOnly")]
    #[allow(dead_code)]
    http_only: Option<bool>,
    #[serde(default, rename = "sameSite")]
    #[allow(dead_code)]
    same_site: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    secure: Option<bool>,
    #[serde(default)]
    #[allow(dead_code)]
    session: Option<bool>,
    #[serde(default, rename = "storeId")]
    #[allow(dead_code)]
    store_id: Option<String>,
}

#[derive(Deserialize)]
struct BrowserCookieExport {
    #[serde(default)]
    #[allow(dead_code)]
    url: Option<String>,
    #[serde(default)]
    label: Option<String>,
    cookies: Vec<BrowserCookieEntry>,
}

fn clean_pasted_cookie_payload(raw: &str) -> &str {
    let mut s = raw.trim();
    if let Some(rest) = s.strip_prefix("```") {
        let rest = rest
            .strip_prefix("json")
            .or_else(|| rest.strip_prefix("JSON"))
            .unwrap_or(rest);
        s = rest.trim_start().strip_suffix("```").unwrap_or(rest).trim();
    }
    if !s.starts_with("wss://")
        && !s.starts_with("ws://")
        && let Some((lhs, rhs)) = s.split_once('=')
        && lhs.trim().eq_ignore_ascii_case("api_key")
    {
        s = rhs.trim();
    }
    if (s.starts_with('\'') && s.ends_with('\'') && s.len() >= 2)
        || (s.starts_with('"') && s.ends_with('"') && s.len() >= 2)
    {
        s = s[1..s.len() - 1].trim();
    }
    if let Some(rest) = s.strip_prefix("tok_muse_") {
        s = rest.trim();
    }
    s
}

const IGNORED_UI_COOKIES: &[&str] = &["layout_state", "theme", "dpr", "wd"];

fn account_from_browser_cookies(
    entries: Vec<BrowserCookieEntry>,
    label: Option<&str>,
    idx: usize,
) -> Option<MuseAccountCredential> {
    let mut cookies = BTreeMap::new();
    let mut min_expires: Option<u64> = None;
    for entry in entries {
        let name = entry.name.trim();
        let value = entry.value.trim();
        if !name.is_empty() && !value.is_empty() && !IGNORED_UI_COOKIES.contains(&name) {
            if REQUIRED_COOKIES.contains(&name)
                && let Some(exp) = entry.expiration_date
                && exp.is_finite()
                && exp > 0.0
            {
                let exp_secs = exp as u64;
                min_expires = Some(min_expires.map_or(exp_secs, |cur| cur.min(exp_secs)));
            }
            cookies.insert(name.to_string(), value.to_string());
        }
    }
    if has_required_cookies(&cookies) {
        Some(MuseAccountCredential {
            label: sanitize_account_label(label.unwrap_or(""), idx),
            cookies,
            cookie_expires: min_expires,
            vm_id: None,
            wss_url: None,
            wss_expires: None,
        })
    } else {
        None
    }
}

pub fn parse_accounts(raw: &str) -> Vec<MuseAccountCredential> {
    let trimmed = clean_pasted_cookie_payload(raw);
    if trimmed.is_empty() {
        return Vec::new();
    }
    if trimmed.starts_with('[') {
        if let Ok(accounts) = serde_json::from_str::<Vec<MuseAccountCredential>>(trimmed) {
            let valid: Vec<MuseAccountCredential> = accounts
                .into_iter()
                .enumerate()
                .filter(|(_, acc)| has_valid_muse_credential(acc))
                .map(|(idx, mut acc)| {
                    acc.label = sanitize_account_label(&acc.label, idx);
                    if acc.wss_expires.is_none() {
                        acc.wss_expires = acc
                            .wss_url
                            .as_deref()
                            .and_then(get_wss_expiry_unix_secs);
                    }
                    if acc.vm_id.is_none() {
                        acc.vm_id = acc
                            .wss_url
                            .as_deref()
                            .and_then(extract_vm_id_from_wss_url);
                    }
                    acc
                })
                .collect();
            if !valid.is_empty() {
                return valid;
            }
        }
        if let Ok(exports) = serde_json::from_str::<Vec<BrowserCookieExport>>(trimmed) {
            let valid: Vec<MuseAccountCredential> = exports
                .into_iter()
                .enumerate()
                .filter_map(|(idx, exp)| {
                    account_from_browser_cookies(exp.cookies, exp.label.as_deref(), idx)
                })
                .collect();
            if !valid.is_empty() {
                return valid;
            }
        }
        if let Ok(entries) = serde_json::from_str::<Vec<BrowserCookieEntry>>(trimmed)
            && let Some(acc) = account_from_browser_cookies(entries, None, 0)
        {
            return vec![acc];
        }
    }
    if trimmed.starts_with('{') {
        if let Ok(mut acc) = serde_json::from_str::<MuseAccountCredential>(trimmed)
            && has_valid_muse_credential(&acc)
        {
            acc.label = sanitize_account_label(&acc.label, 0);
            if acc.wss_expires.is_none() {
                acc.wss_expires = acc.wss_url.as_deref().and_then(get_wss_expiry_unix_secs);
            }
            if acc.vm_id.is_none() {
                acc.vm_id = acc.wss_url.as_deref().and_then(extract_vm_id_from_wss_url);
            }
            return vec![acc];
        }
        if let Ok(export) = serde_json::from_str::<BrowserCookieExport>(trimmed)
            && let Some(acc) =
                account_from_browser_cookies(export.cookies, export.label.as_deref(), 0)
        {
            return vec![acc];
        }
    }
    trimmed
        .split("||")
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .enumerate()
        .filter_map(|(idx, part)| {
            if let Ok(parsed) = ParsedMuseCredential::parse(part) {
                let cookies = parsed
                    .cookie_header
                    .as_deref()
                    .map(parse_cookie_header)
                    .unwrap_or_default();
                let wss_expires = parsed
                    .wss_url
                    .as_deref()
                    .and_then(get_wss_expiry_unix_secs);
                let vm_id = parsed
                    .wss_url
                    .as_deref()
                    .and_then(extract_vm_id_from_wss_url);
                let cred = MuseAccountCredential {
                    label: format!("account-{idx}"),
                    cookies,
                    cookie_expires: None,
                    vm_id,
                    wss_url: parsed.wss_url,
                    wss_expires,
                };
                if has_valid_muse_credential(&cred) {
                    return Some(cred);
                }
            }
            None
        })
        .collect()
}

pub fn parse_cookie_header(raw: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    let trimmed = raw.trim();
    let stripped = if trimmed.len() >= 7 && trimmed[..7].eq_ignore_ascii_case("cookie:") {
        trimmed[7..].trim()
    } else {
        trimmed
    };
    for pair in stripped.split(';') {
        let pair = pair.trim();
        if let Some((k, v)) = pair.split_once('=') {
            let k = k.trim();
            let v = v.trim();
            if !k.is_empty() && !v.is_empty() && !IGNORED_UI_COOKIES.contains(&k) {
                map.insert(k.to_string(), v.to_string());
            }
        }
    }
    map
}
