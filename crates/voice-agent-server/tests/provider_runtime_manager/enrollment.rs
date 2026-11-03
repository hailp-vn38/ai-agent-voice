use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use super::harness::*;

#[tokio::test]
async fn active_enrollment_refuses_cold_start_as_busy_not_permanent_failure() {
    let count = Arc::new(AtomicUsize::new(0));
    let (manager, pilot) = pilot_manager(count.clone());
    // A voice session holds the envelope; cold prep must refuse with Busy and must not
    // touch the materializer at all.
    let voice = pilot.try_voice().unwrap();
    assert!(matches!(
        manager.acquire(snapshot(1)).await,
        Err(RuntimeError::Busy)
    ));
    assert_eq!(count.load(Ordering::SeqCst), 0);
    assert_eq!(manager.accounting().reserved_bytes, 0);
    drop(voice);
    // Envelope free again: the same cold acquisition now proceeds.
    let lease = manager.acquire(snapshot(1)).await.unwrap();
    assert_eq!(lease.version(), &ProviderVersion::database(1, 1));
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn ready_backing_runtime_hot_acquire_keeps_capacity_while_voice_holds_envelope() {
    let count = Arc::new(AtomicUsize::new(0));
    let (manager, pilot) = pilot_manager(count.clone());
    let _first = manager.acquire(snapshot(1)).await.unwrap();
    // Voice owns the envelope, but the backing runtime is already Ready, so hot
    // acquisition must reuse it without asking the pilot for cold rights.
    let voice = pilot.try_voice().unwrap();
    let hot = manager.acquire(snapshot(1)).await.unwrap();
    assert_eq!(hot.version(), &ProviderVersion::database(1, 1));
    assert_eq!(count.load(Ordering::SeqCst), 1);
    drop(voice);
}

#[tokio::test]
async fn cold_start_holds_the_envelope_until_terminal_so_voice_sees_busy() {
    let (entered, mut entered_rx) = tokio::sync::mpsc::unbounded_channel();
    let (release, release_rx) = std::sync::mpsc::channel();
    let manager = gated_manager(entered, release_rx);
    let pilot = Arc::new(voice_agent_server::session::pilot::PilotAdmission::new(
        true,
    ));
    manager.set_pilot_admission((*pilot).clone());

    let cold = {
        let manager = manager.clone();
        tokio::spawn(async move { manager.acquire(snapshot(1)).await })
    };
    entered_rx.recv().await.unwrap();
    // Cold preparation is in progress: voice admission must be refused while it runs.
    assert!(pilot.try_voice().is_none());

    release.send(()).unwrap();
    let lease = cold.await.unwrap().unwrap();
    assert_eq!(lease.version(), &ProviderVersion::database(1, 1));
    // Terminal publication releases the envelope: voice now gets in.
    let voice = pilot
        .try_voice()
        .expect("envelope released after cold Ready");
    drop(voice);
    drop(lease);
}

#[tokio::test(start_paused = true)]
async fn cold_preparation_keeps_envelope_after_waiter_timeout_until_late_completion() {
    let (entered, mut entered_rx) = tokio::sync::mpsc::unbounded_channel();
    let (release, release_rx) = std::sync::mpsc::channel();
    let manager = gated_manager(entered, release_rx);
    let pilot = Arc::new(voice_agent_server::session::pilot::PilotAdmission::new(
        true,
    ));
    manager.set_pilot_admission((*pilot).clone());

    let caller = {
        let manager = manager.clone();
        tokio::spawn(async move { manager.acquire(snapshot(1)).await })
    };
    entered_rx.recv().await.unwrap();
    tokio::time::advance(std::time::Duration::from_secs(2)).await;
    assert!(matches!(caller.await.unwrap(), Err(RuntimeError::Timeout)));
    // The HTTP waiter is gone, but cold preparation still owns the envelope.
    assert!(pilot.try_voice().is_none());

    release.send(()).unwrap();
    tokio::time::resume();
    let lease = manager.acquire(snapshot(1)).await.unwrap();
    assert_eq!(lease.version(), &ProviderVersion::database(1, 1));
    assert!(
        pilot.try_voice().is_some(),
        "envelope released at terminal acknowledgement"
    );
}

#[tokio::test]
async fn draft_versions_are_isolated_from_database_and_deployment_and_bounded() {
    use voice_agent_server::database::secrets::{EnvSecretResolver, SecretResolver};
    struct DraftBuilder;
    impl RuntimeMaterializer for DraftBuilder {
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
            panic!("draft must not use saved/deployment build")
        }
        fn build_draft(
            &self,
            _: &DesiredProvider,
            _: Option<PreparedRuntime>,
            _: voice_agent_server::workers::ProviderRuntimeAdmission,
            _: &dyn SecretResolver,
        ) -> Result<Arc<dyn RuntimeResource>, RuntimeError> {
            Ok(Arc::new(Resource))
        }
    }
    let manager = ProviderRuntimeManager::new(
        RuntimeLimits {
            max_parallel_loads: 1,
            max_pending_loads: 1,
            max_waiters: 4,
            max_resident_bytes: 20,
            max_resources: 2,
            max_version_entries: 2,
            admission_timeout_ms: 1000,
            failure_cooldown_ms: 1,
            idle_ttl_ms: 1,
        },
        Arc::new(DraftBuilder),
        AdmissionGate::open(),
    )
    .unwrap();
    let mut draft = snapshot(1);
    draft.id = 0;
    let secrets = Arc::new(EnvSecretResolver);
    let first = manager
        .acquire_draft_until(draft.clone(), secrets.clone(), manager.admission_deadline())
        .await
        .unwrap();
    let second = manager
        .acquire_draft_until(draft.clone(), secrets.clone(), manager.admission_deadline())
        .await
        .unwrap();
    assert_ne!(first.version(), second.version());
    assert!(matches!(
        first.version().identity,
        voice_agent_server::services::provider_runtime::ProviderIdentity::Draft(_)
    ));
    assert_eq!(
        manager.inspect(1, 1).desired_state,
        voice_agent_server::services::provider_runtime::RuntimeState::Cold
    );
    assert!(matches!(
        manager
            .acquire_draft_until(draft.clone(), secrets.clone(), manager.admission_deadline())
            .await,
        Err(RuntimeError::Busy | RuntimeError::MemoryPressure)
    ));
    drop(first);
    drop(second);
    for _ in 0..20 {
        let result = manager
            .acquire_draft_until(draft.clone(), secrets.clone(), manager.admission_deadline())
            .await;
        // Metadata and memory pressure may reject a draft; they must never allow growth.
        drop(result);
        assert!(manager.accounting().version_entries <= 2);
        assert!(manager.accounting().reserved_bytes <= 20);
    }
}

