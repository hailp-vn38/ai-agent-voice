use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;

use crate::config::LlmInstanceConfig;
use crate::providers::factory_registry::LlmFactory;
use crate::providers::ProviderLoadError;
use crate::providers::llm::{
    effective_capability_tags, LlmCapabilityTag, LlmError, LlmEvent, LlmEventStream, LlmOperation,
    LlmOutput, LlmProvider, LlmRequest, MAX_REFERENCE_IMAGES,
};

#[cfg(test)]
use super::cdp::BrowserEngine;
use super::config::{MuseConfig, ADAPTER_NAME};
#[cfg(test)]
use super::pool::MusePoolKey;
use super::pool::SharedMusePool;
use super::workflow::{
    execute_chat_streaming, execute_media_generation, execute_vision, format_image_prompt,
    format_video_prompt, MediaKind,
};

pub struct MuseLlmProvider {
    config: MuseConfig,
    tags: Vec<LlmCapabilityTag>,
    pool: Arc<SharedMusePool>,
}

impl MuseLlmProvider {
    pub fn from_config(config: &MuseConfig) -> Result<Self, LlmError> {
        if !config.valid() || config.parse_accounts().is_empty() {
            return Err(LlmError::Failed);
        }
        let tags = effective_muse_tags(config.tags.as_deref());
        let pool = SharedMusePool::get_or_create(config);
        Ok(Self {
            config: config.clone(),
            tags,
            pool,
        })
    }

    #[cfg(test)]
    pub(super) fn new_with_engine(
        config: MuseConfig,
        engine: Arc<dyn BrowserEngine>,
    ) -> Result<Self, LlmError> {
        use sha2::{Digest, Sha256};

        if !config.valid() || config.parse_accounts().is_empty() {
            return Err(LlmError::Failed);
        }
        let mut hasher = Sha256::new();
        hasher.update(config.api_key.expose().as_bytes());
        let credential_fingerprint: [u8; 32] = hasher.finalize().into();

        let key = MusePoolKey {
            cdp_port: config.cdp_port,
            headless: config.headless,
            account_concurrency: config.account_concurrency.clamp(1, 8),
            credential_fingerprint,
        };
        let pool =
            SharedMusePool::new_with_engine(key, config.parse_accounts(), engine, false);
        let tags = effective_muse_tags(config.tags.as_deref());
        Ok(Self { config, tags, pool })
    }

    fn timeout(&self) -> Duration {
        Duration::from_millis(self.config.timeout_ms.max(1_000))
    }
}

fn effective_muse_tags(tags: Option<&[String]>) -> Vec<LlmCapabilityTag> {
    if let Some(custom) = tags {
        if !custom.is_empty() {
            return effective_capability_tags(Some(custom));
        }
    }
    vec![
        LlmCapabilityTag::Chat,
        LlmCapabilityTag::Vision,
        LlmCapabilityTag::Image,
        LlmCapabilityTag::Video,
    ]
}

#[async_trait::async_trait]
impl LlmProvider for MuseLlmProvider {
    fn supports_tools(&self) -> bool {
        false
    }

    fn adapter(&self) -> &'static str {
        ADAPTER_NAME
    }

    fn capability_tags(&self) -> &[LlmCapabilityTag] {
        &self.tags
    }

    async fn stream(&self, request: LlmRequest) -> Result<LlmEventStream, LlmError> {
        self.ensure_tag(LlmCapabilityTag::Chat)?;
        let pool = Arc::clone(&self.pool);
        let timeout = self.timeout();
        let (tx, mut rx) = mpsc::unbounded_channel::<Result<LlmEvent, LlmError>>();

        tokio::spawn(async move {
            let tx_delta = tx.clone();
            let res = execute_chat_streaming(&pool, &request, &[], timeout, move |delta| {
                let _ = tx_delta.send(Ok(LlmEvent::TextDelta(delta)));
            })
            .await;

            match res {
                Ok(_) => {
                    let _ = tx.send(Ok(LlmEvent::Finished));
                }
                Err(err) => {
                    let _ = tx.send(Err(err));
                }
            }
        });

        let stream = futures_util::stream::poll_fn(move |cx| rx.poll_recv(cx));
        Ok(Box::pin(stream))
    }

    async fn execute_operation(&self, op: LlmOperation) -> Result<LlmOutput, LlmError> {
        let timeout = self.timeout();
        match op {
            LlmOperation::Chat { request, .. } => {
                let text =
                    execute_chat_streaming(&self.pool, &request, &[], timeout, |_| {}).await?;
                if text.trim().is_empty() {
                    return Err(LlmError::Rejected);
                }
                Ok(LlmOutput::Text(text))
            }
            LlmOperation::Vision {
                prompt,
                image_bytes,
                mime_type,
            } => execute_vision(&self.pool, &prompt, image_bytes, mime_type, timeout).await,
            LlmOperation::ImageGeneration {
                prompt,
                width,
                height,
                reference_images,
                transparent_background,
            } => {
                if reference_images.len() > MAX_REFERENCE_IMAGES {
                    return Err(LlmError::Rejected);
                }
                let formatted =
                    format_image_prompt(&prompt, width, height, transparent_background);
                execute_media_generation(
                    &self.pool,
                    MediaKind::Image,
                    &formatted,
                    &reference_images,
                    transparent_background,
                    timeout,
                )
                .await
            }
            LlmOperation::VideoGeneration {
                prompt,
                width,
                height,
                duration_secs,
                first_frame,
            } => {
                let formatted = format_video_prompt(&prompt, width, height, duration_secs);
                let images = first_frame.into_iter().collect::<Vec<_>>();
                execute_media_generation(
                    &self.pool,
                    MediaKind::Video,
                    &formatted,
                    &images,
                    false,
                    timeout,
                )
                .await
            }
            LlmOperation::AudioSpeech { .. }
            | LlmOperation::AudioTranscription { .. }
            | LlmOperation::Embedding { .. } => {
                Err(LlmError::UnsupportedCapability(op.required_tag().as_str()))
            }
        }
    }
}

pub struct MuseFactory;

impl LlmFactory for MuseFactory {
    fn adapter(&self) -> &'static str {
        ADAPTER_NAME
    }

    fn build(&self, config: &LlmInstanceConfig) -> Result<Arc<dyn LlmProvider>, ProviderLoadError> {
        let LlmInstanceConfig::Muse(options) = config else {
            return Err(ProviderLoadError::Configuration(
                "Muse factory received another adapter config".into(),
            ));
        };
        if !options.valid() || options.parse_accounts().is_empty() {
            return Err(ProviderLoadError::Configuration(
                "invalid Muse configuration or missing required session cookies".into(),
            ));
        }
        MuseLlmProvider::from_config(options)
            .map(|p| Arc::new(p) as Arc<dyn LlmProvider>)
            .map_err(|_| {
                ProviderLoadError::Provider("Muse provider initialization failed".into())
            })
    }
}
