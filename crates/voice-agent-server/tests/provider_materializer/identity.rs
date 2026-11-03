use super::harness::*;

#[test]
fn native_resource_identity_normalizes_defaults_and_isolates_execution_and_credentials() {
    let mut cfg = config();
    cfg.runtime.onnx.library = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/onnx-runtime-fingerprint.txt");
    let builder = FactoryMaterializer::new(
        Arc::new(cfg.clone()),
        Arc::new(Secrets(AtomicUsize::new(0))),
        HashMap::from([("gipformer_sherpa_offline".into(), 1024)]),
        Arc::new(WorkerSupervisor::start_many(vec![], vec![])),
    )
    .unwrap();
    let mut row = DesiredProvider {
        id: 1,
        key: "a".into(),
        kind: "asr".into(),
        adapter: "gipformer_sherpa_offline".into(),
        revision: 1,
        config_json: r#"{}"#.into(),
        secret_ref: None,
    };
    let first = builder.resource_key(&row).unwrap().unwrap();
    row.config_json =
        voice_agent_server::database::provider_config::validate_raw(&row.adapter, &row.config_json)
            .unwrap();
    row.id = 2;
    row.key = "b".into();
    row.revision = 99;
    assert_eq!(builder.resource_key(&row).unwrap(), Some(first.clone()));
    let mut value: serde_json::Value = serde_json::from_str(&row.config_json).unwrap();
    value["num_threads"] = serde_json::json!(2);
    row.config_json = value.to_string();
    assert!(
        builder.resource_key(&row).is_err(),
        "DB thread override must be rejected"
    );
    value.as_object_mut().unwrap().remove("num_threads");
    row.config_json = value.to_string();
    assert_eq!(builder.resource_key(&row).unwrap(), Some(first));
    cfg.runtime
        .onnx
        .threads
        .insert("gipformer_sherpa_offline".into(), 2);
    let other = FactoryMaterializer::new(
        Arc::new(cfg),
        Arc::new(Secrets(AtomicUsize::new(0))),
        HashMap::from([("gipformer_sherpa_offline".into(), 1024)]),
        Arc::new(WorkerSupervisor::start_many(vec![], vec![])),
    )
    .unwrap();
    assert_ne!(
        other.resource_key(&row).unwrap(),
        builder.resource_key(&row).unwrap(),
        "server execution settings must isolate backing resources"
    );
    row.secret_ref = Some("CREDENTIAL".into());
    assert!(builder.resource_key(&row).unwrap().is_none());
}

#[test]
fn sensevoice_resource_identity_normalizes_defaults_and_isolates_language_itn_and_threads() {
    let mut cfg = config();
    cfg.runtime.onnx.library = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/onnx-runtime-fingerprint.txt");
    let builder = FactoryMaterializer::new(
        Arc::new(cfg.clone()),
        Arc::new(Secrets(AtomicUsize::new(0))),
        HashMap::from([("sensevoice_sherpa_offline".into(), 1024)]),
        Arc::new(WorkerSupervisor::start_many(vec![], vec![])),
    )
    .unwrap();
    let mut row = DesiredProvider {
        id: 1,
        key: "sv-a".into(),
        kind: "asr".into(),
        adapter: "sensevoice_sherpa_offline".into(),
        revision: 1,
        config_json: r#"{}"#.into(),
        secret_ref: None,
    };
    let default_key = builder.resource_key(&row).unwrap().unwrap();
    row.config_json = r#"{"language":"auto","use_itn":true}"#.into();
    assert_eq!(
        builder.resource_key(&row).unwrap(),
        Some(default_key.clone())
    );

    row.config_json = r#"{"language":"en","use_itn":true}"#.into();
    let en_key = builder.resource_key(&row).unwrap().unwrap();
    assert_ne!(
        en_key, default_key,
        "language changes the SenseVoice recognizer config"
    );

    row.config_json = r#"{"language":"en","use_itn":false}"#.into();
    let no_itn_key = builder.resource_key(&row).unwrap().unwrap();
    assert_ne!(
        no_itn_key, en_key,
        "use_itn changes the SenseVoice recognizer config"
    );

    row.config_json = r#"{"language":"fr","use_itn":true}"#.into();
    assert!(
        builder.resource_key(&row).is_err(),
        "unsupported language must be rejected"
    );

    row.config_json = r#"{"language":"auto","use_itn":true,"num_threads":2}"#.into();
    assert!(
        builder.resource_key(&row).is_err(),
        "DB thread override must be rejected"
    );
}

#[test]
fn zerotts_resource_identity_shares_voices_but_isolates_delivery_mode() {
    let mut cfg = config();
    cfg.runtime.onnx.library = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/onnx-runtime-fingerprint.txt");
    let builder = FactoryMaterializer::new(
        Arc::new(cfg),
        Arc::new(Secrets(AtomicUsize::new(0))),
        HashMap::from([("zerotts_onnx".into(), 1024)]),
        Arc::new(WorkerSupervisor::start_many(vec![], vec![])),
    )
    .unwrap();
    let row = |key: &str, voice: &str, delivery_mode: &str| DesiredProvider {
        id: 1,
        key: key.into(),
        kind: "tts".into(),
        adapter: "zerotts_onnx".into(),
        revision: 1,
        config_json: format!(
            r#"{{"voice":"{voice}","language":"vi-VN","delivery_mode":"{delivery_mode}"}}"#
        ),
        secret_ref: None,
    };

    let maichi = builder
        .resource_key(&row("maichi", "maichi", "stream"))
        .unwrap();
    let baotrang = builder
        .resource_key(&row("baotrang", "baotrang", "stream"))
        .unwrap();
    let file = builder
        .resource_key(&row("maichi-file", "maichi", "file"))
        .unwrap();
    assert_eq!(maichi, baotrang);
    assert_ne!(maichi, file);
    assert!(
        builder
            .resource_key(&row("unsupported", "not-a-zerotts-voice", "stream"))
            .is_err()
    );
}

