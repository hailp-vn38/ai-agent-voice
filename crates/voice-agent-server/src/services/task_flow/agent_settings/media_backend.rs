//! Media generation (`image_create`, `video_create`), TTS synthesis (`tts_synthesize`),
//! and ASR transcription (`asr_transcribe`) runtime backends backed by the live LLM/TTS/ASR provider catalog.

use super::media_asr::decode_wav_to_16khz_mono;
pub(crate) use super::media_runtime_bridge::{
    LIVE_SERVER_MEDIA_BRIDGE, ServerMediaRuntimeBridge, inspect_wav_metadata,
    install_server_media_runtime,
};
use super::persistence::{PersistedBackgroundSettings, load_persisted_settings};
use super::provider_catalog::ResolvedLlmProvider;

static LIVE_MEDIA_CATALOG: std::sync::OnceLock<
    std::sync::Arc<std::sync::RwLock<Vec<ResolvedLlmProvider>>>,
> = std::sync::OnceLock::new();

static LIVE_MEDIA_SETTINGS: std::sync::OnceLock<
    std::sync::Arc<std::sync::RwLock<Option<PersistedBackgroundSettings>>>,
> = std::sync::OnceLock::new();

pub(crate) fn update_live_media_catalog(
    catalog: &[ResolvedLlmProvider],
    settings: Option<&PersistedBackgroundSettings>,
) {
    if let Some(shared) = LIVE_MEDIA_CATALOG.get() {
        *shared.write().unwrap_or_else(|e| e.into_inner()) = catalog.to_vec();
    }
    if let Some(settings) = settings
        && let Some(shared_settings) = LIVE_MEDIA_SETTINGS.get()
    {
        *shared_settings.write().unwrap_or_else(|e| e.into_inner()) = Some(settings.clone());
    }
}

pub type CatalogImageGeneratorBackend = CatalogMediaBackend;
pub type CatalogVideoGeneratorBackend = CatalogMediaBackend;
pub type CatalogTtsSynthesizerBackend = CatalogMediaBackend;
pub type CatalogAsrTranscriberBackend = CatalogMediaBackend;

#[derive(Clone)]
pub struct CatalogMediaBackend {
    store_dir: Option<std::path::PathBuf>,
    catalog: std::sync::Arc<std::sync::RwLock<Vec<ResolvedLlmProvider>>>,
    settings: std::sync::Arc<std::sync::RwLock<Option<PersistedBackgroundSettings>>>,
    media_bridge: Option<ServerMediaRuntimeBridge>,
    default_image: orchestrator_flow_rs::DefaultImageGeneratorBackend,
    default_video: orchestrator_flow_rs::DefaultVideoGeneratorBackend,
    default_tts: orchestrator_flow_rs::DefaultTtsSynthesizerBackend,
    default_asr: orchestrator_flow_rs::DefaultAsrTranscriberBackend,
}

impl CatalogMediaBackend {
    pub fn new(
        store_dir: impl Into<std::path::PathBuf>,
        catalog: Vec<ResolvedLlmProvider>,
    ) -> Self {
        let dir = store_dir.into();
        let initial_settings = load_persisted_settings(&dir);
        let shared = LIVE_MEDIA_CATALOG
            .get_or_init(|| std::sync::Arc::new(std::sync::RwLock::new(catalog.clone())))
            .clone();
        *shared.write().unwrap_or_else(|e| e.into_inner()) = catalog;
        let shared_settings = LIVE_MEDIA_SETTINGS
            .get_or_init(|| std::sync::Arc::new(std::sync::RwLock::new(initial_settings.clone())))
            .clone();
        *shared_settings.write().unwrap_or_else(|e| e.into_inner()) = initial_settings;
        Self {
            store_dir: Some(dir),
            catalog: shared,
            settings: shared_settings,
            media_bridge: None,
            default_image: orchestrator_flow_rs::DefaultImageGeneratorBackend,
            default_video: orchestrator_flow_rs::DefaultVideoGeneratorBackend,
            default_tts: orchestrator_flow_rs::DefaultTtsSynthesizerBackend,
            default_asr: orchestrator_flow_rs::DefaultAsrTranscriberBackend,
        }
    }

