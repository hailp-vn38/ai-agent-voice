use super::harness::*;

#[test]
#[ignore = "requires prepared ZeroTTS model files and ONNX Runtime"]
fn zerotts_background_pool_runs_successive_jobs() {
    use std::time::Instant;
    use voice_agent_server::{
        config::{AppConfig, TtsInstanceConfig},
        providers::compiled_provider_registry,
    };

    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::env::set_current_dir(&root).unwrap();
    let config = AppConfig::parse_and_resolve(root.join("config.toml")).unwrap();
    let provider = compiled_provider_registry()
        .tts_factory("zerotts_onnx")
        .unwrap()
        .build(
            &TtsInstanceConfig::ZeroTtsOnnx(Default::default()),
            &config.runtime,
        )
        .unwrap();
    let runtime = TtsWorkerRuntime::new_with_binding(
        provider,
        WorkerRuntimeConfig {
            max_workers: 2,
            voice_reserved_capacity: 1,
            final_timeout: Duration::from_secs(60),
            ..Default::default()
        },
        binding("maichi"),
    );
    assert!(runtime.has_background_capacity());
    for _ in 0..2 {
        let lease = runtime
            .start_background("Xin chào Việt Nam.".into())
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut samples = 0;
        loop {
            assert!(
                Instant::now() < deadline,
                "ZeroTTS background synthesis stalled"
            );
            match runtime.poll(lease).unwrap() {
                Some(TtsWorkerEvent::Pcm(pcm)) => samples += pcm.samples().len(),
                Some(TtsWorkerEvent::Finished) => break,
                Some(event) => panic!("unexpected ZeroTTS event: {event:?}"),
                None => std::thread::sleep(Duration::from_millis(5)),
            }
        }
        assert!(samples > 0);
        assert_eq!(runtime.active_leases(), 0);
    }
}

#[test]
fn logical_views_inject_their_own_binding_into_shared_workers() {
    let selected = Arc::new(std::sync::Mutex::new(Vec::new()));
    let owner = TtsWorkerRuntime::new_with_binding(
        Arc::new(BindingRecordingProvider(Arc::clone(&selected))),
        WorkerRuntimeConfig::default(),
        binding("maichi"),
    );
    let baotrang = owner.logical_view_for_test(binding("baotrang"));

    let first = owner.start("first".into()).unwrap();
    wait_for_finish(&owner, first);
    let second = baotrang.start("second".into()).unwrap();
    wait_for_finish(&baotrang, second);
    let third = owner.start("third".into()).unwrap();
    wait_for_finish(&owner, third);

    assert_eq!(
        *selected.lock().unwrap(),
        vec!["maichi", "baotrang", "maichi"]
    );
}

#[test]
fn a_stream_rejects_a_segment_from_a_different_logical_binding() {
    let selected = Arc::new(std::sync::Mutex::new(Vec::new()));
    let owner = TtsWorkerRuntime::new_with_binding(
        Arc::new(BindingRecordingProvider(selected)),
        WorkerRuntimeConfig::default(),
        binding("maichi"),
    );
    let alias = owner.logical_view_for_test(binding("baotrang"));
    let stream = owner.begin_stream();

    assert!(alias.start_in_stream(stream, "must fail".into()).is_err());
    let lease = owner
        .start_in_stream(stream, "owner still works".into())
        .unwrap();
    wait_for_finish(&owner, lease);
    owner.close_stream(stream);
}

#[test]
fn one_tts_stream_preserves_provider_state_across_speech_segments() {
    let runtime = TtsWorkerRuntime::new(Arc::new(StatefulTts), WorkerRuntimeConfig::default());
    let stream = runtime.begin_stream();
    for expected in [1.0, 2.0] {
        let lease = runtime.start_in_stream(stream, "segment".into()).unwrap();
        let pcm = loop {
            if let Some(TtsWorkerEvent::Pcm(pcm)) = runtime.poll(lease).unwrap() {
                break pcm;
            }
            std::thread::sleep(Duration::from_millis(1));
        };
        assert_eq!(pcm.samples()[0], expected);
        loop {
            if matches!(runtime.poll(lease).unwrap(), Some(TtsWorkerEvent::Finished)) {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    runtime.close_stream(stream);
}
