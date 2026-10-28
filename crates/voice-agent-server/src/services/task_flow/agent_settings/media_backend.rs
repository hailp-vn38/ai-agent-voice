//! Media generation (`image_create`, `video_create`) and TTS synthesis (`tts_synthesize`)
//! runtime backends backed by the live LLM/TTS provider catalog.

use crate::app::AppState;

use super::persistence::{PersistedBackgroundSettings, load_persisted_settings};
use super::provider_catalog::ResolvedLlmProvider;

#[derive(Clone, Default)]
pub(crate) struct ServerTtsRuntimeBridge {
    config: Option<std::sync::Arc<crate::config::AppConfig>>,
    database: Option<std::sync::Arc<crate::database::Database>>,
    runtimes: std::sync::Arc<crate::providers::RuntimeCatalog>,
    runtime_manager:
        Option<std::sync::Arc<crate::services::provider_runtime::ProviderRuntimeManager>>,
}

impl ServerTtsRuntimeBridge {
    pub(crate) fn from_state(state: &AppState) -> Self {
        Self {
            config: Some(std::sync::Arc::clone(&state.config)),
            database: state.database.clone(),
            runtimes: std::sync::Arc::clone(&state.runtimes),
            runtime_manager: state.provider_runtime_manager.clone(),
        }
    }

    pub(crate) fn insert_runtime(
        &mut self,
        provider_ref: String,
        runtime: std::sync::Arc<crate::workers::TtsWorkerRuntime>,
    ) {
        std::sync::Arc::make_mut(&mut self.runtimes)
            .tts
            .insert(provider_ref, runtime);
    }

    async fn synthesize(
        &self,
        provider_ref: &str,
        text: &str,
        voice: Option<&str>,
    ) -> Result<Vec<u8>, String> {
        let bare_key = provider_ref
            .strip_prefix("config:tts:")
            .unwrap_or(provider_ref);

        // 1. Resident TTS runtime in `self.runtimes.tts`
        if let Some(runtime) = self
            .runtimes
            .tts
            .get(provider_ref)
            .or_else(|| self.runtimes.tts.get(bare_key))
            && runtime.provider().adapter() != "unavailable"
        {
            return runtime
                .synthesize_background_wav(text, voice)
                .await
                .map_err(|e| e.to_string());
        }

        // 2. Managed runtime lease via `ProviderRuntimeManager` (ADR-0071)
        let manager = self.runtime_manager.as_ref().ok_or_else(|| {
            format!(
                "TTS provider `{provider_ref}` runtime is not loaded and runtime manager is inactive"
            )
        })?;
        let desired = self.resolve_desired_tts_provider(provider_ref).await?;
        let deadline = manager.admission_deadline();
        let lease = if desired.id == 0 {
            manager
                .acquire_deployment_until(desired.clone(), deadline)
                .await
                .map_err(|e| format!("TTS deployment runtime lease failed: {e}"))?
        } else {
            manager
                .acquire_until(desired.clone(), deadline)
                .await
                .map_err(|e| format!("TTS database runtime lease failed: {e}"))?
        };
        let runtime = lease
            .runtimes()
            .and_then(|cat| cat.tts.get(&desired.key).cloned())
            .ok_or_else(|| format!("TTS runtime `{provider_ref}` is unavailable in lease"))?;
        runtime
            .synthesize_background_wav(text, voice)
            .await
            .map_err(|e| e.to_string())
    }

    async fn resolve_desired_tts_provider(
        &self,
        provider_ref: &str,
    ) -> Result<crate::database::DesiredProvider, String> {
        let config_key = provider_ref.strip_prefix("config:tts:").or_else(|| {
            self.config.as_ref().and_then(|c| {
                c.providers
                    .tts
                    .instances
                    .contains_key(provider_ref)
                    .then_some(provider_ref)
            })
        });
        if let Some(key) = config_key {
            let config = self
                .config
                .as_deref()
                .ok_or_else(|| "deployment config unavailable for TTS lookup".to_owned())?;
            return crate::providers::deployment_provider_snapshot(config, "tts", key)
                .map_err(|_| format!("config TTS provider `{provider_ref}` not found"));
        }
        let database = self
            .database
            .as_ref()
            .ok_or_else(|| "database is unavailable for TTS provider lookup".to_owned())?;
        let (id, key, kind, adapter, config, revision, enabled, credential) = database
            .diagnostic_provider_row(provider_ref)
            .await
            .map_err(|e| format!("TTS provider `{provider_ref}` lookup failed: {e}"))?;
        if enabled == 0 {
            return Err(format!("TTS provider `{provider_ref}` is disabled"));
        }
        if kind != "tts" {
            return Err(format!("provider `{provider_ref}` is not a TTS provider"));
        }
        let secret_ref = crate::database::credentials::reference(
            &format!("provider:{key}"),
            credential.as_deref(),
            crate::database::secrets::provider_secret_env(&key, &adapter),
        );
        Ok(crate::database::DesiredProvider {
            id,
            key,
            kind,
            adapter,
            config_json: config.unwrap_or_else(|| "{}".to_owned()),
            secret_ref,
            revision,
        })
    }
}

