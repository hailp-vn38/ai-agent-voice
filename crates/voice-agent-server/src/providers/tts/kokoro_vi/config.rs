use crate::config::defaults::{
    default_asr_threads, default_kokoro_vi_model, default_kokoro_vi_speed_percent,
    default_kokoro_vi_voice, default_vietnamese_language,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
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

impl KokoroViOnnxConfig {
    pub(crate) fn valid_selection(&self) -> bool {
        self.model == "kokoro_vi_contextbox"
            && self.language == "vi-VN"
            && (1..=128).contains(&self.num_threads)
            && (50..=200).contains(&self.speed_percent)
            && KOKORO_VI_VOICES.contains(&self.voice.as_str())
    }
}

const KOKORO_VI_VOICES: &[&str] = &[
    "diem_trinh",
    "hung_thinh",
    "mai_linh",
    "mai_loan",
    "manh_dung",
    "my_yen",
    "ngoc_huyen",
    "phat_tai",
    "thanh_dat",
    "thuc_trinh",
    "tuan_ngoc",
    "storyvert",
    "duc_an",
    "duc_duy",
];
