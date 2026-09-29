use std::{collections::HashMap, sync::Arc};

use crate::{
    audio::VadSegmenterConfig,
    config::EffectiveProviderBindings,
    workers::{AsrWorkerRuntime, LlmRuntime, TtsWorkerRuntime, VadWorkerRuntime, VisionRuntime},
};

#[derive(Debug, thiserror::Error)]
pub enum RuntimeResolveError {
    #[error("unknown {kind} runtime for provider instance `{id}`")]
    Unknown { kind: &'static str, id: String },
}

/// One loaded VAD instance. Segmentation timing belongs to the provider that produced it, so a
/// database Template binding never has to be looked up again in the deployment TOML.
#[derive(Clone)]
pub struct LoadedVad {
    pub runtime: Arc<VadWorkerRuntime>,
    pub segmenter: VadSegmenterConfig,
    pub pre_roll_samples: u64,
}

#[derive(Clone)]
pub struct ResolvedAgentRuntimes {
    pub vad: Arc<VadWorkerRuntime>,
    pub asr: Arc<AsrWorkerRuntime>,
    pub llm: Arc<LlmRuntime>,
    pub tts: Arc<TtsWorkerRuntime>,
    pub vad_segmenter: VadSegmenterConfig,
    pub vad_pre_roll_samples: u64,
}

/// Read-only runtime catalog. Provider IDs are resolved once at the session boundary.
#[derive(Clone, Default)]
pub struct RuntimeCatalog {
    pub(crate) vad: HashMap<String, LoadedVad>,
    pub(crate) asr: HashMap<String, Arc<AsrWorkerRuntime>>,
    pub(crate) llm: HashMap<String, Arc<LlmRuntime>>,
    pub(crate) tts: HashMap<String, Arc<TtsWorkerRuntime>>,
    pub(crate) vision: HashMap<String, Arc<VisionRuntime>>,
}

impl RuntimeCatalog {
    pub fn tts(&self, id: &str) -> Result<Arc<TtsWorkerRuntime>, RuntimeResolveError> {
        self.tts
            .get(id)
            .cloned()
            .ok_or_else(|| RuntimeResolveError::Unknown {
                kind: "TTS",
                id: id.into(),
            })
    }
    pub fn vision(&self, id: &str) -> Result<Arc<VisionRuntime>, RuntimeResolveError> {
        self.vision
            .get(id)
            .cloned()
            .ok_or_else(|| RuntimeResolveError::Unknown {
                kind: "VISION",
                id: id.into(),
            })
    }
    pub fn resolve(
        &self,
        bindings: &EffectiveProviderBindings,
    ) -> Result<ResolvedAgentRuntimes, RuntimeResolveError> {
        let loaded_vad =
            self.vad
                .get(&bindings.vad)
                .cloned()
                .ok_or_else(|| RuntimeResolveError::Unknown {
                    kind: "VAD",
                    id: bindings.vad.clone(),
                })?;
        let (vad_segmenter, vad_pre_roll_samples) =
            (loaded_vad.segmenter, loaded_vad.pre_roll_samples);
        let vad = loaded_vad.runtime;
        Ok(ResolvedAgentRuntimes {
            vad,
            asr: self.asr.get(&bindings.asr).cloned().ok_or_else(|| {
                RuntimeResolveError::Unknown {
                    kind: "ASR",
                    id: bindings.asr.clone(),
                }
            })?,
            llm: self.llm.get(&bindings.llm).cloned().ok_or_else(|| {
                RuntimeResolveError::Unknown {
                    kind: "LLM",
                    id: bindings.llm.clone(),
                }
            })?,
            tts: self.tts.get(&bindings.tts).cloned().ok_or_else(|| {
                RuntimeResolveError::Unknown {
                    kind: "TTS",
                    id: bindings.tts.clone(),
                }
            })?,
            vad_segmenter,
            vad_pre_roll_samples,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::EffectiveProviderBindings,
        providers::{
            asr::UnavailableAsr, llm::UnavailableLlm, tts::UnavailableTts, vad::UnavailableVad,
        },
        workers::WorkerRuntimeConfig,
    };
    use std::time::Duration;

    #[test]
    fn resolve_returns_the_runtime_snapshot_named_by_effective_bindings() {
        let worker = WorkerRuntimeConfig {
            max_workers: 1,
            command_capacity: 1,
            final_timeout: Duration::from_secs(1),
            cleanup_grace: Duration::from_secs(1),
        };
        let catalog = RuntimeCatalog {
            vad: HashMap::from([(
                "vad_a".into(),
                LoadedVad {
                    runtime: Arc::new(VadWorkerRuntime::new(
                        Arc::new(UnavailableVad),
                        worker.clone(),
                    )),
                    segmenter: VadSegmenterConfig::default(),
                    pre_roll_samples: 4_800,
                },
            )]),
            asr: HashMap::from([(
                "asr_a".into(),
                Arc::new(AsrWorkerRuntime::new(
                    Arc::new(UnavailableAsr),
                    worker.clone(),
                )),
            )]),
            llm: HashMap::from([(
                "llm_a".into(),
                Arc::new(LlmRuntime::new(
                    Arc::new(UnavailableLlm),
                    1,
                    Duration::from_secs(1),
                )),
            )]),
            tts: HashMap::from([(
                "tts_b".into(),
                Arc::new(TtsWorkerRuntime::new(Arc::new(UnavailableTts), worker)),
            )]),
            vision: HashMap::new(),
        };
        let resolved = catalog
            .resolve(&EffectiveProviderBindings {
                vad: "vad_a".into(),
                asr: "asr_a".into(),
                llm: "llm_a".into(),
                tts: "tts_b".into(),
                vision: None,
            })
            .unwrap();
        assert_eq!(resolved.tts.provider().adapter(), "unavailable");
        assert_eq!(resolved.vad_pre_roll_samples, 4_800);
    }

    #[test]
    fn a_binding_whose_vad_runtime_is_absent_cannot_admit_a_session() {
        let catalog = RuntimeCatalog::default();
        let Err(error) = catalog.resolve(&EffectiveProviderBindings {
            vad: "vad_a".into(),
            asr: "asr_a".into(),
            llm: "llm_a".into(),
            tts: "tts_b".into(),
            vision: None,
        }) else {
            panic!("an unloaded VAD runtime cannot admit a session");
        };
        assert!(matches!(
            error,
            RuntimeResolveError::Unknown { kind: "VAD", .. }
        ));
    }
}
