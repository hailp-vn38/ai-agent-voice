use super::{
    MaterializationTimings, PreparedRuntime, RuntimeError, RuntimeMaterializer, RuntimeResource,
};
use crate::{
    config::AppConfig,
    database::{DesiredProvider, secrets::SecretResolver},
    providers::{DatabaseRuntimeFailure, RuntimeCatalog},
    workers::{ProviderRuntimeAdmission, WorkerSupervisor},
};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

/// Cumulative materialization counters for one `FactoryMaterializer`. They expose how often native
/// state was actually rebuilt, which is the only reliable way to prove that a logical change (voice,
/// template, agent) did not rematerialize a shared physical runtime. Labels are fixed-cardinality.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FactoryDiagnostics {
    /// Native resources constructed by `build`.
    pub builds: u64,
    /// Materializations whose provider model files were ensured before building.
    pub asset_ensures: u64,
}

pub struct FactoryMaterializer {
    config: Arc<AppConfig>,
    secrets: Arc<dyn SecretResolver>,
    estimates: HashMap<String, u64>,
    supervisor: Arc<WorkerSupervisor>,
    onnx_fingerprint: Option<[u8; 32]>,
    kokoro_g2p_fingerprint: Option<[u8; 32]>,
    builds: AtomicU64,
    asset_ensures: AtomicU64,
}

/// Production bridge to the same installed-artifact factories used by deployment startup.
/// Estimates are deployment-owned worst-case peaks per adapter, including every physical plan
/// that adapter may materialize, its configured workers, and warmup allocations. The current
/// deployment schema intentionally budgets one conservative ceiling per adapter rather than an
/// unqualified per-plan value. A missing estimate refuses allocation, without resolving
/// credentials. This counter does not claim to enforce an OS memory limit.
impl FactoryMaterializer {
    pub fn new(
        config: Arc<AppConfig>,
        secrets: Arc<dyn SecretResolver>,
        estimates: HashMap<String, u64>,
        supervisor: Arc<WorkerSupervisor>,
    ) -> Result<Self, RuntimeError> {
        if estimates.is_empty()
            || estimates.len() > 64
            || estimates.values().any(|bytes| *bytes == 0)
        {
            return Err(RuntimeError::Configuration);
        }
        let native = estimates.keys().any(|adapter| {
            crate::providers::compiled_provider_adapter_registry()
                .assets(adapter)
                .is_some()
        });
        let kokoro = estimates.contains_key("kokoro_vi_onnx");
        let onnx_fingerprint = native
            .then(|| execution_file_fingerprint(&config.runtime.onnx.library, 512 * 1024 * 1024))
            .transpose()?;
        let kokoro_g2p_fingerprint = kokoro
            .then(|| kokoro_g2p_fingerprint(&config.runtime.kokoro_vi.g2p_executable))
            .transpose()?;
        Ok(Self {
            config,
            secrets,
            estimates,
            supervisor,
            onnx_fingerprint,
            kokoro_g2p_fingerprint,
            builds: AtomicU64::new(0),
            asset_ensures: AtomicU64::new(0),
        })
    }

    /// Cumulative materialization counters for this instance. Labels are fixed-cardinality.
    pub fn diagnostics(&self) -> FactoryDiagnostics {
        FactoryDiagnostics {
            builds: self.builds.load(Ordering::Relaxed),
            asset_ensures: self.asset_ensures.load(Ordering::Relaxed),
        }
    }
}
impl RuntimeMaterializer for FactoryMaterializer {
    fn resource_key(
        &self,
        snapshot: &DesiredProvider,
    ) -> Result<Option<super::ResourceKey>, RuntimeError> {
        self.resource_key_for(snapshot)
    }