#[tokio::test]
async fn deployment_acquisition_evicts_idle_database_provider_under_memory_pressure() {
    let count = Arc::new(AtomicUsize::new(0));
    let manager = ProviderRuntimeManager::new(
        RuntimeLimits {
            max_parallel_loads: 1,
            max_pending_loads: 2,
            max_waiters: 8,
            max_resident_bytes: 10,
            max_resources: 1,
            max_version_entries: 4,
            admission_timeout_ms: 1000,
            failure_cooldown_ms: 100,
            idle_ttl_ms: 60_000,
        },
        Arc::new(Builder(count.clone())),
        AdmissionGate::open(),
    )
    .unwrap();

    let db_lease = manager.acquire(snapshot(1)).await.unwrap();
    drop(db_lease);

    let mut deployment = snapshot(1);
    deployment.id = 0;
    deployment.key = "deploy-llm".into();
    let deploy_lease = manager
        .acquire_deployment_until(deployment, manager.admission_deadline())
        .await
        .expect("deployment provider should evict idle DB provider under pressure");
    assert_eq!(
        manager.state(&ProviderVersion::database(1, 1)),
        voice_agent_server::services::provider_runtime::RuntimeState::Cold
    );
    assert_eq!(count.load(Ordering::SeqCst), 2);
    drop(deploy_lease);
}