    pub fn from_catalog(catalog: Vec<ResolvedLlmProvider>) -> Self {
        Self {
            store_dir: None,
            catalog: std::sync::Arc::new(std::sync::RwLock::new(catalog)),
            settings: std::sync::Arc::new(std::sync::RwLock::new(None)),
            media_bridge: None,
            default_image: orchestrator_flow_rs::DefaultImageGeneratorBackend,
            default_video: orchestrator_flow_rs::DefaultVideoGeneratorBackend,
            default_tts: orchestrator_flow_rs::DefaultTtsSynthesizerBackend,
            default_asr: orchestrator_flow_rs::DefaultAsrTranscriberBackend,
        }
    }

    pub fn with_settings(self, settings: PersistedBackgroundSettings) -> Self {
        *self.settings.write().unwrap_or_else(|e| e.into_inner()) = Some(settings);
        self
    }

    pub fn with_tts_runtime(
        mut self,
        provider_ref: impl Into<String>,
        runtime: std::sync::Arc<crate::workers::TtsWorkerRuntime>,
    ) -> Self {
        let bridge = self
            .media_bridge
            .get_or_insert_with(ServerMediaRuntimeBridge::default);
        bridge.insert_runtime(provider_ref.into(), runtime);
        self
    }

    pub fn with_asr_runtime(
        mut self,
        provider_ref: impl Into<String>,
        runtime: std::sync::Arc<crate::workers::AsrWorkerRuntime>,
    ) -> Self {
        let bridge = self
            .media_bridge
            .get_or_insert_with(ServerMediaRuntimeBridge::default);
        bridge.insert_asr_runtime(provider_ref.into(), runtime);
        self
    }

