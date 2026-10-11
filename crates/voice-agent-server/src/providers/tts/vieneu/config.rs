use crate::config::SecretString;
use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VieneuHttpConfig {
    pub base_url: Url,
    #[serde(default, skip_serializing)]
    pub api_key: SecretString,
    pub voice: String,
    #[serde(default = "crate::config::defaults::default_vietnamese_language")]
    pub language: String,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub preload: bool,
}

fn default_timeout() -> u64 {
    30_000
}

impl VieneuHttpConfig {
    pub(crate) fn valid(&self) -> bool {
        matches!(self.base_url.scheme(), "http" | "https")
            && self.base_url.host_str().is_some()
            && self.base_url.username().is_empty()
            && self.base_url.password().is_none()
            && self.base_url.query().is_none()
            && self.base_url.fragment().is_none()
            && self.base_url.as_str().len() <= 2048
            && !self.voice.trim().is_empty()
            && self.voice.len() <= 128
            && !self.voice.chars().any(char::is_control)
            && self.language == "vi-VN"
            && (1..=120_000).contains(&self.timeout_ms)
    }
}

impl std::fmt::Debug for VieneuHttpConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VieneuHttpConfig")
            .field("base_url", &"[REDACTED]")
            .field("api_key", &self.api_key)
            .field("voice", &self.voice)
            .field("language", &self.language)
            .field("timeout_ms", &self.timeout_ms)
            .field("preload", &self.preload)
            .finish()
    }
}
