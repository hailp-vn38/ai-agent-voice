use serde::{Deserialize, Serialize};

use super::assets::{DEFAULT_VOICE, NANO_DEFAULT_VOICE, voices_for_model};

pub const TURBO_MODEL_ID: &str = "vieneu_v3_turbo_fp32";
pub const NANO_MODEL_ID: &str = "vieneu_v3_nano_fp32";
pub const MODEL_ID: &str = TURBO_MODEL_ID;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub enum VieneuModel {
    #[default]
    #[serde(rename = "vieneu_v3_turbo_fp32")]
    V3TurboFp32,
    #[serde(rename = "vieneu_v3_nano_fp32")]
    V3NanoFp32,
}

impl VieneuModel {
    pub fn parse(id: &str) -> Option<Self> {
        match id {
            TURBO_MODEL_ID => Some(Self::V3TurboFp32),
            NANO_MODEL_ID => Some(Self::V3NanoFp32),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::V3TurboFp32 => TURBO_MODEL_ID,
            Self::V3NanoFp32 => NANO_MODEL_ID,
        }
    }

    pub const fn default_voice(self) -> &'static str {
        match self {
            Self::V3TurboFp32 => DEFAULT_VOICE,
            Self::V3NanoFp32 => NANO_DEFAULT_VOICE,
        }
    }

    pub const fn sample_rate_hz(self) -> u32 {
        match self {
            Self::V3TurboFp32 => 48_000,
            Self::V3NanoFp32 => 24_000,
        }
    }
}

/// The desired (non-secret) configuration. ORT threads are injected from the
/// deployment runtime and are never user-editable fields.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(from = "RawVieneuNativeConfig")]
pub struct VieneuNativeConfig {
    pub model: String,
    #[serde(skip_serializing)]
    pub num_threads: i32,
    pub voice: String,
    pub language: String,
    pub preload: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawVieneuNativeConfig {
    #[serde(default = "default_model")]
    model: String,
    #[serde(default = "default_threads")]
    num_threads: i32,
    #[serde(default)]
    voice: Option<String>,
    #[serde(default = "default_language")]
    language: String,
    #[serde(default)]
    preload: bool,
}

impl From<RawVieneuNativeConfig> for VieneuNativeConfig {
    fn from(raw: RawVieneuNativeConfig) -> Self {
        let voice = raw.voice.unwrap_or_else(|| {
            VieneuModel::parse(&raw.model)
                .map(VieneuModel::default_voice)
                .unwrap_or(DEFAULT_VOICE)
                .to_owned()
        });
        Self {
            model: raw.model,
            num_threads: raw.num_threads,
            voice,
            language: raw.language,
            preload: raw.preload,
        }
    }
}

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
    pub fn parsed_model(&self) -> Option<VieneuModel> {
        VieneuModel::parse(&self.model)
    }

    pub fn valid_selection(&self) -> bool {
        let Some(model) = self.parsed_model() else {
            return false;
        };
        self.language == "vi-VN"
            && (1..=128).contains(&self.num_threads)
            && voices_for_model(model)
                .iter()
                .any(|voice| voice.id == self.voice)
    }
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
