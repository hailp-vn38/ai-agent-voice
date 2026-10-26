use std::time::Duration;

use url::Url;

#[cfg(test)]
use crate::config::SecretString;
use crate::providers::llm::{
    LlmCapabilityTag, LlmError, LlmEventStream, LlmOperation, LlmOutput, LlmProvider, LlmRequest,
    effective_capability_tags, openai::config::OpenAiConfig, valid_generation_bounds,
    valid_llm_tags,
};

pub(crate) struct ConfiguredOpenAiLlm {
    pub(super) api_key: String,
    base_url: Url,
    pub(super) base_url_str: String,
    pub(super) model: String,
    pub(super) timeout_seconds: u64,
    pub(super) max_tokens: Option<u32>,
    reasoning_effort: Option<String>,
    temperature_percent: Option<u16>,
    tags: Vec<LlmCapabilityTag>,
    pub(super) client: reqwest::Client,
}

impl ConfiguredOpenAiLlm {
    pub(crate) fn from_config(config: &OpenAiConfig) -> Result<Self, LlmError> {
        if config.model.trim().is_empty()
            || config.timeout_ms == 0
            || !valid_generation_bounds(
                config.max_tokens,
                config.context_limit_tokens,
                config.reasoning_effort.as_deref(),
                config.temperature_percent,
            )
            || !valid_llm_tags(config.tags.as_deref())
        {
            return Err(LlmError::Failed);
        }
        let mut base_url = config.base_url.clone();
        if !base_url.path().ends_with('/') {
            base_url.set_path(&format!("{}/", base_url.path()));
        }
        let timeout_seconds = config.timeout_ms.div_ceil(1_000).max(1);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(config.timeout_ms))
            .build()
            .map_err(|_| LlmError::Failed)?;
        Ok(Self {
            api_key: config.api_key.expose().to_owned(),
            base_url_str: config.base_url.as_str().to_owned(),
            base_url,
            model: config.model.clone(),
            timeout_seconds,
            max_tokens: config.max_tokens,
            reasoning_effort: config.reasoning_effort.clone(),
            temperature_percent: config.temperature_percent,
            tags: effective_capability_tags(config.tags.as_deref()),
            client,
        })
    }

    #[cfg(test)]
    pub(crate) fn build(
        api_key: &str,
        base_url: &str,
        model: &str,
        timeout_seconds: u64,
    ) -> Result<Self, LlmError> {
        if model.trim().is_empty() || timeout_seconds == 0 {
            return Err(LlmError::Failed);
        }
        let parsed_url = Url::parse(base_url).map_err(|_| LlmError::Failed)?;
        Self::from_config(&OpenAiConfig {
            api_key: SecretString::new(api_key),
            base_url: parsed_url,
            model: model.to_owned(),
            timeout_ms: timeout_seconds.saturating_mul(1_000),
            ..OpenAiConfig::default()
        })
    }

    pub(super) fn temperature(&self) -> Option<f32> {
        self.temperature_percent.map(|pct| f32::from(pct) / 100.0)
    }

    pub(super) fn resolved_reasoning_effort(
        &self,
        force_reasoning: bool,
    ) -> Option<llm::chat::ReasoningEffort> {
        match self.reasoning_effort.as_deref() {
            Some("low") => Some(llm::chat::ReasoningEffort::Low),
            Some("medium") => Some(llm::chat::ReasoningEffort::Medium),
            Some("high") => Some(llm::chat::ReasoningEffort::High),
            _ if force_reasoning || self.tags.contains(&LlmCapabilityTag::Reasoning) => {
                Some(llm::chat::ReasoningEffort::Medium)
            }
            _ => None,
        }
    }

    pub(super) fn endpoint(&self, relative: &str) -> Result<Url, LlmError> {
        self.base_url
            .join(relative.trim_start_matches('/'))
            .map_err(|_| LlmError::Failed)
    }

    pub(super) fn ensure_api_key(&self) -> Result<&str, LlmError> {
        if self.api_key.trim().is_empty() {
            Err(LlmError::Failed)
        } else {
            Ok(&self.api_key)
        }
    }

    pub(super) async fn check_response(
        response: reqwest::Response,
    ) -> Result<reqwest::Response, LlmError> {
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        if matches!(status.as_u16(), 408 | 504) {
            Err(LlmError::Failed)
        } else {
            Err(LlmError::Rejected)
        }
    }
}

#[async_trait::async_trait]
impl LlmProvider for ConfiguredOpenAiLlm {
    fn supports_tools(&self) -> bool {
        self.supports_tag(LlmCapabilityTag::Chat)
    }

    fn adapter(&self) -> &'static str {
        "openai"
    }

    fn capability_tags(&self) -> &[LlmCapabilityTag] {
        &self.tags
    }

    async fn stream(&self, request: LlmRequest) -> Result<LlmEventStream, LlmError> {
        self.ensure_tag(LlmCapabilityTag::Chat)?;
        self.open_chat_stream(request, false).await
    }

    async fn execute_operation(&self, op: LlmOperation) -> Result<LlmOutput, LlmError> {
        match op {
            LlmOperation::Chat { request, reasoning } => {
                self.complete_chat(request, reasoning).await
            }
            LlmOperation::Vision {
                prompt,
                image_bytes,
                mime_type,
            } => self.complete_vision(&prompt, &image_bytes, &mime_type).await,
            LlmOperation::ImageGeneration {
                prompt,
                width,
                height,
            } => self.generate_image(&prompt, width, height).await,
            LlmOperation::VideoGeneration {
                prompt,
                width,
                height,
                duration_secs,
            } => {
                self.generate_video(&prompt, width, height, duration_secs)
                    .await
            }
            LlmOperation::AudioSpeech { text, voice, speed } => {
                self.synthesize_speech(&text, voice.as_deref(), speed).await
            }
            LlmOperation::AudioTranscription {
                wav_bytes,
                language,
            } => self.transcribe_audio(wav_bytes, language.as_deref()).await,
            LlmOperation::Embedding { inputs } => self.create_embeddings(inputs).await,
        }
    }
}
