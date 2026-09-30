use crate::config::{
    SecretString,
    defaults::{default_llm_timeout_ms, default_openai_base_url, default_openai_model},
};
use serde::Deserialize;
use url::Url;
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenAiConfig {
    #[serde(default)]
    pub api_key: SecretString,
    #[serde(default = "default_openai_base_url")]
    pub base_url: Url,
    #[serde(default = "default_openai_model")]
    pub model: String,
    #[serde(default = "default_llm_timeout_ms")]
    pub timeout_ms: u64,
}
impl Default for OpenAiConfig {
    fn default() -> Self {
        Self {
            api_key: SecretString(String::new()),
            base_url: default_openai_base_url(),
            model: default_openai_model(),
            timeout_ms: default_llm_timeout_ms(),
        }
    }
}
