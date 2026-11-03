use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
pub use voice_agent_server::{
    database::DesiredProvider,
    lifecycle::AdmissionGate,
    services::provider_runtime::{
        PreparedRuntime, ProviderRuntimeManager, ProviderVersion, RuntimeError, RuntimeLimits,
        RuntimeMaterializer, RuntimeResource,
    },
};

pub struct Resource;
impl RuntimeResource for Resource {
    fn unload(&self) -> bool {
        true
    }
}

pub struct Builder(pub Arc<AtomicUsize>);
impl RuntimeMaterializer for Builder {
    fn estimated_peak_bytes(&self, _: &DesiredProvider) -> Result<u64, RuntimeError> {
        Ok(10)
    }
    fn logical_capacity(&self, _: &DesiredProvider) -> Result<usize, RuntimeError> {
        Ok(2)
    }
    fn build(
        &self,
        _: &DesiredProvider,
        _: Option<PreparedRuntime>,
        _: voice_agent_server::workers::ProviderRuntimeAdmission,
    ) -> Result<Arc<dyn RuntimeResource>, RuntimeError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(Arc::new(Resource))
    }
}

pub fn snapshot(revision: i64) -> DesiredProvider {
    DesiredProvider {
        id: 1,
        key: "llm".into(),
        kind: "llm".into(),
        adapter: "openai".into(),
        config_json: "{}".into(),
        secret_ref: None,
        revision,
    }
}

pub fn manager(count: Arc<AtomicUsize>) -> Arc<ProviderRuntimeManager> {
    ProviderRuntimeManager::new(
        RuntimeLimits {
            max_parallel_loads: 1,
            max_pending_loads: 4,
            max_waiters: 16,
            max_resident_bytes: 40,
            max_resources: 4,
            max_version_entries: 8,
            admission_timeout_ms: 1000,
            failure_cooldown_ms: 100,
            idle_ttl_ms: 1000,
        },
        Arc::new(Builder(count)),
        AdmissionGate::open(),
    )
    .unwrap()
}

pub struct BlockingBuilder {
    pub entered: tokio::sync::mpsc::UnboundedSender<()>,
    pub release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
}
impl RuntimeMaterializer for BlockingBuilder {
    fn estimated_peak_bytes(&self, _: &DesiredProvider) -> Result<u64, RuntimeError> {
        Ok(40)
    }
    fn logical_capacity(&self, _: &DesiredProvider) -> Result<usize, RuntimeError> {
        Ok(2)
    }
    fn build(
        &self,
        _: &DesiredProvider,
        _: Option<PreparedRuntime>,
        _: voice_agent_server::workers::ProviderRuntimeAdmission,
    ) -> Result<Arc<dyn RuntimeResource>, RuntimeError> {
        self.entered.send(()).unwrap();
        self.release.lock().unwrap().recv().unwrap();
        Ok(Arc::new(Resource))
    }
}

pub struct FailureBuilder(pub Arc<AtomicUsize>, pub RuntimeError);
impl RuntimeMaterializer for FailureBuilder {
    fn estimated_peak_bytes(&self, _: &DesiredProvider) -> Result<u64, RuntimeError> {
        Ok(10)
    }
    fn logical_capacity(&self, _: &DesiredProvider) -> Result<usize, RuntimeError> {
        Ok(2)
    }
    fn build(
        &self,
        _: &DesiredProvider,
        _: Option<PreparedRuntime>,
        _: voice_agent_server::workers::ProviderRuntimeAdmission,
    ) -> Result<Arc<dyn RuntimeResource>, RuntimeError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(self.1)
    }
}

pub fn failure_manager(
    count: Arc<AtomicUsize>,
    failure: RuntimeError,
) -> Arc<ProviderRuntimeManager> {
    ProviderRuntimeManager::new(
        RuntimeLimits {
            max_parallel_loads: 1,
            max_pending_loads: 1,
            max_waiters: 2,
            max_resident_bytes: 20,
            max_resources: 2,
            max_version_entries: 2,
            admission_timeout_ms: 1000,
            failure_cooldown_ms: 100,
            idle_ttl_ms: 1000,
        },
        Arc::new(FailureBuilder(count, failure)),
        AdmissionGate::open(),
    )
    .unwrap()
}