    /// Downloads whatever the provider is missing, then reports only what preparation established.
    /// Which files those are belongs to the provider's own asset manager, so this never matches on
    /// an adapter name.
    fn prepare_artifacts(
        &self,
        snapshot: &DesiredProvider,
    ) -> Result<Option<PreparedRuntime>, RuntimeError> {
        if self.local_runtime_plan(snapshot)?.is_none() {
            return Ok(Some(PreparedRuntime::Remote));
        }
        let Some(assets) =
            crate::providers::compiled_provider_adapter_registry().assets(&snapshot.adapter)
        else {
            return Err(RuntimeError::Configuration);
        };
        let started = Instant::now();
        assets
            .ensure_assets()
            .map_err(|_| RuntimeError::ArtifactsNotReady)?;
        self.asset_ensures.fetch_add(1, Ordering::Relaxed);
        Ok(Some(PreparedRuntime::Local {
            ensure: started.elapsed(),
        }))
    }

    fn estimated_peak_bytes(&self, snapshot: &DesiredProvider) -> Result<u64, RuntimeError> {
        // Local requests use exactly the deployment-owned execution settings that were
        // qualified. DB configuration cannot widen a model or thread envelope.
        if self.local_runtime_plan(snapshot)?.is_some() {
            let allowed = match snapshot.kind.as_str() {
                "speaker" => snapshot.adapter == "campplus_sherpa",
                "vad" => self
                    .config
                    .providers
                    .vad
                    .instances
                    .values()
                    .any(|instance| instance.adapter() == snapshot.adapter),
                "asr" => self
                    .config
                    .providers
                    .asr
                    .instances
                    .values()
                    .any(|instance| instance.adapter() == snapshot.adapter),
                "tts" => self
                    .config
                    .providers
                    .tts
                    .instances
                    .values()
                    .any(|instance| instance.adapter() == snapshot.adapter),
                _ => false,
            };
            if !allowed
                || !(1..=128).contains(&self.config.runtime.onnx.threads_for(&snapshot.adapter))
            {
                return Err(RuntimeError::Configuration);
            }
        }
        self.estimates
            .get(&snapshot.adapter)
            .copied()
            .ok_or(RuntimeError::Configuration)
    }
    fn logical_capacity(&self, snapshot: &DesiredProvider) -> Result<usize, RuntimeError> {
        let capacity = match snapshot.kind.as_str() {
            "speaker" => 1,
            "vad" => self.config.workers.vad.max_workers,
            "asr" => self.config.workers.asr.max_workers,
            "llm" => self.config.limits.llm_concurrency,
            "tts" => self.config.workers.tts.max_workers,
            _ => return Err(RuntimeError::Configuration),
        };
        if capacity == 0 {
            return Err(RuntimeError::Configuration);
        }
        Ok(capacity)
    }
    fn physical_capacity(&self, snapshot: &DesiredProvider) -> Result<usize, RuntimeError> {
        let Some(plan) = self.local_runtime_plan(snapshot)? else {
            return self.logical_capacity(snapshot);
        };
        let logical = self.logical_capacity(snapshot)?;
        Ok(plan.physical_capacity().resolve(logical))
    }
    fn global_capacity(&self, snapshot: &DesiredProvider) -> Result<Option<usize>, RuntimeError> {
        let capacity = match snapshot.kind.as_str() {
            "speaker" => 1,
            "vad" => self.config.workers.vad.max_workers,
            "asr" => self.config.workers.asr.max_workers,
            "llm" => self.config.limits.llm_concurrency,
            "tts" => self.config.limits.tts_concurrency,
            _ => return Err(RuntimeError::Configuration),
        };
        Ok(Some(capacity))
    }
    fn build(
        &self,
        snapshot: &DesiredProvider,
        prepared: Option<PreparedRuntime>,
        quota: ProviderRuntimeAdmission,
    ) -> Result<Arc<dyn RuntimeResource>, RuntimeError> {
        let started = Instant::now();
        if snapshot.id == 0
            && crate::providers::deployment_provider_snapshot(
                &self.config,
                &snapshot.kind,
                &snapshot.key,
            )
            .map_err(|_| RuntimeError::Configuration)?
                != *snapshot
        {
            return Err(RuntimeError::Configuration);
        }
        // Preparation has already ensured the provider's files. `build` only resolves their paths,
        // so a materialization never downloads or re-verifies the same model twice.
        let mut timings = MaterializationTimings::default();
        if let Some(PreparedRuntime::Local { ensure }) = prepared {
            timings.artifact_prepare = ensure;
        }
        let resource_key = self.resource_key_for(snapshot)?;
        let physical_capacity = self.physical_capacity(snapshot)?;
        let contract_started = Instant::now();
        let catalog = crate::providers::materialize_provider_from_artifacts(
            &self.config,
            snapshot,
            self.secrets.as_ref(),
            quota.clone(),
            physical_capacity,
        )
        .map_err(|failure| match failure {
            DatabaseRuntimeFailure::Configuration => RuntimeError::Configuration,
            DatabaseRuntimeFailure::Quarantined => RuntimeError::Quarantined,
            DatabaseRuntimeFailure::Secret | DatabaseRuntimeFailure::Runtime => {
                RuntimeError::Unavailable
            }
        })?;
        timings.provider_contract = contract_started.elapsed();
        for runtime in catalog.asr.values() {
            self.supervisor.observe_asr(Arc::clone(runtime));
        }
        for runtime in catalog.vad.values() {
            self.supervisor.observe_vad(Arc::clone(&runtime.runtime));
        }
        let capabilities = crate::providers::compiled_provider_adapter_registry()
            .get(&snapshot.adapter)
            .and_then(|descriptor| serde_json::to_value(&descriptor.capabilities).ok());
        let mut readiness = crate::workers::NativeReadiness::default();
        for runtime in catalog.asr.values() {
            readiness.add(runtime.readiness());
        }
        for runtime in catalog.vad.values() {
            readiness.add(runtime.runtime.readiness());
        }
        for runtime in catalog.tts.values() {
            readiness.add(runtime.readiness());
        }
        let health_flags = catalog
            .asr
            .values()
            .flat_map(|r| r.health_flags())
            .chain(catalog.vad.values().flat_map(|r| r.runtime.health_flags()))
            .chain(catalog.tts.values().flat_map(|r| r.health_flags()))
            .chain(catalog.speaker.values().flat_map(|r| r.health_flags()))
            .collect();
        self.builds.fetch_add(1, Ordering::Relaxed);
        tracing::info!(
            adapter = %snapshot.adapter,
            kind = %snapshot.kind,
            artifact_prepare_ms = timings.artifact_prepare.as_millis(),
            artifact_verify_ms = timings.artifact_verify.as_millis(),
            provider_contract_ms = timings.provider_contract.as_millis(),
            worker_session_init_ms = readiness.initialization.as_millis(),
            worker_warmup_ms = readiness.warmup.as_millis(),
            runtime_total_ms = started.elapsed().as_millis(),
            physical_replicas = physical_capacity,
            "provider runtime materialized"
        );
        Ok(Arc::new(OwnedRuntimeResource {
            resource_key,
            physical_admission: quota,
            readiness,
            health_flags,
            catalog: Mutex::new(Some(catalog)),
            capabilities,
            timings,
        }))
    }
}
struct OwnedRuntimeResource {
    readiness: crate::workers::NativeReadiness,
    physical_admission: ProviderRuntimeAdmission,
    resource_key: Option<super::ResourceKey>,
    health_flags: Vec<Arc<std::sync::atomic::AtomicBool>>,
    catalog: Mutex<Option<RuntimeCatalog>>,
    capabilities: Option<serde_json::Value>,
    timings: MaterializationTimings,
}
impl RuntimeResource for OwnedRuntimeResource {
    fn resource_key(&self) -> Option<super::ResourceKey> {
        self.resource_key.clone()
    }
    fn physical_admission(&self) -> Option<ProviderRuntimeAdmission> {
        Some(self.physical_admission.clone())
    }
    fn readiness(&self) -> crate::workers::NativeReadiness {
        self.readiness
    }
    fn health_flags(&self) -> Vec<Arc<std::sync::atomic::AtomicBool>> {
        self.health_flags.clone()
    }
    fn capabilities(&self) -> Option<serde_json::Value> {
        self.capabilities.clone()
    }
    fn materialization_timings(&self) -> MaterializationTimings {
        self.timings
    }
    fn unload(&self) -> bool {
        let mut catalog = self.catalog.lock().expect("runtime resource poisoned");
        let Some(resident) = catalog.as_ref() else {
            return true;
        };
        if !resident.shutdown_acknowledged() {
            return false;
        }
        // Native provider graphs are destroyed before the manager receives acknowledgement
        // and returns its memory reservation. The manager registry lock is not held here.
        drop(catalog.take());
        true
    }
    fn runtimes_for(
        &self,
        snapshot: &DesiredProvider,
        quota: ProviderRuntimeAdmission,
    ) -> Option<RuntimeCatalog> {
        let resident = self.catalog.lock().expect("runtime resource poisoned");
        let resident = resident.as_ref()?;
        let mut view = RuntimeCatalog::default();
        if let Some(runtime) = resident.speaker.values().next() {
            view.speaker.insert(
                snapshot.key.clone(),
                Arc::new(runtime.logical_view(quota.clone())),
            );
        }
        if let Some(runtime) = resident.asr.values().next() {
            view.asr.insert(
                snapshot.key.clone(),
                Arc::new(runtime.logical_view(quota.clone())),
            );
        }
        if let Some(runtime) = resident.vad.values().next() {
            let mut runtime = runtime.clone();
            runtime.runtime = Arc::new(runtime.runtime.logical_view(quota.clone()));
            let (segmenter, pre_roll_samples) = vad_timing(snapshot)?;
            runtime.segmenter = segmenter;
            runtime.pre_roll_samples = pre_roll_samples;
            view.vad.insert(snapshot.key.clone(), runtime);
        }
        if let Some(runtime) = resident.tts.values().next() {
            let binding = tts_binding(snapshot)?;
            view.tts.insert(
                snapshot.key.clone(),
                Arc::new(runtime.logical_view(quota.clone(), binding)),
            );
        }
        if let Some(runtime) = resident.llm.values().next() {
            view.llm
                .insert(snapshot.key.clone(), Arc::new(runtime.logical_view(quota)));
        }
        Some(view)
    }
    fn runtimes(&self) -> Option<RuntimeCatalog> {
        self.catalog
            .lock()
            .expect("runtime resource poisoned")
            .clone()
    }
}

