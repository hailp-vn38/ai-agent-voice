use crate::config::{
    SecretString,
    defaults::{
        default_chillaudio_app_key, default_chillaudio_timeout_ms, default_chillaudio_voice,
        default_chillaudio_ws_url,
    },
};
use serde::{Deserialize, Serialize};
use url::Url;
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ChillAudioWsConfig {
    #[serde(default = "default_chillaudio_ws_url")]
    #[serde(skip_serializing)]
    pub ws_url: Url,
    #[serde(default = "default_chillaudio_app_key")]
    #[serde(skip_serializing)]
    pub app_key: SecretString,
    #[serde(default)]
    #[serde(skip_serializing)]
    pub token: SecretString,
    #[serde(default = "default_chillaudio_voice")]
    pub voice: String,
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default = "default_chillaudio_timeout_ms")]
    #[serde(skip_serializing)]
    pub timeout_ms: u64,
    #[serde(default)]
    pub preload: bool,
}

fn default_language() -> String {
    "vi".into()
}

impl std::fmt::Debug for ChillAudioWsConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChillAudioWsConfig")
            .field("ws_url", &"[REDACTED]")
            .field("app_key", &self.app_key)
            .field("token", &self.token)
            .field("voice", &self.voice)
            .field("language", &self.language)
            .field("timeout_ms", &self.timeout_ms)
            .field("preload", &self.preload)
            .finish()
    }
}
