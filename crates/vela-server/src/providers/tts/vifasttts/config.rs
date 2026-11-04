use crate::config::defaults::{
    default_asr_threads, default_vietnamese_language, default_vifasttts_model,
    default_vifasttts_speed_percent, default_vifasttts_voice,
};
use serde::{Deserialize, Serialize};

use super::assets::VOICES;

pub const MODEL_ID: &str = "vifasttts_ngochuyen_small";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VifastttsOnnxConfig {
    #[serde(default = "default_vifasttts_model")]
    #[serde(skip_serializing)]
    pub model: String,
    #[serde(default = "default_asr_threads")]
    #[serde(skip_serializing)]
    pub num_threads: i32,
    #[serde(default = "default_vifasttts_voice")]
    pub voice: String,
    #[serde(default = "default_vietnamese_language")]
    pub language: String,
    #[serde(default = "default_vifasttts_speed_percent")]
    pub speed_percent: u16,
    #[serde(default)]
    pub preload: bool,
}

impl Default for VifastttsOnnxConfig {
    fn default() -> Self {
        Self {
            model: default_vifasttts_model(),
            num_threads: default_asr_threads(),
            voice: default_vifasttts_voice(),
            language: default_vietnamese_language(),
            speed_percent: default_vifasttts_speed_percent(),
            preload: false,
        }
    }
}

impl VifastttsOnnxConfig {
    pub fn valid_selection(&self) -> bool {
        self.model == MODEL_ID
            && self.language == "vi-VN"
            && (1..=128).contains(&self.num_threads)
            && (50..=200).contains(&self.speed_percent)
            && VOICES.iter().any(|voice| voice.id == self.voice)
    }
}
