use crate::config::TransducerDecodingMethod;
use crate::config::defaults::{default_asr_model, default_asr_threads, default_decoding_method};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ZipformerSherpaConfig {
    #[serde(default = "default_asr_model")]
    #[serde(skip_serializing)]
    pub model: String,
    #[serde(default = "default_asr_threads")]
    #[serde(skip_serializing)]
    pub num_threads: i32,
    #[serde(default = "default_decoding_method")]
    pub decoding_method: TransducerDecodingMethod,
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