fn zerotts_binding(
    configuration: &crate::config::ZeroTtsOnnxConfig,
) -> Result<crate::providers::TtsBinding, RuntimeError> {
    let voice = configuration.voice.trim();
    let language = configuration.language.trim();
    let supported = crate::providers::tts::zerotts::descriptor::DESCRIPTOR
        .capabilities
        .voices
        .is_some_and(|voices| voices.iter().any(|candidate| candidate.id == voice));
    if voice.is_empty() || language != "vi-VN" || !supported {
        return Err(RuntimeError::Configuration);
    }
    Ok(crate::providers::TtsBinding {
        voice: voice.into(),
        language: language.into(),
    })
}

fn tts_binding(snapshot: &DesiredProvider) -> Option<crate::providers::TtsBinding> {
    if snapshot.adapter != "zerotts_onnx" {
        return Some(crate::providers::TtsBinding::readiness());
    }
    let configuration = serde_json::from_str(&snapshot.config_json).ok()?;
    zerotts_binding(&configuration).ok()
}

fn vad_timing(snapshot: &DesiredProvider) -> Option<(crate::audio::VadSegmenterConfig, u64)> {
    if snapshot.adapter != "silero_onnx" {
        return None;
    }
    let configuration =
        serde_json::from_str::<crate::config::SileroOnnxConfig>(&snapshot.config_json).ok()?;
    Some(crate::providers::vad_timing(
        &crate::config::VadInstanceConfig::SileroOnnx(configuration),
    ))
}

