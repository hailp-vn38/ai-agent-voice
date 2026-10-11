use super::*;

#[test]
fn snapshots_start_empty_and_expire_using_monotonic_time() {
    let metrics = SystemMetrics::new();
    assert!(metrics.latest().is_none());
    let mut collector = Collector::new();
    let snapshot = collector.sample();
    assert!(snapshot.system.cpu_percent.is_none());
    assert!(snapshot.process.cpu_percent.is_none());
    *metrics.snapshot.write().unwrap() = Some((Instant::now(), snapshot.clone()));
    assert_eq!(metrics.latest().unwrap().process.pid, std::process::id());
    *metrics.snapshot.write().unwrap() = Some((Instant::now() - Duration::from_secs(16), snapshot));
    assert!(metrics.latest().is_none());
}

#[test]
fn missing_os_data_remains_nullable_and_never_exposes_paths() {
    let mut collector = Collector::new();
    let snapshot = collector.snapshot(false, false, false, None);
    assert!(snapshot.system.memory_total_bytes.is_none());
    assert!(snapshot.process.rss_bytes.is_none());
    assert!(snapshot.storage.total_bytes.is_none());
    let json = serde_json::to_value(snapshot).unwrap();
    assert_eq!(json.as_object().unwrap().len(), 5);
    assert_eq!(
        json["storage"],
        serde_json::json!({"total_bytes":null,"available_bytes":null})
    );
}

#[tokio::test]
async fn sampler_publishes_and_stops_on_listening_cancellation() {
    let metrics = Arc::new(SystemMetrics::new());
    let stop = CancellationToken::new();
    let task = metrics.clone().start(stop.clone());
    tokio::time::timeout(Duration::from_secs(3), async {
        while metrics.latest().is_none() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    stop.cancel();
    tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap();
}

#[test]
fn cpu_recovery_waits_for_a_successful_process_baseline() {
    let mut collector = Collector::new();
    collector.sample();
    let missing = collector.snapshot(false, false, false, None);
    assert!(missing.system.cpu_percent.is_none());
    assert!(missing.system.memory_total_bytes.is_none());
    assert!(missing.process.cpu_percent.is_none());
    assert!(missing.storage.total_bytes.is_none());
    let recovered = collector.snapshot(true, true, true, None);
    assert!(recovered.system.cpu_percent.is_none());
    assert!(recovered.process.cpu_percent.is_none());
    let process_missing = collector.snapshot(true, true, false, None);
    assert!(process_missing.process.rss_bytes.is_none());
    let process_recovered = collector.snapshot(true, true, true, None);
    assert!(process_recovered.process.cpu_percent.is_none());
}

#[cfg(target_os = "linux")]
#[test]
fn unreadable_os_sources_are_unavailable() {
    assert!(!os_readable("/proc/voice-agent-missing-metrics-source"));
}
