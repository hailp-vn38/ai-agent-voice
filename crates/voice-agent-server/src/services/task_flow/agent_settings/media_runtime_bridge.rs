//! Server-side TTS & ASR runtime bridge (`ServerMediaRuntimeBridge`) for background task media tools.

use crate::app::AppState;

#[derive(Clone, Default)]
pub(crate) struct ServerMediaRuntimeBridge {
    config: Option<std::sync::Arc<crate::config::AppConfig>>,
    database: Option<std::sync::Arc<crate::database::Database>>,
    runtimes: std::sync::Arc<crate::providers::RuntimeCatalog>,
    runtime_manager:
        Option<std::sync::Arc<crate::services::provider_runtime::ProviderRuntimeManager>>,
}

impl ServerMediaRuntimeBridge {
    pub(crate) fn from_state(state: &AppState) -> Self {
        Self {
            config: Some(std::sync::Arc::clone(&state.config)),
            database: state.database.clone(),
            runtimes: std::sync::Arc::clone(&state.runtimes),
            runtime_manager: state.provider_runtime_manager.clone(),
        }
    }

    pub(crate) fn runtimes(&self) -> &std::sync::Arc<crate::providers::RuntimeCatalog> {
        &self.runtimes
    }

    pub(crate) fn runtimes_mut(&mut self) -> &mut std::sync::Arc<crate::providers::RuntimeCatalog> {
        &mut self.runtimes
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

    pub(crate) async fn acquire_managed_lease(
        &self,
        expected_kind: &str,
        label: &str,
        provider_ref: &str,
    ) -> Result<
        (
            crate::database::DesiredProvider,
            crate::services::provider_runtime::ResourceLease,
        ),
        String,
    > {
        let manager = self.runtime_manager.as_ref().ok_or_else(|| {
            format!(
                "{label} provider `{provider_ref}` runtime is not loaded and runtime manager is inactive"
            )
        })?;
        let desired = self
            .resolve_desired_provider(expected_kind, label, provider_ref)
            .await?;
        let deadline = manager.admission_deadline();
        let lease = if desired.id == 0 {
            manager
                .acquire_deployment_until(desired.clone(), deadline)
                .await
                .map_err(|e| format!("{label} deployment runtime lease failed: {e}"))?
        } else {
            manager
                .acquire_until(desired.clone(), deadline)
                .await
                .map_err(|e| format!("{label} database runtime lease failed: {e}"))?
        };
        Ok((desired, lease))
    }

    pub(crate) async fn resolve_desired_provider(
        &self,
        expected_kind: &str,
        label: &str,
        provider_ref: &str,
    ) -> Result<crate::database::DesiredProvider, String> {
        let prefix = format!("config:{expected_kind}:");
        let config_key = provider_ref.strip_prefix(&prefix).or_else(|| {
            self.config.as_ref().and_then(|c| {
                let has_key = match expected_kind {
                    "tts" => c.providers.tts.instances.contains_key(provider_ref),
                    "asr" => c.providers.asr.instances.contains_key(provider_ref),
                    _ => false,
                };
                has_key.then_some(provider_ref)
            })
        });
        if let Some(key) = config_key {
            let config = self
                .config
                .as_deref()
                .ok_or_else(|| format!("deployment config unavailable for {label} lookup"))?;
            return crate::providers::deployment_provider_snapshot(config, expected_kind, key)
                .map_err(|_| format!("config {label} provider `{provider_ref}` not found"));
        }
        let database = self
            .database
            .as_ref()
            .ok_or_else(|| format!("database is unavailable for {label} provider lookup"))?;
        let (id, key, kind, adapter, config, revision, enabled, credential) = database
            .diagnostic_provider_row(provider_ref)
            .await
            .map_err(|e| format!("{label} provider `{provider_ref}` lookup failed: {e}"))?;
        if enabled == 0 {
            return Err(format!("{label} provider `{provider_ref}` is disabled"));
        }
        if kind != expected_kind {
            return Err(format!(
                "provider `{provider_ref}` is not a {label} provider"
            ));
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

    pub(crate) async fn synthesize(
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
        let (desired, lease) = self
            .acquire_managed_lease("tts", "TTS", provider_ref)
            .await?;
        let runtime = lease
            .runtimes()
            .and_then(|cat| cat.tts.get(&desired.key).cloned())
            .ok_or_else(|| format!("TTS runtime `{provider_ref}` is unavailable in lease"))?;
        runtime
            .synthesize_background_wav(text, voice)
            .await
            .map_err(|e| e.to_string())
    }
}

pub(crate) fn inspect_wav_metadata(bytes: &[u8], fallback_text: &str, speed: f32) -> (u32, u64) {
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

pub(crate) static LIVE_SERVER_MEDIA_BRIDGE: std::sync::OnceLock<
    std::sync::Arc<std::sync::RwLock<Option<ServerMediaRuntimeBridge>>>,
> = std::sync::OnceLock::new();

pub(crate) fn install_server_media_runtime(state: &AppState) {
    let bridge = ServerMediaRuntimeBridge::from_state(state);
    let shared = LIVE_SERVER_MEDIA_BRIDGE
        .get_or_init(|| std::sync::Arc::new(std::sync::RwLock::new(None)))
        .clone();
    *shared.write().unwrap_or_else(|e| e.into_inner()) = Some(bridge);
}
