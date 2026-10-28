use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::config::SecretString;
use crate::providers::llm::{valid_generation_bounds, valid_llm_tags};

pub const ADAPTER_NAME: &str = "muse";
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

pub const MUSE_MODELS: &[&str] = &["muse-chat", "muse-image", "muse-video"];

/// Structured credential for each Muse account parsed from `api_key`.
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct MuseAccountCredential {
    #[serde(default = "default_account_label", alias = "id")]
    pub label: String,
    pub cookies: BTreeMap<String, String>,
    #[serde(default)]
    pub cookie_expires: Option<u64>,
    #[serde(default)]
    pub vm_id: Option<String>,
}

impl std::fmt::Debug for MuseAccountCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MuseAccountCredential")
            .field("label", &self.label)
            .field("cookies", &"[REDACTED]")
            .field("cookie_expires", &self.cookie_expires)
            .field("vm_id", &self.vm_id)
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

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MuseConfig {
    /// muse.ai session cookie credential loaded via standard SecretRef.
    /// Accepts JSON `MuseAccountCredential` (single object or array of accounts)
    /// or a Cookie header string `"hatch_sess=...; hatch_gw=...; hatch_vml=...; hatch_native_auth_device=..."`.
    #[serde(default, skip_serializing)]
    pub api_key: SecretString,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_limit_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature_percent: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    /// Maximum concurrent tabs per account (1..=8, default: 2).
    #[serde(default = "default_account_concurrency")]
    pub account_concurrency: usize,
    /// Local Chromium CDP port (1024..=65535, default: 19222).
    #[serde(default = "default_cdp_port")]
    pub cdp_port: u16,
    /// Run Chromium in headless mode (`--headless=new`, default: true).
    #[serde(default = "default_headless")]
    pub headless: bool,
}

impl MuseConfig {
    pub fn valid(&self) -> bool {
        let model = self.model.trim();
        let raw_key = self.api_key.expose().trim();
        let key_valid = raw_key.is_empty() || !parse_accounts(raw_key).is_empty();
        !model.is_empty()
            && model.len() <= 256
            && (1_000..=600_000).contains(&self.timeout_ms)
            && (1..=8).contains(&self.account_concurrency)
            && self.cdp_port >= 1024
            && valid_generation_bounds(
                self.max_tokens,
                self.context_limit_tokens,
                self.reasoning_effort.as_deref(),
                self.temperature_percent,
            )
            && valid_llm_tags(self.tags.as_deref())
            && key_valid
    }

    pub fn parse_accounts(&self) -> Vec<MuseAccountCredential> {
        parse_accounts(self.api_key.expose())
    }
}

pub fn has_required_cookies(cookies: &BTreeMap<String, String>) -> bool {
    REQUIRED_COOKIES
        .iter()
        .all(|name| cookies.get(*name).is_some_and(|v| !v.trim().is_empty()))
}

#[derive(Deserialize)]
struct BrowserCookieEntry {
    name: String,
    value: String,
    #[serde(default, rename = "expirationDate")]
    expiration_date: Option<f64>,
}

#[derive(Deserialize)]
struct BrowserCookieExport {
    #[serde(default)]
    label: Option<String>,
    cookies: Vec<BrowserCookieEntry>,
}

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
        if !name.is_empty() && !value.is_empty() {
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
        })
    } else {
        None
    }
}

pub fn parse_accounts(raw: &str) -> Vec<MuseAccountCredential> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    if trimmed.starts_with('[') {
        if let Ok(accounts) = serde_json::from_str::<Vec<MuseAccountCredential>>(trimmed) {
            let valid: Vec<MuseAccountCredential> = accounts
                .into_iter()
                .enumerate()
                .filter(|(_, acc)| has_required_cookies(&acc.cookies))
                .map(|(idx, mut acc)| {
                    acc.label = sanitize_account_label(&acc.label, idx);
                    acc
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
        if let Ok(mut acc) = serde_json::from_str::<MuseAccountCredential>(trimmed) {
            if has_required_cookies(&acc.cookies) {
                acc.label = sanitize_account_label(&acc.label, 0);
                return vec![acc];
            }
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
        .filter_map(|(idx, cookie_str)| {
            let cookies = parse_cookie_header(cookie_str);
            if has_required_cookies(&cookies) {
                Some(MuseAccountCredential {
                    label: format!("account-{idx}"),
                    cookies,
                    cookie_expires: None,
                    vm_id: None,
                })
            } else {
                None
            }
        })
        .collect()
}

pub fn parse_cookie_header(raw: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for pair in raw.split(';') {
        let pair = pair.trim();
        if let Some((k, v)) = pair.split_once('=') {
            let k = k.trim();
            let v = v.trim();
            if !k.is_empty() && !v.is_empty() {
                map.insert(k.to_string(), v.to_string());
            }
        }
    }
    map
}

fn default_model() -> String {
    "muse-chat".to_string()
}

const fn default_timeout_ms() -> u64 {
    300_000
}

const fn default_account_concurrency() -> usize {
    2
}

const fn default_cdp_port() -> u16 {
    19222
}

const fn default_headless() -> bool {
    true
}
