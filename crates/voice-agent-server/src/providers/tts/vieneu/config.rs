use serde::{Deserialize, Serialize};

use super::assets::{DEFAULT_VOICE, VOICES};

/// The desired (non-secret) configuration. Model identity and ORT threads are
/// injected from the deployment runtime and are never user-editable fields.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VieneuNativeConfig {
    #[serde(default = "default_model")]
    #[serde(skip_serializing)]
    pub model: String,
    #[serde(default = "default_threads")]
    #[serde(skip_serializing)]
    pub num_threads: i32,
    #[serde(default = "default_voice")]
    pub voice: String,
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default)]
    pub preload: bool,
}

pub const MODEL_ID: &str = "vieneu_v3_turbo_fp32";

fn default_model() -> String {
    MODEL_ID.into()
}
fn default_threads() -> i32 {
    1
}
fn default_voice() -> String {
    DEFAULT_VOICE.into()
}
fn default_language() -> String {
    "vi-VN".into()
}

impl Default for VieneuNativeConfig {
    fn default() -> Self {
        Self {
            model: default_model(),
            num_threads: default_threads(),
            voice: default_voice(),
            language: default_language(),
            preload: false,
        }
    }
}

impl VieneuNativeConfig {
    pub fn valid_selection(&self) -> bool {
        self.model == MODEL_ID
            && self.language == "vi-VN"
            && (1..=128).contains(&self.num_threads)
            && VOICES.iter().any(|voice| voice.id == self.voice)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_selection_is_in_local_voice_catalog() {
        let config = VieneuNativeConfig::default();
        assert!(config.valid_selection());
        assert_eq!(config.model, MODEL_ID);
        assert_eq!(config.language, "vi-VN");
    }

    #[test]
    fn rejects_unknown_options_and_credentials() {
        for invalid in [
            r#"{"api_key":"secret"}"#,
            r#"{"model_path":"/tmp/other.onnx"}"#,
            r#"{"precision":"int8"}"#,
            r#"{"base_url":"http://localhost"}"#,
        ] {
            assert!(serde_json::from_str::<VieneuNativeConfig>(invalid).is_err());
        }
    }

    #[test]
    fn rejects_invalid_voice_language_threads_or_model() {
        let mut value = VieneuNativeConfig {
            voice: "not-in-catalog".into(),
            ..Default::default()
        };
        assert!(!value.valid_selection());
        value.voice = DEFAULT_VOICE.into();
        value.language = "en-US".into();
        assert!(!value.valid_selection());
        value.language = "vi-VN".into();
        value.num_threads = 0;
        assert!(!value.valid_selection());
        value.num_threads = 1;
        value.model = "other-model".into();
        assert!(!value.valid_selection());
    }

    #[test]
    fn hides_deployment_owned_fields_when_serializing() {
        let value = serde_json::to_value(VieneuNativeConfig::default()).unwrap();
        assert!(value.get("model").is_none());
        assert!(value.get("num_threads").is_none());
        assert_eq!(value["voice"], DEFAULT_VOICE);
    }
}
