//! Credential-free, bounded provider desired-configuration validation.
use crate::config::{TransducerDecodingMethod, ZeroTtsDeliveryMode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

pub const MAX_PROVIDER_CONFIG_BYTES: usize = 64 * 1024;
const MAX_DEPTH: u8 = 16;
const MAX_NODES: u16 = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderConfigError {
    Invalid,
}

pub fn validate(adapter: &str, value: &Value) -> Result<String, ProviderConfigError> {
    let raw = serde_json::to_string(value).map_err(|_| ProviderConfigError::Invalid)?;
    validate_raw(adapter, &raw)
}

/// This is the startup seam: raw persisted text is bounded before parsing or allocating its tree.
pub fn validate_raw(adapter: &str, raw: &str) -> Result<String, ProviderConfigError> {
    if raw.len() > MAX_PROVIDER_CONFIG_BYTES {
        return Err(ProviderConfigError::Invalid);
    }
    let value: Value = serde_json::from_str(raw).map_err(|_| ProviderConfigError::Invalid)?;
    if !shape(&value, 0, &mut 0) || protected(&value) {
        return Err(ProviderConfigError::Invalid);
    }
    match adapter {
        "openai" => canonical::<OpenAi, _>(&value, OpenAi::valid),
        "silero_onnx" => canonical::<Silero, _>(&value, Silero::valid),
        "zipformer_sherpa" => canonical::<Zipformer, _>(&value, Zipformer::valid),
        "gipformer_sherpa_offline" => canonical::<Gipformer, _>(&value, Gipformer::valid),
        "zerotts_onnx" => canonical::<ZeroTts, _>(&value, ZeroTts::valid),
        "kokoro_vi_onnx" => canonical::<Kokoro, _>(&value, Kokoro::valid),
        "chillaudio_ws" => canonical::<ChillAudio, _>(&value, ChillAudio::valid),
        _ => Err(ProviderConfigError::Invalid),
    }
}
fn canonical<T, F>(value: &Value, valid: F) -> Result<String, ProviderConfigError>
where
    T: for<'de> Deserialize<'de> + Serialize,
    F: FnOnce(&T) -> bool,
{
    let typed: T =
        serde_json::from_value(value.clone()).map_err(|_| ProviderConfigError::Invalid)?;
    if !valid(&typed) {
        return Err(ProviderConfigError::Invalid);
    }
    serde_json::to_string(&typed).map_err(|_| ProviderConfigError::Invalid)
}
fn shape(value: &Value, depth: u8, nodes: &mut u16) -> bool {
    if depth > MAX_DEPTH {
        return false;
    }
    match value {
        Value::Object(values) => values.iter().all(|(_, v)| {
            *nodes = nodes.saturating_add(1);
            *nodes <= MAX_NODES && shape(v, depth + 1, nodes)
        }),
        Value::Array(values) => values.iter().all(|v| {
            *nodes = nodes.saturating_add(1);
            *nodes <= MAX_NODES && shape(v, depth + 1, nodes)
        }),
        _ => true,
    }
}
fn protected(value: &Value) -> bool {
    const DENY: &[&str] = &[
        "apikey",
        "token",
        "accesstoken",
        "refreshtoken",
        "bearertoken",
        "password",
        "passwd",
        "secret",
        "clientsecret",
        "authorization",
        "proxyauthorization",
        "credential",
        "credentials",
        "privatekey",
        "secretref",
    ];
    match value {
        Value::Object(values) => values.iter().any(|(key, value)| {
            let normalized: String = key
                .bytes()
                .filter(|b| !matches!(b, b'_' | b'-' | b' ' | b'\t' | b'\r' | b'\n'))
                .map(|b| b.to_ascii_lowercase() as char)
                .collect();
            DENY.contains(&normalized.as_str()) || protected(value)
        }),
        Value::Array(values) => values.iter().any(protected),
        _ => false,
    }
}
fn string(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct OpenAi {
    base_url: Url,
    model: String,
    #[serde(default = "default_timeout")]
    timeout_ms: u64,
    #[serde(default)]
    max_tokens: Option<u32>,
}
impl OpenAi {
    fn valid(&self) -> bool {
        self.base_url.scheme() == "https"
            && string(&self.model, 256)
            && (1..=120_000).contains(&self.timeout_ms)
            && self.max_tokens.is_none_or(|v| v > 0 && v <= 65_536)
    }
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Silero {}
impl Silero {
    fn valid(&self) -> bool {
        true
    }
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Zipformer {
    #[serde(default)]
    decoding_method: TransducerDecodingMethod,
}
impl Zipformer {
    fn valid(&self) -> bool {
        true
    }
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Gipformer {
    #[serde(default = "default_vietnamese_language")]
    language: String,
    #[serde(default = "default_decode")]
    decoding_method: TransducerDecodingMethod,
    #[serde(default = "default_paths")]
    max_active_paths: i32,
}
impl Gipformer {
    fn valid(&self) -> bool {
        self.language == "vi-VN" && (1..=10_000).contains(&self.max_active_paths)
    }
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ZeroTts {
    voice: String,
    #[serde(default = "default_vietnamese_language")]
    language: String,
    #[serde(default)]
    preload: bool,
    #[serde(default)]
    delivery_mode: ZeroTtsDeliveryMode,
}
impl ZeroTts {
    fn valid(&self) -> bool {
        crate::providers::tts::zerotts::descriptor::DESCRIPTOR
            .capabilities
            .voices
            .is_some_and(|voices| voices.iter().any(|voice| voice.id == self.voice))
            && self.language == "vi-VN"
    }
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Kokoro {
    voice: String,
    #[serde(default = "default_vietnamese_language")]
    language: String,
    #[serde(default = "crate::config::defaults::default_kokoro_vi_speed_percent")]
    speed_percent: u16,
    #[serde(default)]
    preload: bool,
}
impl Kokoro {
    fn valid(&self) -> bool {
        crate::providers::tts::kokoro_vi::descriptor::DESCRIPTOR
            .capabilities
            .voices
            .is_some_and(|voices| voices.iter().any(|voice| voice.id == self.voice))
            && self.language == "vi-VN"
            && (50..=200).contains(&self.speed_percent)
    }
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ChillAudio {
    voice: String,
    #[serde(default = "default_chillaudio_language")]
    language: String,
    #[serde(default)]
    preload: bool,
}
impl ChillAudio {
    fn valid(&self) -> bool {
        crate::providers::tts::chillaudio::descriptor::DESCRIPTOR
            .capabilities
            .voices
            .is_some_and(|voices| voices.iter().any(|voice| voice.id == self.voice))
            && self.language == "vi"
    }
}
fn default_chillaudio_language() -> String {
    "vi".into()
}
fn default_timeout() -> u64 {
    30_000
}
fn default_decode() -> TransducerDecodingMethod {
    TransducerDecodingMethod::ModifiedBeamSearch
}
fn default_paths() -> i32 {
    4
}
fn default_vietnamese_language() -> String {
    "vi-VN".into()
}
