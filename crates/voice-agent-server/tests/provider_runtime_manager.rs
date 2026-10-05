use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use voice_agent_server::{
    database::DesiredProvider,
    lifecycle::AdmissionGate,
    services::provider_runtime::{
        PreparedRuntime, ProviderRuntimeManager, ProviderVersion, RuntimeError, RuntimeLimits,
        RuntimeMaterializer, RuntimeResource,
    },
};

struct Resource;
impl RuntimeResource for Resource {
    fn unload(&self) -> bool {
        true
    }
}
struct Builder(Arc<AtomicUsize>);
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
fn snapshot(revision: i64) -> DesiredProvider {
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
fn manager(count: Arc<AtomicUsize>) -> Arc<ProviderRuntimeManager> {
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

#[tokio::test]
async fn concurrent_exact_version_acquisition_coalesces_and_retains_provenance() {
    let count = Arc::new(AtomicUsize::new(0));
    let manager = manager(count.clone());
    let (a, b, c) = tokio::join!(
        manager.acquire(snapshot(1)),
        manager.acquire(snapshot(1)),
        manager.acquire(snapshot(1))
    );
    let leases = [a.unwrap(), b.unwrap(), c.unwrap()];
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert!(
        leases
            .iter()
            .all(|lease| lease.version() == &ProviderVersion::database(1, 1))
    );
    let second = manager.acquire(snapshot(2)).await.unwrap();
    assert_eq!(second.version(), &ProviderVersion::database(1, 2));
    assert_eq!(leases[0].version(), &ProviderVersion::database(1, 1));
    assert_eq!(count.load(Ordering::SeqCst), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sequential_cold_acquisitions_release_the_only_load_slot_before_ready() {
    // Admission loads each required provider in sequence. With no pending queue, Ready
    // must mean the preceding native attempt has returned its capacity as well.
    for _ in 0..128 {
        let count = Arc::new(AtomicUsize::new(0));
        let manager = ProviderRuntimeManager::new(
            RuntimeLimits {
                max_parallel_loads: 1,
                max_pending_loads: 0,
                max_waiters: 16,
                max_resident_bytes: 40,
                max_resources: 4,
                max_version_entries: 8,
                admission_timeout_ms: 2000,
                failure_cooldown_ms: 100,
                idle_ttl_ms: 1000,
            },
            Arc::new(Builder(count.clone())),
            AdmissionGate::open(),
        )
        .unwrap();
        let mut leases = Vec::new();
        for revision in 1..=4 {
            leases.push(manager.acquire(snapshot(revision)).await.unwrap());
        }
        assert_eq!(count.load(Ordering::SeqCst), 4);
    }
}

struct BlockingBuilder {
    entered: tokio::sync::mpsc::UnboundedSender<()>,
    release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
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

#[tokio::test(start_paused = true)]
async fn waiter_timeout_retains_native_attempt_memory_and_load_capacity() {
    let (entered, mut entered_rx) = tokio::sync::mpsc::unbounded_channel();
    let (release, release_rx) = std::sync::mpsc::channel();
    let manager = ProviderRuntimeManager::new(
        RuntimeLimits {
            max_parallel_loads: 1,
            max_pending_loads: 1,
            max_waiters: 2,
            max_resident_bytes: 40,
            max_resources: 1,
            max_version_entries: 2,
            admission_timeout_ms: 1000,
            failure_cooldown_ms: 100,
            idle_ttl_ms: 1000,
        },
        Arc::new(BlockingBuilder {
            entered,
            release: std::sync::Mutex::new(release_rx),
        }),
        AdmissionGate::open(),
    )
    .unwrap();
    let caller = {
        let manager = manager.clone();
        tokio::spawn(async move { manager.acquire(snapshot(1)).await })
    };
    entered_rx.recv().await.unwrap();
    tokio::time::advance(std::time::Duration::from_secs(2)).await;
    assert!(matches!(caller.await.unwrap(), Err(RuntimeError::Timeout)));
    assert_eq!(manager.accounting().reserved_bytes, 40);
    assert_eq!(manager.accounting().build_attempts, 1);
    assert_eq!(manager.accounting().waiters, 0);
    assert!(matches!(
        manager.acquire(snapshot(2)).await,
        Err(RuntimeError::MemoryPressure)
    ));
    release.send(()).unwrap();
    tokio::time::resume();
    let lease = manager.acquire(snapshot(1)).await.unwrap();
    assert_eq!(lease.version(), &ProviderVersion::database(1, 1));
    assert_eq!(manager.accounting().build_attempts, 1);
}

#[tokio::test]
async fn active_lease_prevents_eviction_and_last_clone_release_starts_idle_lifetime() {
    let manager = manager(Arc::new(AtomicUsize::new(0)));
    let lease = manager.acquire(snapshot(1)).await.unwrap();
    let clone = lease.clone();
    drop(lease);
    assert_eq!(manager.evict_idle().await.unwrap(), 0);
    assert_eq!(manager.accounting().reserved_bytes, 10);
    drop(clone);
    assert_eq!(manager.evict_idle().await.unwrap(), 1);
    assert_eq!(manager.accounting().reserved_bytes, 0);
    assert_eq!(
        manager.state(&ProviderVersion::database(1, 1)),
        voice_agent_server::services::provider_runtime::RuntimeState::Cold
    );
}

struct FailureBuilder(Arc<AtomicUsize>, RuntimeError);
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
fn failure_manager(count: Arc<AtomicUsize>, failure: RuntimeError) -> Arc<ProviderRuntimeManager> {
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
#[tokio::test]
async fn failed_attempt_is_coalesced_during_cooldown_and_does_not_publish_ready() {
    let count = Arc::new(AtomicUsize::new(0));
    let manager = failure_manager(count.clone(), RuntimeError::Unavailable);
    assert!(matches!(
        manager.acquire(snapshot(1)).await,
        Err(RuntimeError::Unavailable)
    ));
    assert!(matches!(
        manager.acquire(snapshot(1)).await,
        Err(RuntimeError::Unavailable)
    ));
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(manager.accounting().reserved_bytes, 0);
}
#[tokio::test]
async fn uncertain_cleanup_retains_memory_and_loader_and_refuses_replacement() {
    let count = Arc::new(AtomicUsize::new(0));
    let manager = failure_manager(count.clone(), RuntimeError::Quarantined);
    assert!(matches!(
        manager.acquire(snapshot(1)).await,
        Err(RuntimeError::Quarantined)
    ));
    assert!(matches!(
        manager.acquire(snapshot(1)).await,
        Err(RuntimeError::Quarantined)
    ));
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(manager.accounting().reserved_bytes, 10);
    // A different revision queues but cannot start native work through the held loader slot.
    assert!(matches!(
        manager.acquire(snapshot(2)).await,
        Err(RuntimeError::Timeout)
    ));
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(manager.accounting().reserved_bytes, 20);
    assert!(!manager.shutdown_until(tokio::time::Instant::now()).await);
    assert!(matches!(
        manager.acquire(snapshot(3)).await,
        Err(RuntimeError::ShuttingDown)
    ));
}

struct QuotaBuilder(std::sync::Mutex<Vec<voice_agent_server::workers::ProviderRuntimeAdmission>>);
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
#[tokio::test]
async fn revisions_share_logical_provider_capacity() {
    use voice_agent_server::workers::ProviderWorkloadClass;
    let builder = Arc::new(QuotaBuilder(std::sync::Mutex::new(Vec::new())));
    let manager = ProviderRuntimeManager::new(
        RuntimeLimits {
            max_parallel_loads: 1,
            max_pending_loads: 2,
            max_waiters: 4,
            max_resident_bytes: 40,
            max_resources: 4,
            max_version_entries: 4,
            admission_timeout_ms: 1000,
            failure_cooldown_ms: 100,
            idle_ttl_ms: 1000,
        },
        builder.clone(),
        AdmissionGate::open(),
    )
    .unwrap();
    let a = manager.acquire(snapshot(1)).await.unwrap();
    let b = manager.acquire(snapshot(2)).await.unwrap();
    let quotas = builder.0.lock().unwrap();
    let first = quotas[0].try_admit(ProviderWorkloadClass::Voice).unwrap();
    let second = quotas[1].try_admit(ProviderWorkloadClass::Voice).unwrap();
    assert!(quotas[0].try_admit(ProviderWorkloadClass::Voice).is_err());
    assert!(
        quotas[1]
            .try_admit(ProviderWorkloadClass::Diagnostic)
            .is_err()
    );
    drop(first);
    drop(second);
    drop(a);
    drop(b);
}

struct DrainingResource {
    entered: tokio::sync::mpsc::UnboundedSender<()>,
    release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
}
impl RuntimeResource for DrainingResource {
    fn unload(&self) -> bool {
        self.entered.send(()).unwrap();
        self.release.lock().unwrap().recv().unwrap();
        true
    }
}
struct DrainingBuilder(Arc<DrainingResource>);
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
#[tokio::test]
async fn draining_resource_keeps_memory_until_actual_unload_acknowledgement() {
    let (entered, mut entered_rx) = tokio::sync::mpsc::unbounded_channel();
    let (release, release_rx) = std::sync::mpsc::channel();
    let manager = ProviderRuntimeManager::new(
        RuntimeLimits {
            max_parallel_loads: 1,
            max_pending_loads: 1,
            max_waiters: 2,
            max_resident_bytes: 10,
            max_resources: 1,
            max_version_entries: 2,
            admission_timeout_ms: 1000,
            failure_cooldown_ms: 100,
            idle_ttl_ms: 1000,
        },
        Arc::new(DrainingBuilder(Arc::new(DrainingResource {
            entered,
            release: std::sync::Mutex::new(release_rx),
        }))),
        AdmissionGate::open(),
    )
    .unwrap();
    drop(manager.acquire(snapshot(1)).await.unwrap());
    let unload = {
        let manager = manager.clone();
        tokio::spawn(async move { manager.evict_idle().await })
    };
    entered_rx.recv().await.unwrap();
    assert_eq!(manager.accounting().reserved_bytes, 10);
    assert!(matches!(
        manager.acquire(snapshot(2)).await,
        Err(RuntimeError::MemoryPressure)
    ));
    assert!(matches!(
        manager.acquire(snapshot(1)).await,
        Err(RuntimeError::Unavailable)
    ));
    release.send(()).unwrap();
    assert_eq!(unload.await.unwrap().unwrap(), 1);
    assert_eq!(manager.accounting().reserved_bytes, 0);
}

#[tokio::test]
async fn load_queue_is_bounded_without_allocating_an_extra_native_resource() {
    let (entered, mut entered_rx) = tokio::sync::mpsc::unbounded_channel();
    let (release, release_rx) = std::sync::mpsc::channel();
    let manager = ProviderRuntimeManager::new(
        RuntimeLimits {
            max_parallel_loads: 1,
            max_pending_loads: 1,
            max_waiters: 8,
            max_resident_bytes: 200,
            max_resources: 4,
            max_version_entries: 4,
            admission_timeout_ms: 1000,
            failure_cooldown_ms: 100,
            idle_ttl_ms: 1000,
        },
        Arc::new(BlockingBuilder {
            entered,
            release: std::sync::Mutex::new(release_rx),
        }),
        AdmissionGate::open(),
    )
    .unwrap();
    let first = {
        let manager = manager.clone();
        tokio::spawn(async move { manager.acquire(snapshot(1)).await })
    };
    entered_rx.recv().await.unwrap();
    let second = {
        let manager = manager.clone();
        tokio::spawn(async move { manager.acquire(snapshot(2)).await })
    };
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while manager.accounting().waiters != 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(matches!(
        manager.acquire(snapshot(3)).await,
        Err(RuntimeError::Busy)
    ));
    assert_eq!(manager.accounting().build_attempts, 1);
    release.send(()).unwrap();
    entered_rx.recv().await.unwrap();
    release.send(()).unwrap();
    let leases = [
        first.await.unwrap().unwrap(),
        second.await.unwrap().unwrap(),
    ];
    assert_eq!(manager.accounting().build_attempts, 2);
    assert_eq!(leases[1].version(), &ProviderVersion::database(1, 2));
}

#[tokio::test]
async fn status_polling_does_not_extend_idle_ttl() {
    let manager = manager(Arc::new(AtomicUsize::new(0)));
    let lease = manager.acquire(snapshot(1)).await.unwrap();
    tokio::time::pause();
    drop(lease);
    tokio::time::advance(std::time::Duration::from_millis(900)).await;
    for _ in 0..10 {
        assert_eq!(
            manager.state(&ProviderVersion::database(1, 1)),
            voice_agent_server::services::provider_runtime::RuntimeState::Ready
        );
    }
    assert_eq!(manager.evict_expired().await.unwrap(), 0);
    tokio::time::advance(std::time::Duration::from_millis(100)).await;
    tokio::time::resume();
    assert_eq!(manager.evict_expired().await.unwrap(), 1);
    assert_eq!(manager.accounting().build_attempts, 1);
}

#[tokio::test]
async fn shutdown_uses_supplied_deadline_and_keeps_active_resource_accounted() {
    let manager = manager(Arc::new(AtomicUsize::new(0)));
    let lease = manager.acquire(snapshot(1)).await.unwrap();
    assert!(!manager.shutdown_until(tokio::time::Instant::now()).await);
    assert_eq!(manager.accounting().reserved_bytes, 10);
    assert!(matches!(
        manager.acquire(snapshot(2)).await,
        Err(RuntimeError::ShuttingDown)
    ));
    drop(lease);
    assert!(
        manager
            .shutdown_until(tokio::time::Instant::now() + std::time::Duration::from_secs(1))
            .await
    );
    assert_eq!(manager.accounting().reserved_bytes, 0);
}

#[test]
fn native_completion_remains_owned_when_the_original_async_executor_is_gone() {
    let (entered, mut entered_rx) = tokio::sync::mpsc::unbounded_channel();
    let (release, release_rx) = std::sync::mpsc::channel();
    let manager = ProviderRuntimeManager::new(
        RuntimeLimits {
            max_parallel_loads: 1,
            max_pending_loads: 1,
            max_waiters: 2,
            max_resident_bytes: 40,
            max_resources: 1,
            max_version_entries: 2,
            admission_timeout_ms: 1000,
            failure_cooldown_ms: 100,
            idle_ttl_ms: 1000,
        },
        Arc::new(BlockingBuilder {
            entered,
            release: std::sync::Mutex::new(release_rx),
        }),
        AdmissionGate::open(),
    )
    .unwrap();
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    executor.block_on(async {
        let manager = manager.clone();
        tokio::spawn(async move { manager.acquire(snapshot(1)).await });
        entered_rx.recv().await.unwrap();
    });
    executor.shutdown_background();
    assert_eq!(manager.accounting().reserved_bytes, 40);
    release.send(()).unwrap();
    let replacement = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let lease = replacement.block_on(manager.acquire(snapshot(1))).unwrap();
    assert_eq!(lease.version(), &ProviderVersion::database(1, 1));
    assert_eq!(manager.accounting().build_attempts, 1);
}

struct HealthResource(Arc<std::sync::atomic::AtomicBool>);
impl RuntimeResource for HealthResource {
    fn health_flags(&self) -> Vec<Arc<std::sync::atomic::AtomicBool>> {
        vec![self.0.clone()]
    }
    fn unload(&self) -> bool {
        true
    }
}
struct HealthBuilder(Arc<std::sync::atomic::AtomicBool>);
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
#[tokio::test]
async fn unhealthy_retained_worker_is_quarantined_without_releasing_memory_or_rebuilding() {
    let health = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let manager = ProviderRuntimeManager::new(
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
        Arc::new(HealthBuilder(health.clone())),
        AdmissionGate::open(),
    )
    .unwrap();
    let lease = manager.acquire(snapshot(1)).await.unwrap();
    health.store(false, Ordering::Release);
    assert_eq!(
        manager.inspect(1, 1).desired_state,
        voice_agent_server::services::provider_runtime::RuntimeState::Quarantined
    );
    assert!(manager.inspect(1, 1).ready_revisions.is_empty());
    assert!(matches!(
        manager.acquire(snapshot(1)).await,
        Err(RuntimeError::Quarantined)
    ));
    assert_eq!(manager.accounting().reserved_bytes, 10);
    assert_eq!(manager.accounting().build_attempts, 1);
    drop(lease);
}

#[tokio::test]
async fn deployment_defaults_have_a_separate_identity_and_are_retained_until_shutdown() {
    let manager = manager(Arc::new(AtomicUsize::new(0)));
    let database = manager.acquire(snapshot(1)).await.unwrap();
    let mut deployment = snapshot(1);
    deployment.id = 0;
    let default = manager
        .acquire_deployment_until(deployment, manager.admission_deadline())
        .await
        .unwrap();
    assert_ne!(default.version(), database.version());
    assert!(matches!(
        default.version().identity,
        voice_agent_server::services::provider_runtime::ProviderIdentity::Deployment { .. }
    ));
    manager.retain_deployment(default.version()).unwrap();
    drop(default);
    drop(database);
    assert_eq!(manager.evict_idle().await.unwrap(), 1);
    assert!(manager.deployment_ready("llm", "llm"));
    assert!(
        manager
            .shutdown_until(tokio::time::Instant::now() + std::time::Duration::from_secs(1))
            .await
    );
    assert!(!manager.deployment_ready("llm", "llm"));
    assert_eq!(manager.accounting().reserved_bytes, 0);
}

struct SharedBuilder(Arc<AtomicUsize>);
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

#[tokio::test]
async fn metadata_pressure_never_prunes_a_retained_deployment_alias() {
    let manager = ProviderRuntimeManager::new(
        RuntimeLimits {
            max_parallel_loads: 1,
            max_pending_loads: 4,
            max_waiters: 16,
            max_resident_bytes: 10,
            max_resources: 1,
            max_version_entries: 4,
            admission_timeout_ms: 1000,
            failure_cooldown_ms: 100,
            idle_ttl_ms: 1000,
        },
        Arc::new(SharedBuilder(Arc::new(AtomicUsize::new(0)))),
        AdmissionGate::open(),
    )
    .unwrap();
    let mut first = snapshot(1);
    first.id = 0;
    let first = manager
        .acquire_deployment_until(first, manager.admission_deadline())
        .await
        .unwrap();
    manager.retain_deployment(first.version()).unwrap();
    let mut second = snapshot(1);
    second.id = 0;
    second.key = "other".into();
    let second = manager
        .acquire_deployment_until(second, manager.admission_deadline())
        .await
        .unwrap();
    manager.retain_deployment(second.version()).unwrap();
    drop((first, second));

    for revision in 1..8 {
        drop(manager.acquire(snapshot(revision)).await.unwrap());
        assert!(manager.deployment_ready("llm", "llm"));
        assert!(manager.deployment_ready("llm", "other"));
        assert!(manager.accounting().version_entries <= 4);
    }
}

#[tokio::test]
async fn equivalent_versions_and_instances_share_one_backing_resource_with_exact_provenance() {
    let count = Arc::new(AtomicUsize::new(0));
    let manager = ProviderRuntimeManager::new(
        RuntimeLimits {
            max_parallel_loads: 1,
            max_pending_loads: 4,
            max_waiters: 16,
            max_resident_bytes: 10,
            max_resources: 1,
            max_version_entries: 4,
            admission_timeout_ms: 1000,
            failure_cooldown_ms: 100,
            idle_ttl_ms: 1000,
        },
        Arc::new(SharedBuilder(count.clone())),
        AdmissionGate::open(),
    )
    .unwrap();
    let mut other = snapshot(1);
    other.id = 2;
    other.key = "other".into();
    let (a, b, c) = tokio::join!(
        manager.acquire(snapshot(1)),
        manager.acquire(snapshot(2)),
        manager.acquire(other)
    );
    let a = a.unwrap();
    let b = b.unwrap();
    let c = c.unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(manager.accounting().resources, 1);
    assert_eq!(manager.accounting().reserved_bytes, 10);
    assert_eq!(a.version(), &ProviderVersion::database(1, 1));
    assert_eq!(b.version(), &ProviderVersion::database(1, 2));
    assert_eq!(c.version(), &ProviderVersion::database(2, 1));
    drop(a);
    drop(b);
    assert_eq!(manager.evict_idle().await.unwrap(), 0);
    drop(c);
    assert_eq!(manager.evict_idle().await.unwrap(), 1);
    assert_eq!(manager.accounting().reserved_bytes, 0);
}

#[tokio::test]
async fn pressure_evicts_only_lru_idle_resource_and_preserves_active_lease() {
    let count = Arc::new(AtomicUsize::new(0));
    let manager = manager(count.clone());
    let first = manager.acquire(snapshot(1)).await.unwrap();
    let second = manager.acquire(snapshot(2)).await.unwrap();
    drop(second);
    let third = manager.acquire(snapshot(3)).await.unwrap();
    drop(third);
    let fourth = manager.acquire(snapshot(4)).await.unwrap();
    let fifth = manager.acquire(snapshot(5)).await.unwrap();
    assert_eq!(manager.accounting().reserved_bytes, 40);
    assert_eq!(manager.accounting().resources, 4);
    assert!(manager.inspect(1, 1).ready_revisions.contains(&1));
    assert!(!manager.inspect(1, 2).ready_revisions.contains(&2));
    assert!(manager.inspect(1, 3).ready_revisions.contains(&3));
    drop((first, fourth, fifth));
}

#[tokio::test]
async fn equivalent_revision_churn_keeps_alias_metadata_bounded() {
    let count = Arc::new(AtomicUsize::new(0));
    let manager = ProviderRuntimeManager::new(
        RuntimeLimits {
            max_parallel_loads: 1,
            max_pending_loads: 4,
            max_waiters: 16,
            max_resident_bytes: 10,
            max_resources: 1,
            max_version_entries: 4,
            admission_timeout_ms: 1000,
            failure_cooldown_ms: 100,
            idle_ttl_ms: 1000,
        },
        Arc::new(SharedBuilder(count.clone())),
        AdmissionGate::open(),
    )
    .unwrap();
    let original = manager.acquire(snapshot(1)).await.unwrap();
    for revision in 2..100 {
        let lease = manager.acquire(snapshot(revision)).await.unwrap();
        assert_eq!(lease.version(), &ProviderVersion::database(1, revision));
        assert!(manager.accounting().version_entries <= 4);
    }
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(manager.accounting().reserved_bytes, 10);
    drop(original);
    assert_eq!(manager.evict_idle().await.unwrap(), 1);
    assert_eq!(manager.accounting().reserved_bytes, 0);
}

struct InvalidMetadataResource {
    panic: bool,
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
struct InvalidMetadataBuilder {
    panic: bool,
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
#[tokio::test]
async fn installed_identity_drift_and_metadata_panics_quarantine_until_unload_ack() {
    for panic in [false, true] {
        let manager = ProviderRuntimeManager::new(
            RuntimeLimits {
                max_parallel_loads: 1,
                max_pending_loads: 0,
                max_waiters: 4,
                max_resident_bytes: 10,
                max_resources: 1,
                max_version_entries: 4,
                admission_timeout_ms: 1000,
                failure_cooldown_ms: 100,
                idle_ttl_ms: 1000,
            },
            Arc::new(InvalidMetadataBuilder { panic }),
            AdmissionGate::open(),
        )
        .unwrap();
        assert!(matches!(
            manager.acquire(snapshot(1)).await,
            Err(RuntimeError::Quarantined)
        ));
        assert_eq!(manager.accounting().quarantined_bytes, 10);
        assert!(manager.inspect(1, 1).ready_revisions.is_empty());
        assert_eq!(manager.evict_idle().await.unwrap(), 1);
        assert_eq!(manager.accounting().reserved_bytes, 0);
    }
}
