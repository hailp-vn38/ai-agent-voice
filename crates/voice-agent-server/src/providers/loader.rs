use std::{collections::HashMap, sync::Arc, time::Duration};

use crate::{
    audio::VadSegmenterConfig,
    config::{AppConfig, SileroOnnxConfig},
    models::prepare,
    providers::{
        LoadedVad, ProviderCatalog, ProviderLoadError, RuntimeCatalog, compiled_provider_registry,
    },
    workers::{
        AsrWorkerRuntime, LlmRuntime, TtsWorkerRuntime, VadWorkerRuntime, VisionRuntime,
        WorkerRuntimeConfig,
    },
};

/// One millisecond of VAD timing is exactly 16 samples at the 16 kHz capture rate, so the
/// deployment TOML and a database provider describe segmentation with the same numbers.
pub(crate) fn vad_timing(config: &SileroOnnxConfig) -> (VadSegmenterConfig, u64) {
    (
        VadSegmenterConfig {
            speech_threshold: config.speech_threshold,
            exit_threshold: config.exit_threshold,
            min_speech_samples: config.min_speech_ms * 16,
            end_silence_samples: config.end_silence_ms * 16,
        },
        config.pre_roll_ms * 16,
    )
}

pub struct LoadedProviders {
    pub providers: ProviderCatalog,
    pub runtimes: RuntimeCatalog,
}

impl LoadedProviders {
    /// A database instance key that the deployment already loaded can never be resolved
    /// unambiguously, so the plan rejects it before any credential is resolved.
    pub(crate) fn has_loaded_key(&self, kind: &str, key: &str) -> bool {
        match kind {
            "vad" => self.runtimes.vad.contains_key(key),
            "asr" => self.runtimes.asr.contains_key(key),
            "llm" => self.runtimes.llm.contains_key(key),
            "tts" => self.runtimes.tts.contains_key(key),
            _ => false,
        }
    }
}

impl LoadedProviders {
    /// Database instance keys must never replace a deployment-selected instance.  Ticket 07 can
    /// resolve DB bindings only when this startup snapshot has an unambiguous runtime.
    pub(crate) fn extend_database_without_collisions(&mut self, other: Self) -> Vec<String> {
        let mut collisions = Vec::new();
        for (key, provider) in other.providers.vad {
            if self.providers.vad.contains_key(&key) {
                collisions.push(key);
            } else {
                let loaded = other.runtimes.vad[&key].clone();
                self.providers.vad.insert(key.clone(), provider);
                self.runtimes.vad.insert(key, loaded);
            }
        }
        for (key, provider) in other.providers.asr {
            if self.providers.asr.contains_key(&key) {
                collisions.push(key);
            } else {
                let runtime = other.runtimes.asr[&key].clone();
                self.providers.asr.insert(key.clone(), provider);
                self.runtimes.asr.insert(key, runtime);
            }
        }
        for (key, provider) in other.providers.llm {
            if self.providers.llm.contains_key(&key) {
                collisions.push(key);
            } else {
                let runtime = other.runtimes.llm[&key].clone();
                self.providers.llm.insert(key.clone(), provider);
                self.runtimes.llm.insert(key, runtime);
            }
        }
        for (key, provider) in other.providers.tts {
            if self.providers.tts.contains_key(&key) {
                collisions.push(key);
            } else {
                let runtime = other.runtimes.tts[&key].clone();
                self.providers.tts.insert(key.clone(), provider);
                self.runtimes.tts.insert(key, runtime);
            }
        }
        collisions
    }
}

