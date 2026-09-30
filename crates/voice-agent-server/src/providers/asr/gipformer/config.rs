use crate::config::defaults::{
    default_gipformer_decoding_method, default_gipformer_max_active_paths,
    default_gipformer_threads, default_vietnamese_language,
};
use serde::Deserialize;
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GipformerSherpaOfflineConfig {
    pub model: String,
    #[serde(default = "default_vietnamese_language")]
    pub language: String,
    #[serde(default = "default_gipformer_threads")]
    pub num_threads: i32,
    #[serde(default = "default_gipformer_decoding_method")]
    pub decoding_method: String,
    #[serde(default = "default_gipformer_max_active_paths")]
    pub max_active_paths: i32,
}
impl Default for GipformerSherpaOfflineConfig {
    fn default() -> Self {
        Self {
            model: String::new(),
            language: default_vietnamese_language(),
            num_threads: default_gipformer_threads(),
            decoding_method: default_gipformer_decoding_method(),
            max_active_paths: default_gipformer_max_active_paths(),
        }
    }
}
