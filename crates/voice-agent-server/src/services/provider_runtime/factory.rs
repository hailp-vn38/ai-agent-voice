use super::{RuntimeError, RuntimeMaterializer, RuntimeResource};
use crate::{
    config::AppConfig,
    database::{DesiredProvider, secrets::SecretResolver},
    providers::{DatabaseRuntimeFailure, RuntimeCatalog},
    workers::{ProviderRuntimeAdmission, WorkerSupervisor},
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

/// Adapter-owned portion of a ZeroTTS physical resource. Logical provider selection deliberately
/// stays out of this identity so aliases can share one resident worker pool safely.
#[derive(serde::Serialize)]
struct ZeroTtsPhysicalSpec {
    delivery_mode: crate::config::ZeroTtsDeliveryMode,
}

/// Production bridge to the same installed-artifact factories used by deployment startup.
/// Estimates are deployment-owned worst-case peaks per adapter, including all configured
/// workers and warmup allocations. A missing estimate refuses allocation, without resolving
/// credentials. This counter does not claim to enforce an OS memory limit.
pub struct FactoryMaterializer {
    config: Arc<AppConfig>,
    secrets: Arc<dyn SecretResolver>,
    estimates: HashMap<String, u64>,
    supervisor: Arc<WorkerSupervisor>,
    qualified_manifest_fingerprint: Option<[u8; 32]>,
    onnx_fingerprint: Option<[u8; 32]>,
    kokoro_g2p_fingerprint: Option<[u8; 32]>,
}
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
        let qualified_manifest_fingerprint = if let Some(profile) = &config.provider_runtime {
            let fingerprint = manifest_fingerprint(&config.deployment.model_manifest)?;
            let hash: String = fingerprint
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            if hash != profile.measured_manifest_sha256 {
                tracing::error!(
                    reason = "manifest_receipt_mismatch",
                    measured_manifest_sha256 = %profile.measured_manifest_sha256,
                    current_manifest_sha256 = %hash,
                    "provider runtime qualification receipt is stale; remeasure the model and execution settings before updating the receipt"
                );
                return Err(RuntimeError::Configuration);
            }
            Some(fingerprint)
        } else {
            None
        };
        let native = estimates
            .keys()
            .any(|adapter| !matches!(adapter.as_str(), "openai" | "chillaudio_ws"));
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
            qualified_manifest_fingerprint,
            onnx_fingerprint,
            kokoro_g2p_fingerprint,
        })
    }
}
impl RuntimeMaterializer for FactoryMaterializer {
    fn resource_key(
        &self,
        snapshot: &DesiredProvider,
    ) -> Result<Option<super::ResourceKey>, RuntimeError> {
        self.verify_qualified_manifest()?;
        self.resource_key_with_fingerprint(snapshot, None)
    }
    fn prepare_artifacts(&self, snapshot: &DesiredProvider) -> Result<(), RuntimeError> {
        self.verify_qualified_manifest()?;
        if matches!(snapshot.adapter.as_str(), "openai" | "chillaudio_ws") {
            return Ok(());
        }
        let value: serde_json::Value = self
            .effective_config(snapshot)
            .map_err(|_| RuntimeError::Configuration)?;
        let model = value
            .get("model")
            .and_then(|v| v.as_str())
            .ok_or(RuntimeError::Configuration)?;
        crate::models::prepare_immutable(
            &self.config.deployment.model_manifest,
            &self.config.deployment.models.root,
            self.config.deployment.models.offline,
            model,
            &snapshot.adapter,
            &self.config.deployment,
        )
        .map_err(|_| RuntimeError::ArtifactsNotReady)?;
        Ok(())
    }
    fn estimated_peak_bytes(&self, snapshot: &DesiredProvider) -> Result<u64, RuntimeError> {
        self.verify_qualified_manifest()?;
        // Local requests use exactly the deployment-owned execution settings that were
        // qualified. DB configuration cannot widen a model or thread envelope.
        if !matches!(snapshot.adapter.as_str(), "openai" | "chillaudio_ws") {
            self.effective_config(snapshot)?;
            let allowed = match snapshot.kind.as_str() {
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
    fn global_capacity(&self, snapshot: &DesiredProvider) -> Result<Option<usize>, RuntimeError> {
        let capacity = match snapshot.kind.as_str() {
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
        quota: ProviderRuntimeAdmission,
    ) -> Result<Arc<dyn RuntimeResource>, RuntimeError> {
        self.verify_qualified_manifest()?;
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
        let prepared_model = if matches!(snapshot.adapter.as_str(), "openai" | "chillaudio_ws") {
            None
        } else {
            let value: serde_json::Value = self
                .effective_config(snapshot)
                .map_err(|_| RuntimeError::Configuration)?;
            let model = value
                .get("model")
                .and_then(|v| v.as_str())
                .ok_or(RuntimeError::Configuration)?;
            Some(
                crate::models::prepare_immutable(
                    &self.config.deployment.model_manifest,
                    &self.config.deployment.models.root,
                    self.config.deployment.models.offline,
                    model,
                    &snapshot.adapter,
                    &self.config.deployment,
                )
                .map_err(|_| RuntimeError::ArtifactsNotReady)?,
            )
        };
        let resource_key = self.resource_key_with_fingerprint(
            snapshot,
            prepared_model.as_ref().map(|model| model.fingerprint()),
        )?;
        let catalog = crate::providers::materialize_provider_from_artifacts(
            &self.config,
            snapshot,
            self.secrets.as_ref(),
            quota.clone(),
            prepared_model.as_ref(),
        )
        .map_err(|failure| match failure {
            DatabaseRuntimeFailure::Configuration => RuntimeError::Configuration,
            DatabaseRuntimeFailure::Quarantined => RuntimeError::Quarantined,
            DatabaseRuntimeFailure::Secret | DatabaseRuntimeFailure::Runtime => {
                RuntimeError::Unavailable
            }
        })?;
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
            .collect();
        Ok(Arc::new(OwnedRuntimeResource {
            resource_key,
            physical_admission: quota,
            readiness,
            health_flags,
            catalog: Mutex::new(Some(catalog)),
            capabilities,
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
        if let Some(runtime) = resident.asr.values().next() {
            view.asr.insert(
                snapshot.key.clone(),
                Arc::new(runtime.logical_view(quota.clone())),
            );
        }
        if let Some(runtime) = resident.vad.values().next() {
            let mut runtime = runtime.clone();
            runtime.runtime = Arc::new(runtime.runtime.logical_view(quota.clone()));
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

impl FactoryMaterializer {
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
    fn verify_qualified_manifest(&self) -> Result<(), RuntimeError> {
        if let Some(expected) = self.qualified_manifest_fingerprint
            && manifest_fingerprint(&self.config.deployment.model_manifest)? != expected
        {
            return Err(RuntimeError::Configuration);
        }
        Ok(())
    }

    fn resource_key_with_fingerprint(
        &self,
        snapshot: &DesiredProvider,
        installed_fingerprint: Option<&str>,
    ) -> Result<Option<super::ResourceKey>, RuntimeError> {
        // Authenticated clients are isolated until the resolver provides credential generations.
        if snapshot.secret_ref.is_some()
            || matches!(snapshot.adapter.as_str(), "openai" | "chillaudio_ws")
        {
            return Ok(None);
        }
        let value: serde_json::Value = self
            .effective_config(snapshot)
            .map_err(|_| RuntimeError::Configuration)?;
        let model = value
            .get("model")
            .and_then(|v| v.as_str())
            .ok_or(RuntimeError::Configuration)?;
        let fingerprint = match installed_fingerprint {
            Some(fingerprint) => fingerprint.to_owned(),
            None => crate::models::model_fingerprint(
                &self.config.deployment.model_manifest,
                model,
                &snapshot.adapter,
            )
            .map_err(|_| RuntimeError::ArtifactsNotReady)?,
        };
        use sha2::{Digest, Sha256};
        let specification = match snapshot.adapter.as_str() {
            "silero_onnx" => serde_json::to_value(
                serde_json::from_value::<crate::config::SileroOnnxConfig>(value)
                    .map_err(|_| RuntimeError::Configuration)?,
            ),
            "zipformer_sherpa" => serde_json::to_value(
                serde_json::from_value::<crate::config::ZipformerSherpaConfig>(value)
                    .map_err(|_| RuntimeError::Configuration)?,
            ),
            "gipformer_sherpa_offline" => serde_json::to_value(
                serde_json::from_value::<crate::config::GipformerSherpaOfflineConfig>(value)
                    .map_err(|_| RuntimeError::Configuration)?,
            ),
            "zerotts_onnx" => {
                let configuration: crate::config::ZeroTtsOnnxConfig =
                    serde_json::from_value(value).map_err(|_| RuntimeError::Configuration)?;
                zerotts_binding(&configuration)?;
                serde_json::to_value(ZeroTtsPhysicalSpec {
                    delivery_mode: configuration.delivery_mode,
                })
            }
            "kokoro_vi_onnx" => {
                let mut specification: crate::config::KokoroViOnnxConfig =
                    serde_json::from_value(value).map_err(|_| RuntimeError::Configuration)?;
                specification.preload = false;
                serde_json::to_value(specification)
            }
            _ => return Ok(None),
        }
        .map_err(|_| RuntimeError::Configuration)?;
        let (onnx, g2p) = if installed_fingerprint.is_some() {
            (
                Some(execution_file_fingerprint(
                    &self.config.runtime.onnx.library,
                    512 * 1024 * 1024,
                )?),
                (snapshot.adapter == "kokoro_vi_onnx")
                    .then(|| kokoro_g2p_fingerprint(&self.config.runtime.kokoro_vi.g2p_executable))
                    .transpose()?,
            )
        } else {
            (
                self.onnx_fingerprint,
                (snapshot.adapter == "kokoro_vi_onnx")
                    .then_some(self.kokoro_g2p_fingerprint)
                    .flatten(),
            )
        };
        let onnx = onnx.ok_or(RuntimeError::Configuration)?;
        let mut digest = Sha256::new();
        digest.update(serde_json::to_vec(&serde_json::json!({"adapter":snapshot.adapter,"specification":specification,"execution_threads":self.config.runtime.onnx.threads_for(&snapshot.adapter),"artifacts":fingerprint,"capacity":self.logical_capacity(snapshot)?,"onnx_execution":onnx,"g2p_execution":g2p})).map_err(|_| RuntimeError::Configuration)?);
        Ok(Some(super::ResourceKey(digest.finalize().into())))
    }
}

fn manifest_fingerprint(path: &std::path::Path) -> Result<[u8; 32], RuntimeError> {
    execution_file_fingerprint(path, 2 * 1024 * 1024)
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