/// Builds only effective bindings and explicitly preloaded instances. Other valid instances stay
/// configured but unloaded until a later lazy-loading phase is introduced.
pub(crate) fn load_local(config: &AppConfig) -> Result<LoadedProviders, ProviderLoadError> {
    config
        .validate()
        .map_err(|error| ProviderLoadError::Configuration(error.to_string()))?;
    let registry = compiled_provider_registry();
    let bindings = &config.effective_agent.providers;
    let defaults = &config.provider_defaults;

    let mut vad_providers = HashMap::new();
    let mut vad_runtimes = HashMap::new();
    for (id, instance) in &config.providers.vad.instances {
        if id != &bindings.vad && id != &defaults.vad {
            continue;
        }
        let factory = registry.vad_factory(instance.adapter())?;
        let model = prepare(
            &config.deployment.model_manifest,
            &config.deployment.models.root,
            config.deployment.models.offline,
            factory.model_identity(instance)?,
            factory.adapter(),
            &config.deployment,
        )?;
        let provider = factory.build(instance, &config.runtime, &model)?;
        let (segmenter, pre_roll_samples) = vad_timing(instance.silero_onnx());
        vad_runtimes.insert(
            id.clone(),
            LoadedVad {
                runtime: Arc::new(VadWorkerRuntime::new(
                    Arc::clone(&provider),
                    WorkerRuntimeConfig {
                        max_workers: config.workers.vad.max_workers,
                        command_capacity: config.workers.vad.command_queue_capacity,
                        final_timeout: Duration::from_millis(config.workers.vad.reset_timeout_ms),
                        cleanup_grace: Duration::from_millis(config.workers.vad.cleanup_grace_ms),
                    },
                )),
                segmenter,
                pre_roll_samples,
            },
        );
        tracing::info!(provider_kind = "vad", provider_instance = %id, adapter = instance.adapter(), "provider runtime loaded");
        vad_providers.insert(id.clone(), provider);
    }

    let mut asr_providers = HashMap::new();
    let mut asr_runtimes = HashMap::new();
    for (id, instance) in &config.providers.asr.instances {
        if id != &bindings.asr && id != &defaults.asr {
            continue;
        }
        let factory = registry.asr_factory(instance.adapter())?;
        let model = prepare(
            &config.deployment.model_manifest,
            &config.deployment.models.root,
            config.deployment.models.offline,
            factory.model_identity(instance)?,
            factory.adapter(),
            &config.deployment,
        )?;
        let max_buffered_samples = usize::try_from(config.audio.max_utterance_ms)
            .map_err(|_| {
                ProviderLoadError::Configuration("audio.max_utterance_ms is too large".into())
            })?
            .checked_mul(16)
            .ok_or_else(|| {
                ProviderLoadError::Configuration("audio.max_utterance_ms is too large".into())
            })?;
        let provider = factory.build(instance, &model, max_buffered_samples)?;
        asr_runtimes.insert(
            id.clone(),
            Arc::new(AsrWorkerRuntime::new(
                Arc::clone(&provider),
                WorkerRuntimeConfig {
                    max_workers: config.workers.asr.max_workers,
                    command_capacity: config.workers.asr.command_queue_capacity,
                    final_timeout: Duration::from_millis(config.workers.asr.final_timeout_ms),
                    cleanup_grace: Duration::from_millis(config.workers.asr.cleanup_grace_ms),
                },
            )),
        );
        tracing::info!(provider_kind = "asr", provider_instance = %id, adapter = instance.adapter(), model = %model.identity(), "provider runtime loaded");
        asr_providers.insert(id.clone(), provider);
    }

    let mut llm_providers = HashMap::new();
    let mut llm_runtimes = HashMap::new();
    for (id, instance) in &config.providers.llm.instances {
        if id != &bindings.llm && id != &defaults.llm {
            continue;
        }
        let provider = registry.llm_factory(instance.adapter())?.build(instance)?;
        llm_runtimes.insert(
            id.clone(),
            Arc::new(LlmRuntime::new(
                Arc::clone(&provider),
                config.limits.llm_concurrency,
                Duration::from_millis(instance.openai().timeout_ms),
            )),
        );
        tracing::info!(provider_kind = "llm", provider_instance = %id, adapter = instance.adapter(), "provider runtime loaded");
        llm_providers.insert(id.clone(), provider);
    }

    let mut tts_providers = HashMap::new();
    let mut tts_runtimes = HashMap::new();
    for (id, instance) in &config.providers.tts.instances {
        if id != &bindings.tts && id != &defaults.tts && !instance.preload() {
            continue;
        }
        let factory = registry.tts_factory(instance.adapter())?;
        let model = factory
            .model_identity(instance)?
            .map(|identity| {
                prepare(
                    &config.deployment.model_manifest,
                    &config.deployment.models.root,
                    config.deployment.models.offline,
                    identity,
                    factory.adapter(),
                    &config.deployment,
                )
            })
            .transpose()?;
        let provider = factory.build(instance, &config.runtime, model.as_ref())?;
        tts_runtimes.insert(
            id.clone(),
            Arc::new(TtsWorkerRuntime::new(
                Arc::clone(&provider),
                WorkerRuntimeConfig {
                    max_workers: config.workers.tts.max_workers,
                    command_capacity: config.workers.tts.command_queue_capacity,
                    final_timeout: Duration::from_millis(config.tts.timeout_ms),
                    cleanup_grace: Duration::from_millis(config.workers.tts.cleanup_grace_ms),
                },
            )),
        );
        tracing::info!(provider_kind = "tts", provider_instance = %id, adapter = instance.adapter(), "provider runtime loaded");
        tts_providers.insert(id.clone(), provider);
    }

    let mut vision_providers = HashMap::new();
    let mut vision_runtimes = HashMap::new();
    if config.vision.enabled {
        let binding = config
            .effective_agent
            .providers
            .vision
            .as_deref()
            .expect("validated Vision binding");
        for (id, instance) in &config.providers.vision.instances {
            if id != binding {
                continue;
            }
            let provider = registry
                .vision_factory(instance.adapter())?
                .build(instance)?;
            vision_runtimes.insert(
                id.clone(),
                Arc::new(VisionRuntime::new(
                    Arc::clone(&provider),
                    config.limits.vision_concurrency,
                    Duration::from_millis(instance.openai_vision().timeout_ms),
                )),
            );
            vision_providers.insert(id.clone(), provider);
        }
    }

    Ok(LoadedProviders {
        providers: ProviderCatalog {
            vad: vad_providers,
            asr: asr_providers,
            llm: llm_providers,
            tts: tts_providers,
            vision: vision_providers,
        },
        runtimes: RuntimeCatalog {
            vad: vad_runtimes,
            asr: asr_runtimes,
            llm: llm_runtimes,
            tts: tts_runtimes,
            vision: vision_runtimes,
        },
    })
}