#[tokio::test]
async fn sibling_version_does_not_alias_failed_draining_or_quarantined_owner_with_same_resource_key()
 {
    use std::sync::atomic::AtomicBool;
    use voice_agent_server::services::provider_runtime::{ResourceKey, RuntimeState};

    struct SharedStateResource {
        healthy: Arc<AtomicBool>,
        drain_entered: Option<tokio::sync::mpsc::UnboundedSender<()>>,
        drain_release: Option<Arc<std::sync::Mutex<std::sync::mpsc::Receiver<()>>>>,
    }
    impl RuntimeResource for SharedStateResource {
        fn resource_key(&self) -> Option<ResourceKey> {
            Some(ResourceKey([9_u8; 32]))
        }
        fn health_flags(&self) -> Vec<Arc<AtomicBool>> {
            vec![self.healthy.clone()]
        }
        fn unload(&self) -> bool {
            if let Some(entered) = &self.drain_entered {
                let _ = entered.send(());
            }
            if let Some(release) = &self.drain_release {
                let _ = release.lock().unwrap().recv();
            }
            true
        }
    }

    struct SharedStateBuilder {
        builds: AtomicUsize,
        fail_first: bool,
        first_healthy: Arc<AtomicBool>,
        drain_entered: Option<tokio::sync::mpsc::UnboundedSender<()>>,
        drain_release: Option<Arc<std::sync::Mutex<std::sync::mpsc::Receiver<()>>>>,
    }
    impl RuntimeMaterializer for SharedStateBuilder {
        fn resource_key(&self, _: &DesiredProvider) -> Result<Option<ResourceKey>, RuntimeError> {
            Ok(Some(ResourceKey([9_u8; 32])))
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
            let attempt = self.builds.fetch_add(1, Ordering::SeqCst);
            if self.fail_first && attempt == 0 {
                return Err(RuntimeError::Unavailable);
            }
            Ok(Arc::new(SharedStateResource {
                healthy: if attempt == 0 {
                    self.first_healthy.clone()
                } else {
                    Arc::new(AtomicBool::new(true))
                },
                drain_entered: if attempt == 0 {
                    self.drain_entered.clone()
                } else {
                    None
                },
                drain_release: if attempt == 0 {
                    self.drain_release.clone()
                } else {
                    None
                },
            }))
        }
    }

    let limits = RuntimeLimits {
        max_parallel_loads: 2,
        max_pending_loads: 2,
        max_waiters: 8,
        max_resident_bytes: 40,
        max_resources: 4,
        max_version_entries: 8,
        admission_timeout_ms: 1000,
        failure_cooldown_ms: 5000,
        idle_ttl_ms: 60_000,
    };

    // Case 1: Failed owner must not be aliased
    let failed_builder = Arc::new(SharedStateBuilder {
        builds: AtomicUsize::new(0),
        fail_first: true,
        first_healthy: Arc::new(AtomicBool::new(true)),
        drain_entered: None,
        drain_release: None,
    });
    let manager = ProviderRuntimeManager::new(
        limits.clone(),
        failed_builder.clone(),
        AdmissionGate::open(),
    )
    .unwrap();
    assert!(matches!(
        manager.acquire(snapshot(1)).await,
        Err(RuntimeError::Unavailable)
    ));
    assert_eq!(
        manager.state(&ProviderVersion::database(1, 1)),
        RuntimeState::Failed
    );
    let mut second_snap = snapshot(1);
    second_snap.id = 2;
    let second = manager.acquire(second_snap.clone()).await.unwrap();
    assert_eq!(failed_builder.builds.load(Ordering::SeqCst), 2);
    drop(second);

    // Case 2: Quarantined (unhealthy) owner must not be aliased
    let first_healthy = Arc::new(AtomicBool::new(true));
    let quarantined_builder = Arc::new(SharedStateBuilder {
        builds: AtomicUsize::new(0),
        fail_first: false,
        first_healthy: first_healthy.clone(),
        drain_entered: None,
        drain_release: None,
    });
    let manager = ProviderRuntimeManager::new(
        limits.clone(),
        quarantined_builder.clone(),
        AdmissionGate::open(),
    )
    .unwrap();
    let first = manager.acquire(snapshot(1)).await.unwrap();
    first_healthy.store(false, Ordering::Release);
    assert_eq!(
        manager.inspect(1, 1).desired_state,
        RuntimeState::Quarantined
    );
    let second = manager.acquire(second_snap.clone()).await.unwrap();
    assert_eq!(quarantined_builder.builds.load(Ordering::SeqCst), 2);
    drop((first, second));

    // Case 3: Draining owner must not be aliased
    let (drain_entered, mut drain_entered_rx) = tokio::sync::mpsc::unbounded_channel();
    let (drain_release, drain_release_rx) = std::sync::mpsc::channel();
    let draining_builder = Arc::new(SharedStateBuilder {
        builds: AtomicUsize::new(0),
        fail_first: false,
        first_healthy: Arc::new(AtomicBool::new(true)),
        drain_entered: Some(drain_entered),
        drain_release: Some(Arc::new(std::sync::Mutex::new(drain_release_rx))),
    });
    let manager =
        ProviderRuntimeManager::new(limits, draining_builder.clone(), AdmissionGate::open())
            .unwrap();
    drop(manager.acquire(snapshot(1)).await.unwrap());
    let evict_task = {
        let manager = manager.clone();
        tokio::spawn(async move { manager.evict_idle().await })
    };
    drain_entered_rx.recv().await.unwrap();
    assert_eq!(
        manager.state(&ProviderVersion::database(1, 1)),
        RuntimeState::Draining
    );
    let second = manager.acquire(second_snap).await.unwrap();
    assert_eq!(draining_builder.builds.load(Ordering::SeqCst), 2);
    drain_release.send(()).unwrap();
    assert_eq!(evict_task.await.unwrap().unwrap(), 1);
    drop(second);
}
