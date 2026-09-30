use crate::config::{
    SecretString,
    defaults::{
        default_chillaudio_app_key, default_chillaudio_timeout_ms, default_chillaudio_voice,
        default_chillaudio_ws_url,
    },
};
use serde::Deserialize;
use url::Url;
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChillAudioWsConfig {
    #[serde(default = "default_chillaudio_ws_url")]
    pub ws_url: Url,
    #[serde(default = "default_chillaudio_app_key")]
    pub app_key: SecretString,
    #[serde(default)]
    pub token: SecretString,
    #[serde(default = "default_chillaudio_voice")]
    pub voice: String,
    #[serde(default = "default_chillaudio_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub preload: bool,
}