impl FactoryMaterializer {
    fn local_runtime_plan(
        &self,
        snapshot: &DesiredProvider,
    ) -> Result<Option<super::LocalRuntimePlan>, RuntimeError> {
        let registry = crate::providers::compiled_local_runtime_adapter_registry();
        let Some(adapter) = registry.get(&snapshot.adapter) else {
            return Ok(None);
        };
        let effective = self.effective_config(snapshot)?;
        adapter
            .physical_plan(effective)
            .map(Some)
            .map_err(|_| RuntimeError::Configuration)
    }

    fn effective_config(
        &self,
        snapshot: &DesiredProvider,
    ) -> Result<serde_json::Value, RuntimeError> {
        let raw = if snapshot.id == 0 {
            snapshot.config_json.clone()
        } else {
            crate::database::provider_config::validate_raw(&snapshot.adapter, &snapshot.config_json)
                .map_err(|_| RuntimeError::Configuration)?
        };
        let value = serde_json::from_str(&raw).map_err(|_| RuntimeError::Configuration)?;
        crate::providers::effective_local_config(&snapshot.adapter, value, &self.config.runtime)
            .map_err(|_| RuntimeError::Configuration)
    }
    /// Physical identity of the runtime this provider would build.
    ///
    /// Derived from the adapter's pinned model revision and its execution settings, never from
    /// hashing model files: two deployments on the same revision share a runtime, and a revision
    /// bump produces a different key without anything having to read the model.
    fn resource_key_for(
        &self,
        snapshot: &DesiredProvider,
    ) -> Result<Option<super::ResourceKey>, RuntimeError> {
        // Authenticated clients are isolated until the resolver provides credential generations.
        if snapshot.secret_ref.is_some() {
            return Ok(None);
        }
        let Some(plan) = self.local_runtime_plan(snapshot)? else {
            return Ok(None);
        };
        let revision = crate::providers::compiled_provider_adapter_registry()
            .assets(&snapshot.adapter)
            .ok_or(RuntimeError::Configuration)?
            .revision();
        // Execution binaries are immutable for the lifetime of this process, so their identities
        // were captured once at construction. Re-hashing them per build would read the same
        // unchanged bytes again; replacing a runtime binary requires a server restart.
        let onnx = self.onnx_fingerprint.ok_or(RuntimeError::Configuration)?;
        let g2p = plan
            .requires_kokoro_g2p()
            .then_some(self.kokoro_g2p_fingerprint)
            .flatten();
        let threads = usize::try_from(self.config.runtime.onnx.threads_for(&snapshot.adapter))
            .map_err(|_| RuntimeError::Configuration)?;
        let capacity = self.logical_capacity(snapshot)?;
        plan.resource_key(revision, onnx, g2p, threads, capacity)
            .map(Some)
            .map_err(|_| RuntimeError::Configuration)
    }
}

fn execution_file_fingerprint(
    path: &std::path::Path,
    max_bytes: u64,
) -> Result<[u8; 32], RuntimeError> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let metadata = std::fs::metadata(path).map_err(|_| RuntimeError::Configuration)?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > max_bytes {
        return Err(RuntimeError::Configuration);
    }
    let mut file = std::fs::File::open(path).map_err(|_| RuntimeError::Configuration)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| RuntimeError::Configuration)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(digest.finalize().into())
}

fn kokoro_g2p_fingerprint(path: &std::path::Path) -> Result<[u8; 32], RuntimeError> {
    use sha2::{Digest, Sha256};
    let executable = execution_file_fingerprint(path, 2 * 1024 * 1024)?;
    let worker = path
        .parent()
        .ok_or(RuntimeError::Configuration)?
        .join("kokoro_vi_g2p_worker.py");
    let worker = execution_file_fingerprint(&worker, 2 * 1024 * 1024)?;
    let mut digest = Sha256::new();
    digest.update(executable);
    digest.update(worker);
    Ok(digest.finalize().into())
}

#[cfg(test)]
mod tests;
