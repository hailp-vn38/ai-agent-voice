use serde::Serialize;

#[derive(Serialize)]
pub struct ProviderCapabilities {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub models: Option<&'static [ModelOption]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voices: Option<&'static [VoiceOption]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub languages: Option<&'static [LanguageOption]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub streaming: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offline: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calling: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vision: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_sample_rates: Option<&'static [u32]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channels: Option<&'static [u8]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_output_sample_rates: Option<&'static [u32]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voice_delivery_sample_rates: Option<&'static [u32]>,
}

#[derive(Serialize)]
pub struct ModelOption {
    pub id: &'static str,
    pub name: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<&'static str>,
}

#[derive(Serialize)]
pub struct LanguageOption {
    pub id: &'static str,
    pub name: &'static str,
}

#[derive(Serialize)]
pub struct VoiceOption {
    pub id: &'static str,
    pub name: &'static str,
    pub languages: &'static [&'static str],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<&'static str>,
}

#[derive(Serialize)]
pub struct CapabilityDiscoveryMode {
    pub models: DiscoverySource,
    pub voices: DiscoverySource,
    pub languages: DiscoverySource,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoverySource {
    Unsupported,
    Static,
    Bootstrap,
    Runtime,
    BootstrapAndRuntime,
    Remote,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilitySource {
    Models,
    Voices,
    Languages,
}
