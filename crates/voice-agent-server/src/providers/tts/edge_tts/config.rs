use serde::{Deserialize, Serialize};
use url::Url;

use crate::config::defaults::{
    default_edge_tts_sec_ms_gec_version, default_edge_tts_timeout_ms,
    default_edge_tts_trusted_client_token, default_edge_tts_voice, default_edge_tts_ws_url,
};

pub const MIN_RATE_PERCENT: i16 = -50;
pub const MAX_RATE_PERCENT: i16 = 100;
pub const MIN_PITCH_HZ: i16 = -50;
pub const MAX_PITCH_HZ: i16 = 50;

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(from = "RawEdgeTtsConfig")]
pub struct EdgeTtsConfig {
    #[serde(skip_serializing)]
    pub ws_url: Url,
    #[serde(skip_serializing)]
    pub trusted_client_token: String,
    #[serde(skip_serializing)]
    pub sec_ms_gec_version: String,
    #[serde(skip_serializing)]
    pub timeout_ms: u64,
    pub voice: String,
    pub language: String,
    pub rate_percent: i16,
    pub pitch_hz: i16,
    pub preload: bool,
}

impl EdgeTtsConfig {
    pub fn valid_selection(&self) -> bool {
        super::voices::is_valid_voice_and_language(&self.voice, &self.language)
            && (MIN_RATE_PERCENT..=MAX_RATE_PERCENT).contains(&self.rate_percent)
            && (MIN_PITCH_HZ..=MAX_PITCH_HZ).contains(&self.pitch_hz)
    }
}

impl std::fmt::Debug for EdgeTtsConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EdgeTtsConfig")
            .field("ws_url", &self.ws_url.as_str())
            .field("trusted_client_token", &"[REDACTED]")
            .field("sec_ms_gec_version", &self.sec_ms_gec_version)
            .field("timeout_ms", &self.timeout_ms)
            .field("voice", &self.voice)
            .field("language", &self.language)
            .field("rate_percent", &self.rate_percent)
            .field("pitch_hz", &self.pitch_hz)
            .field("preload", &self.preload)
            .finish()
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEdgeTtsConfig {
    #[serde(default = "default_edge_tts_ws_url")]
    ws_url: Url,
    #[serde(default = "default_edge_tts_trusted_client_token")]
    trusted_client_token: String,
    #[serde(default = "default_edge_tts_sec_ms_gec_version")]
    sec_ms_gec_version: String,
    #[serde(default = "default_edge_tts_timeout_ms")]
    timeout_ms: u64,
    #[serde(default = "default_edge_tts_voice")]
    voice: String,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    rate_percent: i16,
    #[serde(default)]
    pitch_hz: i16,
    #[serde(default)]
    preload: bool,
}

impl From<RawEdgeTtsConfig> for EdgeTtsConfig {
    fn from(raw: RawEdgeTtsConfig) -> Self {
        let language = raw.language.unwrap_or_else(|| {
            super::voices::voice_locale(&raw.voice)
                .unwrap_or("vi-VN")
                .to_string()
        });
        Self {
            ws_url: raw.ws_url,
            trusted_client_token: raw.trusted_client_token,
            sec_ms_gec_version: raw.sec_ms_gec_version,
            timeout_ms: raw.timeout_ms,
            voice: raw.voice,
            language,
            rate_percent: raw.rate_percent,
            pitch_hz: raw.pitch_hz,
            preload: raw.preload,
        }
    }
}
