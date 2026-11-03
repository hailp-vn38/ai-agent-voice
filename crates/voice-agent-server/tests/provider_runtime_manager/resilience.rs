use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use super::harness::*;

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
async fn unhealthy_retained_deployment_is_drained_and_retained_again_after_rebuild() {
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
    let mut deployment = snapshot(1);
    deployment.id = 0;
    let lease = manager
        .acquire_deployment_until(deployment.clone(), manager.admission_deadline())
        .await
        .unwrap();
    manager.retain_deployment(lease.version()).unwrap();
    drop(lease);
    health.store(false, Ordering::Release);
    assert!(!manager.deployment_ready("llm", "llm"));
    assert_eq!(manager.evict_idle().await.unwrap(), 1);
    assert_eq!(manager.accounting().reserved_bytes, 0);
    health.store(true, Ordering::Release);
    drop(
        manager
            .acquire_deployment_until(deployment, manager.admission_deadline())
            .await
            .unwrap(),
    );
    assert!(manager.deployment_ready("llm", "llm"));
    // The rebuilt default inherits the earlier retention instead of expiring like a cache entry.
    assert_eq!(manager.evict_idle().await.unwrap(), 0);
    assert_eq!(manager.accounting().reserved_bytes, 10);
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