#[test]
fn local_runtime_identity_uses_adapter_owned_physical_specs() {
    let mut cfg = config();
    cfg.runtime.onnx.library = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/onnx-runtime-fingerprint.txt");
    let builder = FactoryMaterializer::new(
        Arc::new(cfg),
        Arc::new(Secrets(AtomicUsize::new(0))),
        HashMap::from([
            ("silero_onnx".into(), 1024),
            ("zipformer_sherpa".into(), 1024),
            ("gipformer_sherpa_offline".into(), 1024),
        ]),
        Arc::new(WorkerSupervisor::start_many(vec![], vec![])),
    )
    .unwrap();
    let row = |kind: &str, adapter: &str, config_json: &str| DesiredProvider {
        // Deployment snapshots may include server-owned local settings; database provider
        // configuration intentionally cannot.
        id: 0,
        key: "local".into(),
        kind: kind.into(),
        adapter: adapter.into(),
        revision: 1,
        config_json: config_json.into(),
        secret_ref: None,
    };

    let silero_a = builder
        .resource_key(&row(
            "vad",
            "silero_onnx",
            r#"{"speech_threshold":0.5,"end_silence_ms":400}"#,
        ))
        .unwrap();
    let silero_b = builder
        .resource_key(&row(
            "vad",
            "silero_onnx",
            r#"{"speech_threshold":0.7,"end_silence_ms":800}"#,
        ))
        .unwrap();
    assert_eq!(silero_a, silero_b, "VAD endpoint policy is a logical view");

    let zipformer_greedy = builder
        .resource_key(&row(
            "asr",
            "zipformer_sherpa",
            r#"{"decoding_method":"greedy_search"}"#,
        ))
        .unwrap();
    let zipformer_beam = builder
        .resource_key(&row(
            "asr",
            "zipformer_sherpa",
            r#"{"decoding_method":"modified_beam_search"}"#,
        ))
        .unwrap();
    assert_ne!(
        zipformer_greedy, zipformer_beam,
        "Zipformer constructs a recognizer with its decoding method"
    );

    let gipformer_small_beam = builder
        .resource_key(&row(
            "asr",
            "gipformer_sherpa_offline",
            r#"{"max_active_paths":4}"#,
        ))
        .unwrap();
    let gipformer_large_beam = builder
        .resource_key(&row(
            "asr",
            "gipformer_sherpa_offline",
            r#"{"max_active_paths":8}"#,
        ))
        .unwrap();
    assert_ne!(
        gipformer_small_beam, gipformer_large_beam,
        "Gipformer constructs a recognizer with max_active_paths"
    );
}

#[test]
fn kokoro_runtime_plan_excludes_preload_but_keeps_native_selection_and_g2p_identity() {
    let planner = compiled_local_runtime_adapter_registry()
        .get("kokoro_vi_onnx")
        .expect("compiled Kokoro planner");
    let plan = |voice: &str, speed_percent: u16, preload: bool| {
        planner
            .physical_plan(serde_json::json!({
                "model": "kokoro_vi_contextbox",
                "num_threads": 2,
                "voice": voice,
                "language": "vi-VN",
                "speed_percent": speed_percent,
                "preload": preload,
            }))
            .unwrap()
    };
    let key = |plan: &LocalRuntimePlan, g2p_fingerprint| {
        plan.resource_key("artifact", [1; 32], Some(g2p_fingerprint), 2, 1)
            .unwrap()
    };

    let default = plan("mai_linh", 100, false);
    let preloaded = plan("mai_linh", 100, true);
    let other_voice = plan("thanh_dat", 100, false);
    let other_speed = plan("mai_linh", 120, false);
    assert_eq!(key(&default, [2; 32]), key(&preloaded, [2; 32]));
    assert_ne!(key(&default, [2; 32]), key(&other_voice, [2; 32]));
    assert_ne!(key(&default, [2; 32]), key(&other_speed, [2; 32]));
    assert_ne!(key(&default, [2; 32]), key(&default, [3; 32]));
}

#[test]
fn native_resource_identity_uses_execution_content_not_only_its_path() {
    let path = std::env::temp_dir().join(format!(
        "provider-runtime-execution-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::write(&path, b"execution-one").unwrap();
    let mut cfg = config();
    cfg.runtime.onnx.library = path.clone();
    let row = DesiredProvider {
        id: 1,
        key: "asr".into(),
        kind: "asr".into(),
        adapter: "gipformer_sherpa_offline".into(),
        revision: 1,
        config_json: r#"{}"#.into(),
        secret_ref: None,
    };
    let build = |cfg: AppConfig| {
        FactoryMaterializer::new(
            Arc::new(cfg),
            Arc::new(Secrets(AtomicUsize::new(0))),
            HashMap::from([("gipformer_sherpa_offline".into(), 1024)]),
            Arc::new(WorkerSupervisor::start_many(vec![], vec![])),
        )
        .unwrap()
    };
    let first = build(cfg.clone()).resource_key(&row).unwrap();
    std::fs::write(&path, b"execution-two").unwrap();
    let second = build(cfg).resource_key(&row).unwrap();
    std::fs::remove_file(path).unwrap();
    assert_ne!(first, second);
}
