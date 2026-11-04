use crate::config::defaults::{
    default_sensevoice_itn, default_sensevoice_language, default_sensevoice_model,
    default_sensevoice_threads,
};
use serde::{Deserialize, Serialize};

pub const SUPPORTED_LANGUAGES: &[&str] = &["auto", "en", "zh", "ja", "ko", "yue"];

pub fn valid_language(language: &str) -> bool {
    SUPPORTED_LANGUAGES.contains(&language)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SenseVoiceSherpaOfflineConfig {
    #[serde(default = "default_sensevoice_model", skip_serializing)]
    pub model: String,
    #[serde(default = "default_sensevoice_language")]
    pub language: String,
    #[serde(default = "default_sensevoice_itn")]
    pub use_itn: bool,
    #[serde(default = "default_sensevoice_threads", skip_serializing)]
    pub num_threads: i32,
}

impl Default for SenseVoiceSherpaOfflineConfig {
    fn default() -> Self {
        Self {
            model: default_sensevoice_model(),
            language: default_sensevoice_language(),
            use_itn: default_sensevoice_itn(),
            num_threads: default_sensevoice_threads(),
        }
    }
}
