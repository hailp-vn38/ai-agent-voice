use crate::config::defaults::{default_asr_model, default_asr_threads, default_decoding_method};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ZipformerSherpaConfig {
    #[serde(default = "default_asr_model")]
    pub model: String,
    #[serde(default = "default_asr_threads")]
    pub num_threads: i32,
    #[serde(default = "default_decoding_method")]
    pub decoding_method: String,
}
impl Default for ZipformerSherpaConfig {
    fn default() -> Self {
        Self {
            model: default_asr_model(),
            num_threads: default_asr_threads(),
            decoding_method: default_decoding_method(),
        }
    }
}