    fn effective_settings(&self) -> Option<PersistedBackgroundSettings> {
        if let Some(ref dir) = self.store_dir
            && let Some(loaded) = load_persisted_settings(dir)
        {
            return Some(loaded);
        }
        self.settings
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn select_provider(&self, tool_name: &str, required_tag: &str) -> Option<ResolvedLlmProvider> {
        let guard = self.catalog.read().unwrap_or_else(|e| e.into_inner());
        let settings = self.effective_settings();
        resolve_media_tool_provider(&guard, settings.as_ref(), tool_name, required_tag, true)
    }

    fn resolve_bridge(
        &self,
        label: &str,
        provider_ref: &str,
    ) -> Result<ServerMediaRuntimeBridge, String> {
        self.media_bridge
            .clone()
            .or_else(|| {
                LIVE_SERVER_MEDIA_BRIDGE
                    .get()
                    .and_then(|lock| lock.read().unwrap_or_else(|e| e.into_inner()).clone())
            })
            .ok_or_else(|| {
                format!("server {label} runtime bridge is not initialized for `{provider_ref}`")
            })
    }

    async fn synthesize_via_server_tts(
        &self,
        provider_ref: &str,
        text: &str,
        voice: Option<&str>,
    ) -> Result<Vec<u8>, String> {
        let bridge = self.resolve_bridge("TTS", provider_ref)?;
        bridge.synthesize(provider_ref, text, voice).await
    }

    fn inspect_runtime_model_and_voice(
        &self,
        provider_ref: &str,
    ) -> (Option<String>, Option<String>) {
        let Ok(bridge) = self.resolve_bridge("TTS", provider_ref) else {
            return (None, None);
        };
        bridge.inspect_runtime_model_and_voice(provider_ref)
    }

    async fn transcribe_via_server_asr(
        &self,
        provider_ref: &str,
        pcm: crate::audio::PcmF32Mono,
    ) -> Result<super::media_asr::MediaAsrTranscript, String> {
        let bridge = self.resolve_bridge("ASR", provider_ref)?;
        bridge.transcribe_asr(provider_ref, pcm).await
    }
}

pub(crate) fn resolve_media_tool_provider(
    catalog: &[ResolvedLlmProvider],
    settings: Option<&PersistedBackgroundSettings>,
    tool_name: &str,
    required_tag: &str,
    require_api_key: bool,
) -> Option<ResolvedLlmProvider> {
    let is_usable = |p: &ResolvedLlmProvider| -> bool {
        p.supports_tag(required_tag)
            && (!require_api_key || !p.requires_api_key() || p.api_key.is_some())
    };
    let matches_prov =
        |p: &ResolvedLlmProvider, pref: &str| -> bool { p.provider_ref == pref && is_usable(p) };
    let owner_id = super::specs::find_tool_owner_spec(tool_name)
        .map(|s| s.id)
        .unwrap_or("media_creator");
    if let Some(settings) = settings {
        if let Some(tool_ref) = settings
            .agents
            .get(owner_id)
            .and_then(|o| o.tool_providers.as_ref())
            .and_then(|m| m.get(tool_name))
            .map(String::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            && let Some(found) = catalog.iter().find(|p| matches_prov(p, tool_ref))
        {
            return Some(found.clone());
        }
        if matches!(required_tag, "image" | "video" | "tts" | "asr" | "audio")
            && let Some(explicit_ref) = settings
                .agents
                .get(owner_id)
                .and_then(|o| o.provider_ref.as_deref())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            && let Some(found) = catalog.iter().find(|p| matches_prov(p, explicit_ref))
        {
            return Some(found.clone());
        }
        let worker_ref = settings.worker_llm_provider.trim();
        if !worker_ref.is_empty()
            && let Some(found) = catalog.iter().find(|p| matches_prov(p, worker_ref))
        {
            return Some(found.clone());
        }
    }
    if required_tag.eq_ignore_ascii_case("tts")
        && let Some(tts_prov) = catalog.iter().find(|p| p.is_tts_adapter() && is_usable(p))
    {
        return Some(tts_prov.clone());
    }
    if required_tag.eq_ignore_ascii_case("asr")
        && let Some(asr_prov) = catalog.iter().find(|p| p.is_asr_adapter() && is_usable(p))
    {
        return Some(asr_prov.clone());
    }
    catalog.iter().find(|p| is_usable(p)).cloned()
}

#[async_trait::async_trait]
impl orchestrator_flow_rs::ImageGeneratorBackend for CatalogMediaBackend {
    async fn generate_image(
        &self,
        req: &orchestrator_flow_rs::ImageGenerationRequest,
    ) -> Result<orchestrator_flow_rs::GeneratedImagePayload, String> {
        if req.is_svg() {
            return self.default_image.generate_image(req).await;
        }
        let Some(resolved) = self.select_provider("image_create", "image") else {
            return self.default_image.generate_image(req).await;
        };
        let (width, height) = req.effective_dimensions();
        let provider = resolved.to_llm_provider().map_err(|e| e.to_string())?;
        let reference_images = req
            .reference_images
            .iter()
            .map(|img| crate::providers::ReferenceImage {
                bytes: img.bytes.clone(),
                mime_type: img.mime_type.clone(),
            })
            .collect();
        let output = provider
            .invoke(crate::providers::LlmOperation::ImageGeneration {
                prompt: req.prompt.clone(),
                width,
                height,
                reference_images,
                transparent_background: req.transparent_background,
            })
            .await
            .map_err(|e| e.to_string())?;
        let crate::providers::LlmOutput::Media {
            bytes, mime_type, ..
        } = output
        else {
            return Err("image provider returned non-media output".to_owned());
        };
        let stem = llm_wiki::slugify(
            req.filename
                .trim_end_matches(".png")
                .trim_end_matches(".svg"),
            64,
        );
        let safe_name = format!("{}.png", if stem.is_empty() { "image" } else { &stem });
        Ok(orchestrator_flow_rs::GeneratedImagePayload {
            filename: safe_name,
            mime_type,
            bytes,
            width,
            height,
            provider: resolved.provider_ref.clone(),
        })
    }
}

#[async_trait::async_trait]
impl orchestrator_flow_rs::VideoGeneratorBackend for CatalogMediaBackend {
    fn video_constraints(&self) -> orchestrator_flow_rs::VideoProviderConstraints {
        match self.select_provider("video_create", "video") {
            Some(resolved) => resolved.video_constraints(),
            None => self.default_video.video_constraints(),
        }
    }

    async fn generate_video(
        &self,
        req: &orchestrator_flow_rs::VideoGenerationRequest,
    ) -> Result<orchestrator_flow_rs::GeneratedVideoPayload, String> {
        let Some(resolved) = self.select_provider("video_create", "video") else {
            return self.default_video.generate_video(req).await;
        };
        let constraints = resolved.video_constraints();
        let (width, height) = req.effective_dimensions();
        let duration_secs = req
            .duration_secs
            .clamp(1, constraints.max_total_duration_secs.max(1));
        let provider = resolved.to_llm_provider().map_err(|e| e.to_string())?;
        let first_frame = req
            .first_frame
            .as_ref()
            .map(|img| crate::providers::ReferenceImage {
                bytes: img.bytes.clone(),
                mime_type: img.mime_type.clone(),
            });
        let output = provider
            .invoke(crate::providers::LlmOperation::VideoGeneration {
                prompt: req.prompt.clone(),
                width,
                height,
                duration_secs,
                first_frame,
            })
            .await
            .map_err(|e| e.to_string())?;
        build_generated_video_payload(
            req,
            &resolved.provider_ref,
            &constraints,
            width,
            height,
            duration_secs,
            output,
        )
    }
}

pub(crate) fn build_generated_video_payload(
    req: &orchestrator_flow_rs::VideoGenerationRequest,
    provider_ref: &str,
    constraints: &orchestrator_flow_rs::VideoProviderConstraints,
    width: u32,
    height: u32,
    duration_secs: u32,
    output: crate::providers::LlmOutput,
) -> Result<orchestrator_flow_rs::GeneratedVideoPayload, String> {
    let Some((parts, assistant_message)) = output.into_media_parts() else {
        return Err("video provider returned non-media output".to_owned());
    };
    if parts.is_empty() {
        return Err("video provider returned empty media parts".to_owned());
    }
    let base_stem = orchestrator_flow_rs::normalize_video_stem(&req.filename, "video");
    let safe_name = format!("{base_stem}.mp4");
    let total_clips = parts.len();
    let fallback_clip_duration = if total_clips > 1 {
        constraints.max_clip_duration_secs.max(1)
    } else {
        duration_secs.min(constraints.max_clip_duration_secs.max(1))
    };
    let clips: Vec<orchestrator_flow_rs::GeneratedVideoClip> = parts
        .into_iter()
        .enumerate()
        .map(|(idx, part)| {
            let clip_duration = orchestrator_flow_rs::inspect_mp4_duration_secs(&part.bytes)
                .unwrap_or(fallback_clip_duration);
            let clip_filename = if total_clips == 1 {
                safe_name.clone()
            } else {
                format!("{base_stem}-part-{}.mp4", idx + 1)
            };
            orchestrator_flow_rs::GeneratedVideoClip {
                filename: clip_filename,
                mime_type: part.mime_type,
                bytes: part.bytes,
                width,
                height,
                duration_secs: clip_duration,
            }
        })
        .collect();
    let total_duration: u32 = clips.iter().map(|c| c.duration_secs).sum();
    let first_mime = clips[0].mime_type.clone();
    let first_bytes = if total_clips == 1 {
        clips[0].bytes.clone()
    } else {
        Vec::new()
    };
    Ok(orchestrator_flow_rs::GeneratedVideoPayload {
        filename: safe_name,
        mime_type: first_mime,
        bytes: first_bytes,
        width,
        height,
        duration_secs: total_duration.max(1),
        provider: provider_ref.to_owned(),
        clips,
        provider_message: assistant_message,
    })
}

#[async_trait::async_trait]
impl orchestrator_flow_rs::TtsSynthesizerBackend for CatalogMediaBackend {
    fn tts_constraints(&self) -> orchestrator_flow_rs::TtsProviderConstraints {
        let Some(resolved) = self.select_provider("tts_synthesize", "tts") else {
            return self.default_tts.tts_constraints();
        };
        let (runtime_model, runtime_voice) =
            self.inspect_runtime_model_and_voice(&resolved.provider_ref);
        resolved.tts_constraints_with_runtime(runtime_model.as_deref(), runtime_voice.as_deref())
    }

    async fn synthesize_tts(
        &self,
        req: &orchestrator_flow_rs::TtsSynthesisRequest,
    ) -> Result<orchestrator_flow_rs::SynthesizedAudioPayload, String> {
        let Some(resolved) = self.select_provider("tts_synthesize", "tts") else {
            return self.default_tts.synthesize_tts(req).await;
        };
        let stem = llm_wiki::slugify(
            req.filename
                .trim_end_matches(".wav")
                .trim_end_matches(".mp3"),
            64,
        );
        let safe_name = format!("{}.wav", if stem.is_empty() { "tts-audio" } else { &stem });
        let (runtime_model, runtime_voice) =
            self.inspect_runtime_model_and_voice(&resolved.provider_ref);
        let effective_voice = resolved.resolve_tts_voice_with_runtime(
            runtime_model.as_deref(),
            runtime_voice.as_deref(),
            Some(&req.voice),
        );

        if resolved.is_tts_adapter() {
            let wav_bytes = self
                .synthesize_via_server_tts(
                    &resolved.provider_ref,
                    &req.text,
                    Some(effective_voice.as_str()),
                )
                .await?;
            let (sample_rate_hz, duration_ms) =
                inspect_wav_metadata(&wav_bytes, &req.text, req.speed);
            return Ok(orchestrator_flow_rs::SynthesizedAudioPayload {
                filename: safe_name,
                mime_type: "audio/wav".to_owned(),
                bytes: wav_bytes,
                sample_rate_hz,
                duration_ms,
                provider: resolved.provider_ref.clone(),
                voice: effective_voice,
            });
        }

        let provider = resolved.to_llm_provider().map_err(|e| e.to_string())?;
        let speed = req.speed.clamp(0.5, 2.0);
        let output = provider
            .invoke(crate::providers::LlmOperation::AudioSpeech {
                text: req.text.clone(),
                voice: Some(effective_voice.clone()),
                speed: Some(speed),
            })
            .await
            .map_err(|e| e.to_string())?;
        let crate::providers::LlmOutput::Media {
            bytes, mime_type, ..
        } = output
        else {
            return Err("audio provider returned non-media output".to_owned());
        };
        let word_count = req.text.split_whitespace().count().max(1);
        let duration_ms = (((word_count as f32) * 120.0 / speed) as u64).clamp(250, 60_000);
        Ok(orchestrator_flow_rs::SynthesizedAudioPayload {
            filename: safe_name,
            mime_type,
            bytes,
            sample_rate_hz: 24_000,
            duration_ms,
            provider: resolved.provider_ref.clone(),
            voice: effective_voice,
        })
    }
}

#[async_trait::async_trait]
impl orchestrator_flow_rs::AsrTranscriberBackend for CatalogMediaBackend {
    async fn transcribe_asr(
        &self,
        req: &orchestrator_flow_rs::AsrTranscriptionRequest,
    ) -> Result<orchestrator_flow_rs::TranscribedAudioPayload, String> {
        let Some(resolved) = self.select_provider("asr_transcribe", "asr") else {
            return self.default_asr.transcribe_asr(req).await;
        };
        let lang = req
            .language
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);

        if resolved.is_asr_adapter() {
            let wav_bytes = req.wav_bytes.clone();
            let pcm = tokio::task::spawn_blocking(move || decode_wav_to_16khz_mono(&wav_bytes))
                .await
                .map_err(|e| format!("WAV decode worker join error: {e}"))??;
            let transcript = self
                .transcribe_via_server_asr(&resolved.provider_ref, pcm)
                .await?;
            let segments = transcript
                .segments
                .into_iter()
                .map(|seg| orchestrator_flow_rs::TranscribedAudioSegment {
                    start_ms: seg.start_ms,
                    end_ms: seg.end_ms,
                    text: seg.text,
                })
                .collect();
            return Ok(orchestrator_flow_rs::TranscribedAudioPayload {
                text: transcript.text,
                language: lang,
                duration_ms: req.header.duration_ms,
                sample_rate: req.header.sample_rate,
                channels: req.header.channels,
                provider_id: Some(resolved.provider_ref.clone()),
                model: Some(resolved.model.clone()),
                segments,
            });
        }

        let provider = resolved.to_llm_provider().map_err(|e| e.to_string())?;
        let output = provider
            .invoke(crate::providers::LlmOperation::AudioTranscription {
                wav_bytes: req.wav_bytes.clone(),
                language: lang.clone(),
            })
            .await
            .map_err(|e| e.to_string())?;
        let crate::providers::LlmOutput::Text(text) = output else {
            return Err("ASR audio provider returned non-text output".to_owned());
        };
        Ok(orchestrator_flow_rs::TranscribedAudioPayload {
            text,
            language: lang,
            duration_ms: req.header.duration_ms,
            sample_rate: req.header.sample_rate,
            channels: req.header.channels,
            provider_id: Some(resolved.provider_ref.clone()),
            model: Some(resolved.model.clone()),
            segments: Vec::new(),
        })
    }
}
