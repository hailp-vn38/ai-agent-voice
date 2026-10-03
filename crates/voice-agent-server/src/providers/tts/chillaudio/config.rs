use crate::config::{
    SecretString,
    defaults::{
        default_chillaudio_app_key, default_chillaudio_timeout_ms, default_chillaudio_voice,
        default_chillaudio_ws_url,
    },
};
use serde::{Deserialize, Serialize};
use url::Url;
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ChillAudioWsConfig {
    #[serde(default = "default_chillaudio_ws_url")]
    pub ws_url: Url,
    #[serde(default = "default_chillaudio_app_key")]
    #[serde(skip_serializing)]
    pub app_key: SecretString,
    #[serde(default)]
    #[serde(skip_serializing)]
    pub token: SecretString,
    #[serde(default = "default_chillaudio_voice")]
    pub voice: String,
    #[serde(default = "default_chillaudio_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub preload: bool,
}
