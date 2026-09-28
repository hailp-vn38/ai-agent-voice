use std::{collections::HashMap, sync::Arc, time::Duration};

use crate::{
    config::AppConfig,
    models::prepare,
    providers::{ProviderCatalog, ProviderLoadError, RuntimeCatalog, compiled_provider_registry},
    workers::{
        AsrWorkerRuntime, LlmRuntime, TtsWorkerRuntime, VadWorkerRuntime, VisionRuntime,
        WorkerRuntimeConfig,
    },
};

pub struct LoadedProviders {
    pub providers: ProviderCatalog,
    pub runtimes: RuntimeCatalog,
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
        vad_runtimes.insert(
            id.clone(),
            Arc::new(VadWorkerRuntime::new(
                Arc::clone(&provider),
                WorkerRuntimeConfig {
                    max_workers: config.workers.vad.max_workers,
                    command_capacity: config.workers.vad.command_queue_capacity,
                    final_timeout: Duration::from_millis(config.workers.vad.reset_timeout_ms),
                    cleanup_grace: Duration::from_millis(config.workers.vad.cleanup_grace_ms),
                },
            )),
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