fn inspect_wav_metadata(bytes: &[u8], fallback_text: &str, speed: f32) -> (u32, u64) {
    if let Ok(reader) = hound::WavReader::new(std::io::Cursor::new(bytes)) {
        let spec = reader.spec();
        let sample_rate_hz = spec.sample_rate.max(1);
        let frames = u64::from(reader.duration());
        let duration_ms = ((frames * 1_000) / u64::from(sample_rate_hz)).max(1);
        (sample_rate_hz, duration_ms)
    } else {
        let word_count = fallback_text.split_whitespace().count().max(1);
        let duration_ms =
            (((word_count as f32) * 120.0 / speed.clamp(0.5, 2.0)) as u64).clamp(250, 60_000);
        (24_000, duration_ms)
    }
}

static LIVE_SERVER_TTS_BRIDGE: std::sync::OnceLock<
    std::sync::Arc<std::sync::RwLock<Option<ServerTtsRuntimeBridge>>>,
> = std::sync::OnceLock::new();

pub(crate) fn install_server_tts_runtime(state: &AppState) {
    let bridge = ServerTtsRuntimeBridge::from_state(state);
    let shared = LIVE_SERVER_TTS_BRIDGE
        .get_or_init(|| std::sync::Arc::new(std::sync::RwLock::new(None)))
        .clone();
    *shared.write().unwrap_or_else(|e| e.into_inner()) = Some(bridge);
}

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

#[derive(Clone)]
pub struct CatalogMediaBackend {
    store_dir: Option<std::path::PathBuf>,
    catalog: std::sync::Arc<std::sync::RwLock<Vec<ResolvedLlmProvider>>>,
    settings: std::sync::Arc<std::sync::RwLock<Option<PersistedBackgroundSettings>>>,
    tts_bridge: Option<ServerTtsRuntimeBridge>,
    default_image: orchestrator_flow_rs::DefaultImageGeneratorBackend,
    default_video: orchestrator_flow_rs::DefaultVideoGeneratorBackend,
    default_tts: orchestrator_flow_rs::DefaultTtsSynthesizerBackend,
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
            tts_bridge: None,
            default_image: orchestrator_flow_rs::DefaultImageGeneratorBackend,
            default_video: orchestrator_flow_rs::DefaultVideoGeneratorBackend,
            default_tts: orchestrator_flow_rs::DefaultTtsSynthesizerBackend,
        }
    }

    pub fn from_catalog(catalog: Vec<ResolvedLlmProvider>) -> Self {
        Self {
            store_dir: None,
            catalog: std::sync::Arc::new(std::sync::RwLock::new(catalog)),
            settings: std::sync::Arc::new(std::sync::RwLock::new(None)),
            tts_bridge: None,
            default_image: orchestrator_flow_rs::DefaultImageGeneratorBackend,
            default_video: orchestrator_flow_rs::DefaultVideoGeneratorBackend,
            default_tts: orchestrator_flow_rs::DefaultTtsSynthesizerBackend,
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
            .tts_bridge
            .get_or_insert_with(ServerTtsRuntimeBridge::default);
        bridge.insert_runtime(provider_ref.into(), runtime);
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

    async fn synthesize_via_server_tts(
        &self,
        provider_ref: &str,
        text: &str,
        voice: Option<&str>,
    ) -> Result<Vec<u8>, String> {
        let bridge = self
            .tts_bridge
            .clone()
            .or_else(|| {
                LIVE_SERVER_TTS_BRIDGE
                    .get()
                    .and_then(|lock| lock.read().unwrap_or_else(|e| e.into_inner()).clone())
            })
            .ok_or_else(|| {
                format!("server TTS runtime bridge is not initialized for `{provider_ref}`")
            })?;
        bridge.synthesize(provider_ref, text, voice).await
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
    let matches_prov = |p: &ResolvedLlmProvider, pref: &str| -> bool {
        p.provider_ref == pref && is_usable(p)
    };
    if let Some(settings) = settings {
        if let Some(tool_ref) = settings
            .agents
            .get("media_creator")
            .and_then(|o| o.tool_providers.as_ref())
            .and_then(|m| m.get(tool_name))
            .map(String::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            && let Some(found) = catalog.iter().find(|p| matches_prov(p, tool_ref))
        {
            return Some(found.clone());
        }
        if matches!(required_tag, "image" | "video" | "tts" | "audio")
            && let Some(explicit_ref) = settings
                .agents
                .get("media_creator")
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
        let first_frame = req.first_frame.as_ref().map(|img| crate::providers::ReferenceImage {
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

        if resolved.is_tts_adapter() {
            let wav_bytes = self
                .synthesize_via_server_tts(
                    &resolved.provider_ref,
                    &req.text,
                    (!req.voice.trim().is_empty()).then_some(req.voice.as_str()),
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
            });
        }

        let provider = resolved.to_llm_provider().map_err(|e| e.to_string())?;
        let speed = req.speed.clamp(0.5, 2.0);
        let output = provider
            .invoke(crate::providers::LlmOperation::AudioSpeech {
                text: req.text.clone(),
                voice: (!req.voice.trim().is_empty()).then(|| req.voice.clone()),
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
        })
    }
}
