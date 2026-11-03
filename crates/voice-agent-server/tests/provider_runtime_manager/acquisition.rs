use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use super::harness::*;

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
async fn uncertain_cleanup_retains_memory_and_refuses_replacement_but_returns_the_loader() {
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
    // The quarantined attempt's thread has returned, so another revision still gets the loader.
    assert!(matches!(
        manager.acquire(snapshot(2)).await,
        Err(RuntimeError::Quarantined)
    ));
    assert_eq!(count.load(Ordering::SeqCst), 2);
    assert_eq!(manager.accounting().reserved_bytes, 20);
    // Reservations nothing can reclaim do not hold shutdown until its deadline.
    let started = std::time::Instant::now();
    assert!(
        !manager
            .shutdown_until(tokio::time::Instant::now() + std::time::Duration::from_secs(10))
            .await
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
    assert!(matches!(
        manager.acquire(snapshot(3)).await,
        Err(RuntimeError::ShuttingDown)
    ));
}
