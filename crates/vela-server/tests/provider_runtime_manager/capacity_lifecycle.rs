use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use super::harness::*;

#[tokio::test]
async fn revisions_share_logical_provider_capacity() {
    use vela_server::workers::ProviderWorkloadClass;
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
    release.send(()).unwrap();
    assert_eq!(unload.await.unwrap().unwrap(), 1);
    assert_eq!(manager.accounting().reserved_bytes, 0);
}

#[tokio::test]
async fn acquiring_a_draining_version_waits_for_unload_and_rebuilds() {
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
    let reacquire = {
        let manager = manager.clone();
        tokio::spawn(async move { manager.acquire(snapshot(1)).await })
    };
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while manager.accounting().waiters != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    release.send(()).unwrap();
    assert_eq!(unload.await.unwrap().unwrap(), 1);
    let lease = reacquire.await.unwrap().unwrap();
    assert_eq!(lease.version(), &ProviderVersion::database(1, 1));
    assert_eq!(manager.accounting().reserved_bytes, 10);
    assert_eq!(manager.accounting().build_attempts, 2);
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
            vela_server::services::provider_runtime::RuntimeState::Ready
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
