use crate::config::defaults::{
    default_asr_threads, default_kokoro_vi_model, default_kokoro_vi_speed_percent,
    default_kokoro_vi_voice, default_vietnamese_language,
};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KokoroViOnnxConfig {
    #[serde(default = "default_kokoro_vi_model")]
    pub model: String,
    #[serde(default = "default_asr_threads")]
    pub num_threads: i32,
    #[serde(default = "default_kokoro_vi_voice")]
    pub voice: String,
    #[serde(default = "default_vietnamese_language")]
    pub language: String,
    #[serde(default = "default_kokoro_vi_speed_percent")]
    pub speed_percent: u16,
    #[serde(default)]
    pub preload: bool,
}

impl Default for KokoroViOnnxConfig {
    fn default() -> Self {
        Self {
            model: default_kokoro_vi_model(),
            num_threads: default_asr_threads(),
            voice: default_kokoro_vi_voice(),
            language: default_vietnamese_language(),
            speed_percent: default_kokoro_vi_speed_percent(),
            preload: false,
        }
    }
}
