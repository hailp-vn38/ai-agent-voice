use crate::config::defaults::{
    default_asr_threads, default_tts_model, default_tts_voice, default_vietnamese_language,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ZeroTtsDeliveryMode {
    File,
    #[default]
    Stream,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ZeroTtsOnnxConfig {
    #[serde(default = "default_tts_model")]
    #[serde(skip_serializing)]
    pub model: String,
    #[serde(default = "default_asr_threads")]
    #[serde(skip_serializing)]
    pub num_threads: i32,
    #[serde(default = "default_tts_voice")]
    pub voice: String,
    #[serde(default = "default_vietnamese_language")]
    pub language: String,
    #[serde(default)]
    pub delivery_mode: ZeroTtsDeliveryMode,
    #[serde(default)]
    pub preload: bool,
}
impl Default for ZeroTtsOnnxConfig {
    fn default() -> Self {
        Self {
            model: default_tts_model(),
            num_threads: default_asr_threads(),
            voice: default_tts_voice(),
            language: default_vietnamese_language(),
            delivery_mode: ZeroTtsDeliveryMode::Stream,
            preload: false,
        }
    }
}