pub struct QuotaBuilder(
    pub std::sync::Mutex<Vec<voice_agent_server::workers::ProviderRuntimeAdmission>>,
);
impl RuntimeMaterializer for QuotaBuilder {
    fn estimated_peak_bytes(&self, _: &DesiredProvider) -> Result<u64, RuntimeError> {
        Ok(10)
    }
    fn logical_capacity(&self, _: &DesiredProvider) -> Result<usize, RuntimeError> {
        Ok(2)
    }
    fn build(
        &self,
        _: &DesiredProvider,
        _: Option<PreparedRuntime>,
        quota: voice_agent_server::workers::ProviderRuntimeAdmission,
    ) -> Result<Arc<dyn RuntimeResource>, RuntimeError> {
        self.0.lock().unwrap().push(quota);
        Ok(Arc::new(Resource))
    }
}

pub struct DrainingResource {
    pub entered: tokio::sync::mpsc::UnboundedSender<()>,
    pub release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
}
impl RuntimeResource for DrainingResource {
    fn unload(&self) -> bool {
        self.entered.send(()).unwrap();
        self.release.lock().unwrap().recv().unwrap();
        true
    }
}

pub struct DrainingBuilder(pub Arc<DrainingResource>);
impl RuntimeMaterializer for DrainingBuilder {
    fn estimated_peak_bytes(&self, _: &DesiredProvider) -> Result<u64, RuntimeError> {
        Ok(10)
    }
    fn logical_capacity(&self, _: &DesiredProvider) -> Result<usize, RuntimeError> {
        Ok(2)
    }
    fn build(
        &self,
        _: &DesiredProvider,
        _: Option<PreparedRuntime>,
        _: voice_agent_server::workers::ProviderRuntimeAdmission,
    ) -> Result<Arc<dyn RuntimeResource>, RuntimeError> {
        Ok(self.0.clone())
    }
}

pub struct HealthResource(pub Arc<std::sync::atomic::AtomicBool>);
impl RuntimeResource for HealthResource {
    fn health_flags(&self) -> Vec<Arc<std::sync::atomic::AtomicBool>> {
        vec![self.0.clone()]
    }
    fn unload(&self) -> bool {
        true
    }
}

pub struct HealthBuilder(pub Arc<std::sync::atomic::AtomicBool>);
impl RuntimeMaterializer for HealthBuilder {
    fn estimated_peak_bytes(&self, _: &DesiredProvider) -> Result<u64, RuntimeError> {
        Ok(10)
    }
    fn logical_capacity(&self, _: &DesiredProvider) -> Result<usize, RuntimeError> {
        Ok(2)
    }
    fn build(
        &self,
        _: &DesiredProvider,
        _: Option<PreparedRuntime>,
        _: voice_agent_server::workers::ProviderRuntimeAdmission,
    ) -> Result<Arc<dyn RuntimeResource>, RuntimeError> {
        Ok(Arc::new(HealthResource(self.0.clone())))
    }
}

pub struct SharedBuilder(pub Arc<AtomicUsize>);
impl RuntimeMaterializer for SharedBuilder {
    fn resource_key(
        &self,
        _: &DesiredProvider,
    ) -> Result<Option<voice_agent_server::services::provider_runtime::ResourceKey>, RuntimeError>
    {
        Ok(Some(
            voice_agent_server::services::provider_runtime::ResourceKey([1; 32]),
        ))
    }
    fn estimated_peak_bytes(&self, _: &DesiredProvider) -> Result<u64, RuntimeError> {
        Ok(10)
    }
    fn logical_capacity(&self, _: &DesiredProvider) -> Result<usize, RuntimeError> {
        Ok(2)
    }
    fn build(
        &self,
        _: &DesiredProvider,
        _: Option<PreparedRuntime>,
        _: voice_agent_server::workers::ProviderRuntimeAdmission,
    ) -> Result<Arc<dyn RuntimeResource>, RuntimeError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(Arc::new(Resource))
    }
}

