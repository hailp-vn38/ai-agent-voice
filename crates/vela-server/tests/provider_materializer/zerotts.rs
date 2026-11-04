use super::harness::*;

#[tokio::test]
async fn switching_the_logical_voice_never_rematerializes_the_zerotts_runtime() {
    let builds = Arc::new(AtomicUsize::new(0));
    let manager = zerotts_manager(
        zerotts_factory(2),
        Arc::clone(&builds),
        Arc::new(Mutex::new(Vec::new())),
    );

    // An Agent's home binds Template A -> maichi and later switches to Template B -> hamy. The two
    // templates differ only in the logical voice, so the switch must alias the resident runtime.
    let template_a = manager
        .acquire(zerotts_provider(1, "tts-maichi", "maichi", "stream"))
        .await
        .unwrap();
    assert_eq!(builds.load(Ordering::SeqCst), 1);
    drop(template_a);

    let template_b = manager
        .acquire(zerotts_provider(2, "tts-hamy", "hamy", "stream"))
        .await
        .unwrap();
    assert_eq!(
        builds.load(Ordering::SeqCst),
        1,
        "switching template voice must not rebuild the ZeroTTS runtime"
    );
    assert_eq!(manager.accounting().resources, 1);
    drop(template_b);
    assert_eq!(
        manager.evict_idle().await.unwrap(),
        1,
        "one physical runtime stays resident until it is idle-drained"
    );
}

#[tokio::test]
async fn a_different_zerotts_delivery_mode_builds_a_second_physical_runtime() {
    let builds = Arc::new(AtomicUsize::new(0));
    let manager = zerotts_manager(
        zerotts_factory(2),
        Arc::clone(&builds),
        Arc::new(Mutex::new(Vec::new())),
    );

    let stream = manager
        .acquire(zerotts_provider(1, "tts-stream", "maichi", "stream"))
        .await
        .unwrap();
    let file = manager
        .acquire(zerotts_provider(2, "tts-file", "hamy", "file"))
        .await
        .unwrap();
    assert_eq!(
        builds.load(Ordering::SeqCst),
        2,
        "codec session topology differs, so delivery mode must isolate the runtime"
    );
    assert_eq!(manager.accounting().resources, 2);
    drop((stream, file));
}

#[test]
fn zerotts_physical_replica_count_follows_configured_tts_workers() {
    let builder = zerotts_factory(4);
    let row = zerotts_provider(1, "tts-maichi", "maichi", "stream");
    assert_eq!(builder.logical_capacity(&row).unwrap(), 4);
    assert_eq!(
        builder.physical_capacity(&row).unwrap(),
        4,
        "the configured pool must leave native capacity beyond the Voice reservation"
    );
}

#[tokio::test]
async fn a_shared_zerotts_pool_reserves_voice_capacity_and_releases_background_capacity() {
    let quotas = Arc::new(Mutex::new(Vec::new()));
    let manager = zerotts_manager(
        zerotts_factory(2),
        Arc::new(AtomicUsize::new(0)),
        Arc::clone(&quotas),
    );
    let lease = manager
        .acquire(zerotts_provider(1, "tts-maichi", "maichi", "stream"))
        .await
        .unwrap();
    let quota = quotas.lock().unwrap()[0].clone();

    for _ in 0..2 {
        let background = quota.try_admit(ProviderWorkloadClass::Background).unwrap();
        assert!(quota.try_admit(ProviderWorkloadClass::Background).is_err());
        let voice = quota.try_admit(ProviderWorkloadClass::Voice).unwrap();
        assert!(quota.try_admit(ProviderWorkloadClass::Voice).is_err());
        drop((background, voice));
    }
    drop(lease);
}

#[test]
fn deployment_startup_and_the_manager_agree_on_the_zerotts_replica_count() {
    let cfg = zerotts_config();
    let instance = ZeroTtsOnnxConfig {
        model: "zerotts_default".into(),
        num_threads: cfg.runtime.onnx.threads_for("zerotts_onnx"),
        voice: "maichi".into(),
        language: "vi-VN".into(),
        delivery_mode: ZeroTtsDeliveryMode::Stream,
        preload: false,
    };
    let value = serde_json::to_value(&instance).unwrap();

    assert_eq!(
        configured_physical_replicas("zerotts_onnx", value, &cfg.runtime),
        None,
        "deployment startup must also use the configured worker count"
    );
    // A remote adapter has no resident native topology at all.
    assert_eq!(
        configured_physical_replicas("chillaudio_ws", serde_json::json!({}), &cfg.runtime),
        None
    );
}
