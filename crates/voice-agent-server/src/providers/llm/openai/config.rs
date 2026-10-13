use crate::config::{
    SecretString,
    defaults::{default_llm_timeout_ms, default_openai_base_url, default_openai_model},
};
use serde::{Deserialize, Serialize};
use url::Url;
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OpenAiConfig {
    #[serde(default)]
    #[serde(skip_serializing)]
    pub api_key: SecretString,
    #[serde(default = "default_openai_base_url")]
    pub base_url: Url,
    #[serde(default = "default_openai_model")]
    pub model: String,
    #[serde(default = "default_llm_timeout_ms")]
    pub timeout_ms: u64,
    /// Explicit provider capability; Chat Completions stays the Voice default.
    #[serde(default, skip_serializing_if = "is_false")]
    pub supports_responses: bool,
}
fn is_false(value: &bool) -> bool { !*value }

impl Default for OpenAiConfig {
    fn default() -> Self {
        Self {
            api_key: SecretString(String::new()),
            base_url: default_openai_base_url(),
            model: default_openai_model(),
            timeout_ms: default_llm_timeout_ms(),
            supports_responses: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn responses_is_explicit_opt_in_and_does_not_change_default_voice_transport() {
        let default:OpenAiConfig=toml::from_str("").unwrap();
        assert!(!default.supports_responses);
        assert!(serde_json::to_value(&default).unwrap().get("supports_responses").is_none());
        let enabled:OpenAiConfig=toml::from_str("supports_responses = true").unwrap();
        assert!(enabled.supports_responses);
        assert_eq!(enabled.base_url.as_str(),"https://api.openai.com/v1");
    }
}
