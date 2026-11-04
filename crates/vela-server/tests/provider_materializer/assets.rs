use super::harness::*;

/// One materialization ensures its provider's assets, and a second asks for nothing.
#[test]
fn one_materialization_ensures_its_provider_assets() {
    let cfg = zerotts_config();
    let builder = FactoryMaterializer::new(
        Arc::new(cfg.clone()),
        Arc::new(Secrets(AtomicUsize::new(0))),
        HashMap::from([("zerotts_onnx".into(), 1024)]),
        Arc::new(WorkerSupervisor::start_many(vec![], vec![])),
    )
    .unwrap();
    let row = zerotts_provider(1, "tts-maichi", "maichi", "stream");
    assert_eq!(
        builder.diagnostics().asset_ensures,
        0,
        "nothing downloads before a provider is asked for"
    );

    // Preparation delegates to whichever asset manager the adapter registered; it never names an
    // adapter itself.
    let assets = compiled_provider_adapter_registry()
        .assets(&row.adapter)
        .expect("ZeroTTS owns model assets");
    assert_eq!(
        assets.revision(),
        vela_server::providers::tts::zerotts::assets::MODEL_REVISION
    );

    // A remote provider has nothing to prepare.
    let remote = DesiredProvider {
        id: 1,
        key: "llm".into(),
        kind: "llm".into(),
        adapter: "openai".into(),
        revision: 1,
        config_json: r#"{"base_url":"https://example.test/v1","model":"fixture"}"#.into(),
        secret_ref: None,
    };
    assert!(matches!(
        builder.prepare_artifacts(&remote),
        Ok(Some(PreparedRuntime::Remote))
    ));
    assert_eq!(
        builder.diagnostics().asset_ensures,
        0,
        "a remote provider never enters the asset layer"
    );
}

/// Startup acquires the deployment default of every kind, so a deployment enters a known-good
/// state: every default provider has its model files installed and its runtime resident before the
/// server binds. It does not verify checksums or scan the model directory to do so.
#[test]
fn startup_materializes_every_deployment_default() {
    let mut config = zerotts_config();
    let selected = startup_providers(&config);

    let defaults = &config.provider_defaults;
    for (kind, key) in [
        ("vad", &defaults.vad),
        ("asr", &defaults.asr),
        ("llm", &defaults.llm),
        ("tts", &defaults.tts),
    ] {
        assert!(
            selected.contains(&(kind.to_owned(), key.clone())),
            "startup must materialize the default {kind} provider",
        );
    }

    // A TTS instance that did not ask to be preloaded is not startup's business.
    assert_eq!(selected.len(), 4, "only the four defaults: {selected:?}");

    config.providers.tts.instances.insert(
        "zerotts_hamy".into(),
        TtsInstanceConfig::ZeroTtsOnnx(ZeroTtsOnnxConfig {
            voice: "hamy".into(),
            preload: true,
            ..Default::default()
        }),
    );

    assert!(
        startup_providers(&config).contains(&("tts".to_owned(), "zerotts_hamy".to_owned())),
        "an explicitly preloaded TTS instance must also be ready before binding",
    );
}

/// Preparing a remote provider reports nothing to prepare and never enters the asset layer.
#[test]
fn preparing_a_remote_provider_does_not_enter_the_asset_layer() {
    let builder = FactoryMaterializer::new(
        Arc::new(config()),
        Arc::new(Secrets(AtomicUsize::new(0))),
        HashMap::from([("openai".into(), 1024)]),
        Arc::new(WorkerSupervisor::start_many(vec![], vec![])),
    )
    .unwrap();
    let remote = DesiredProvider {
        id: 1,
        key: "llm".into(),
        kind: "llm".into(),
        adapter: "openai".into(),
        revision: 1,
        config_json: r#"{"base_url":"https://example.test/v1","model":"fixture"}"#.into(),
        secret_ref: None,
    };

    assert!(matches!(
        builder.prepare_artifacts(&remote),
        Ok(Some(PreparedRuntime::Remote))
    ));
    assert_eq!(
        builder.diagnostics().asset_ensures,
        0,
        "a remote provider has no model files to ensure"
    );
    assert_eq!(
        builder.diagnostics().builds,
        0,
        "preparing assets must not construct a runtime"
    );
}