pub struct InvalidMetadataResource {
    pub panic: bool,
}
impl RuntimeResource for InvalidMetadataResource {
    fn resource_key(&self) -> Option<voice_agent_server::services::provider_runtime::ResourceKey> {
        assert!(!self.panic, "fixture metadata callback panic");
        Some(voice_agent_server::services::provider_runtime::ResourceKey(
            [2; 32],
        ))
    }
    fn unload(&self) -> bool {
        true
    }
}

pub struct InvalidMetadataBuilder {
    pub panic: bool,
}
impl RuntimeMaterializer for InvalidMetadataBuilder {
    fn resource_key(
        &self,
        _: &DesiredProvider,
    ) -> Result<Option<voice_agent_server::services::provider_runtime::ResourceKey>, RuntimeError>
    {
        Ok(Some(
            voice_agent_server::services::provider_runtime::ResourceKey([1; 32]),
        ))
    }
    fn estimated_peak_bytes(&self, _: &DesiredProvider) -> Result<u64, RuntimeError> {
        Ok(10)
    }
    fn logical_capacity(&self, _: &DesiredProvider) -> Result<usize, RuntimeError> {
        Ok(2)
    }
    fn build(
        &self,
        _: &DesiredProvider,
        _: Option<PreparedRuntime>,
        _: voice_agent_server::workers::ProviderRuntimeAdmission,
    ) -> Result<Arc<dyn RuntimeResource>, RuntimeError> {
        Ok(Arc::new(InvalidMetadataResource { panic: self.panic }))
    }
}

pub struct GatedBuilder {
    pub entered: tokio::sync::mpsc::UnboundedSender<()>,
    pub release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
}
impl RuntimeMaterializer for GatedBuilder {
    fn estimated_peak_bytes(&self, _: &DesiredProvider) -> Result<u64, RuntimeError> {
        Ok(10)
    }
    fn logical_capacity(&self, _: &DesiredProvider) -> Result<usize, RuntimeError> {
        Ok(2)
    }
    fn build(
        &self,
        _: &DesiredProvider,
        _: Option<PreparedRuntime>,
        _: voice_agent_server::workers::ProviderRuntimeAdmission,
    ) -> Result<Arc<dyn RuntimeResource>, RuntimeError> {
        self.entered.send(()).unwrap();
        self.release.lock().unwrap().recv().unwrap();
        Ok(Arc::new(Resource))
    }
}

pub fn gated_manager(
    entered: tokio::sync::mpsc::UnboundedSender<()>,
    release: std::sync::mpsc::Receiver<()>,
) -> Arc<ProviderRuntimeManager> {
    ProviderRuntimeManager::new(
        RuntimeLimits {
            max_parallel_loads: 1,
            max_pending_loads: 0,
            max_waiters: 8,
            max_resident_bytes: 40,
            max_resources: 4,
            max_version_entries: 8,
            admission_timeout_ms: 1000,
            failure_cooldown_ms: 100,
            idle_ttl_ms: 1000,
        },
        Arc::new(GatedBuilder {
            entered,
            release: std::sync::Mutex::new(release),
        }),
        AdmissionGate::open(),
    )
    .unwrap()
}

pub fn pilot_manager(
    count: Arc<AtomicUsize>,
) -> (
    Arc<ProviderRuntimeManager>,
    Arc<voice_agent_server::session::pilot::PilotAdmission>,
) {
    let manager = manager(count);
    let pilot = Arc::new(voice_agent_server::session::pilot::PilotAdmission::new(
        true,
    ));
    manager.set_pilot_admission((*pilot).clone());
    (manager, pilot)
}
