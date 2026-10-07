use std::{collections::HashMap, sync::Arc};

use super::ProviderType;
use crate::{
    audio::VadSegmenterConfig,
    config::EffectiveProviderBindings,
    workers::{
        AsrDiagnosticOperation, AsrWorkerRuntime, LlmDiagnosticOperation, LlmRuntime,
        ProviderAdmissionError, ProviderCapacityPermit, TtsDiagnosticOperation, TtsWorkerRuntime,
        VadDiagnosticOperation, VadWorkerRuntime, VisionRuntime,
    },
};

#[derive(Debug, thiserror::Error)]
pub enum RuntimeResolveError {
    #[error("unknown {kind} runtime for provider instance `{id}`")]
    Unknown { kind: &'static str, id: String },
}

/// A diagnostic can only target runtime classes that own bounded provider capacity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagnosticRuntimeKind {
    Vad,
    Asr,
    Llm,
    Tts,
}

impl TryFrom<ProviderType> for DiagnosticRuntimeKind {
    type Error = ();

    fn try_from(value: ProviderType) -> Result<Self, Self::Error> {
        match value {
            ProviderType::Vad => Ok(Self::Vad),
            ProviderType::Asr => Ok(Self::Asr),
            ProviderType::Llm => Ok(Self::Llm),
            ProviderType::Tts => Ok(Self::Tts),
            ProviderType::Speaker => Err(()),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DiagnosticRuntimeError {
    #[error("provider runtime is not loaded")]
    NotLoaded,
    #[error("provider diagnostic capacity is exhausted")]
    Capacity,
}

#[derive(Debug, thiserror::Error)]
pub enum TtsDiagnosticValidationError {
    #[error("provider runtime is not loaded")]
    NotLoaded,
    #[error("TTS diagnostic input is not supported by the loaded runtime")]
    InvalidInput,
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
    pub(crate) speaker: HashMap<String, Arc<super::speaker::SpeakerRuntime>>,
    pub(crate) vision: HashMap<String, Arc<VisionRuntime>>,
}

impl RuntimeCatalog {
    pub fn single_speaker(key: String, runtime: Arc<super::speaker::SpeakerRuntime>) -> Self {
        let mut catalog = Self::default();
        catalog.speaker.insert(key, runtime);
        catalog
    }
    pub fn speaker(&self, key: &str) -> Option<Arc<super::speaker::SpeakerRuntime>> {
        self.speaker.get(key).cloned()
    }

    /// Publishes one materializer-owned slot under its immutable provider key.
    /// The caller must retain the resource lease alongside every resolved handle.
    pub fn single_provider(
        kind: DiagnosticRuntimeKind,
        key: String,
        runtime: &ResolvedAgentRuntimes,
    ) -> Self {
        let mut catalog = Self::default();
        match kind {
            DiagnosticRuntimeKind::Vad => {
                catalog.vad.insert(
                    key,
                    LoadedVad {
                        runtime: Arc::clone(&runtime.vad),
                        segmenter: runtime.vad_segmenter,
                        pre_roll_samples: runtime.vad_pre_roll_samples,
                    },
                );
            }
            DiagnosticRuntimeKind::Asr => {
                catalog.asr.insert(key, Arc::clone(&runtime.asr));
            }
            DiagnosticRuntimeKind::Llm => {
                catalog.llm.insert(key, Arc::clone(&runtime.llm));
            }
            DiagnosticRuntimeKind::Tts => {
                catalog.tts.insert(key, Arc::clone(&runtime.tts));
            }
        }
        catalog
    }
    /// Called outside the manager lock after lease admission has closed. Every worker
    /// owns its native exit acknowledgement; no Arc reference-count heuristic is used.
    pub(crate) fn shutdown_acknowledged(&self) -> bool {
        let mut acknowledged = self.vision.is_empty();
        for runtime in self.speaker.values() {
            acknowledged &= runtime.shutdown_acknowledged();
        }
        for runtime in self.vad.values() {
            acknowledged &= runtime.runtime.shutdown_acknowledged();
        }
        for runtime in self.asr.values() {
            acknowledged &= runtime.shutdown_acknowledged();
        }
        for runtime in self.llm.values() {
            acknowledged &= runtime.shutdown_acknowledged();
        }
        for runtime in self.tts.values() {
            acknowledged &= runtime.shutdown_acknowledged();
        }
        acknowledged
    }
    /// Atomically admits diagnostic work at the runtime materialized during startup.
    pub fn admit_diagnostic(
        &self,
        kind: DiagnosticRuntimeKind,
        key: &str,
    ) -> Result<ProviderCapacityPermit, DiagnosticRuntimeError> {
        match kind {
            DiagnosticRuntimeKind::Vad => self
                .vad
                .get(key)
                .ok_or(DiagnosticRuntimeError::NotLoaded)
                .and_then(|loaded| {
                    loaded
                        .runtime
                        .admit_diagnostic()
                        .map_err(map_admission_error)
                }),
            DiagnosticRuntimeKind::Asr => self
                .asr
                .get(key)
                .ok_or(DiagnosticRuntimeError::NotLoaded)
                .and_then(|runtime| runtime.admit_diagnostic().map_err(map_admission_error)),
            DiagnosticRuntimeKind::Llm => self
                .llm
                .get(key)
                .ok_or(DiagnosticRuntimeError::NotLoaded)
                .and_then(|runtime| runtime.admit_diagnostic().map_err(map_admission_error)),
            DiagnosticRuntimeKind::Tts => self
                .tts
                .get(key)
                .ok_or(DiagnosticRuntimeError::NotLoaded)
                .and_then(|runtime| runtime.admit_diagnostic().map_err(map_admission_error)),
        }
    }
    pub fn tts(&self, id: &str) -> Result<Arc<TtsWorkerRuntime>, RuntimeResolveError> {
        self.tts
            .get(id)
            .cloned()
            .ok_or_else(|| RuntimeResolveError::Unknown {
                kind: "TTS",
                id: id.into(),
            })
    }
    /// Builds a diagnostic request for an already-loaded LLM runtime without admitting capacity
    /// or altering the catalog. Admission remains atomic at `admit_diagnostic`.
    pub fn llm_diagnostic(
        &self,
        key: &str,
        request: crate::providers::llm::LlmRequest,
        max_text_bytes: usize,
    ) -> Result<LlmDiagnosticOperation, DiagnosticRuntimeError> {
        self.llm
            .get(key)
            .map(|runtime| runtime.diagnostic(request, max_text_bytes))
            .ok_or(DiagnosticRuntimeError::NotLoaded)
    }
    pub fn asr_diagnostic(
        &self,
        key: &str,
        pcm: crate::audio::PcmF32Mono,
    ) -> Result<AsrDiagnosticOperation, DiagnosticRuntimeError> {
        self.asr
            .get(key)
            .map(|runtime| runtime.diagnostic(pcm))
            .ok_or(DiagnosticRuntimeError::NotLoaded)
    }
    pub fn vad_diagnostic(
        &self,
        key: &str,
    ) -> Result<VadDiagnosticOperation, DiagnosticRuntimeError> {
        self.vad
            .get(key)
            .map(|loaded| loaded.runtime.diagnostic())
            .ok_or(DiagnosticRuntimeError::NotLoaded)
    }
    pub fn tts_diagnostic(
        &self,
        key: &str,
        request: crate::providers::TtsDiagnosticRequest,
    ) -> Result<TtsDiagnosticOperation, DiagnosticRuntimeError> {
        self.tts
            .get(key)
            .map(|runtime| runtime.diagnostic(request))
            .ok_or(DiagnosticRuntimeError::NotLoaded)
    }
    pub fn validate_tts_diagnostic(
        &self,
        key: &str,
        request: &crate::providers::TtsDiagnosticRequest,
    ) -> Result<(), TtsDiagnosticValidationError> {
        self.tts
            .get(key)
            .ok_or(TtsDiagnosticValidationError::NotLoaded)?
            .validate_diagnostic(request)
            .map_err(|_| TtsDiagnosticValidationError::InvalidInput)
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

fn map_admission_error(_: ProviderAdmissionError) -> DiagnosticRuntimeError {
    DiagnosticRuntimeError::Capacity
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
            voice_reserved_capacity: 1,
            command_capacity: 1,
            final_timeout: Duration::from_secs(1),
            cleanup_grace: Duration::from_secs(1),
        };
        let catalog = RuntimeCatalog {
            speaker: HashMap::new(),
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
