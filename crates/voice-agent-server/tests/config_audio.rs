use std::{
    collections::BTreeMap,
    fs,
    net::SocketAddr,
    path::PathBuf,
    sync::Arc,
    sync::atomic::{AtomicU64, Ordering},
};
use url::Url;
use voice_agent_server::config::{
    AppConfig, AsrInstanceConfig, AsrProvidersConfig, AudioConfig, AuthConfig, BargeInConfig,
    DeploymentConfig, EffectiveAgentConfig, EffectiveProviderBindings,
    GipformerSherpaOfflineConfig, LimitsConfig, LlmConfig, LlmInstanceConfig, LlmProvidersConfig,
    OpenAiConfig, ProviderDefaultsConfig, ProvidersConfig, RuntimeConfig, ServerConfig,
    SileroOnnxConfig, SpeechOutputConfig, TtsConfig, TtsInstanceConfig, TtsProvidersConfig,
    VadInstanceConfig, VadProvidersConfig, WebsocketConfig, WorkersConfig, ZeroTtsOnnxConfig,
    ZipformerSherpaConfig,
};
use voice_agent_server::{
    app::AppState,
    providers::{LlmProvider, ProviderSet, TtsProvider},
};

struct FakeLlm;

impl LlmProvider for FakeLlm {
    fn adapter(&self) -> &'static str {
        "fake_llm"
    }
}

struct FakeTts;

impl TtsProvider for FakeTts {
    fn adapter(&self) -> &'static str {
        "fake_tts"
    }
}

fn valid_config() -> AppConfig {
    AppConfig {
        server: ServerConfig {
            bind: "127.0.0.1:8000".parse::<SocketAddr>().unwrap(),
            public_ws_url: Url::parse("ws://127.0.0.1:8000/voice/v1/").unwrap(),
            hello_timeout_ms: 5_000,
        },
        auth: AuthConfig::default(),
        audio: AudioConfig::default(),
        websocket: WebsocketConfig::default(),
        limits: LimitsConfig::default(),
        provider_defaults: ProviderDefaultsConfig {
            vad: "vad".into(),
            asr: "asr".into(),
            llm: "llm".into(),
            tts: "tts".into(),
            vision: None,
        },
        providers: ProvidersConfig {
            vad: VadProvidersConfig {
                instances: BTreeMap::from([(
                    "vad".into(),
                    VadInstanceConfig::SileroOnnx(SileroOnnxConfig::default()),
                )]),
            },
            asr: AsrProvidersConfig {
                instances: BTreeMap::from([(
                    "asr".into(),
                    AsrInstanceConfig::ZipformerSherpa(ZipformerSherpaConfig::default()),
                )]),
            },
            llm: LlmProvidersConfig {
                instances: BTreeMap::from([(
                    "llm".into(),
                    LlmInstanceConfig::Openai(OpenAiConfig::default()),
                )]),
            },
            tts: TtsProvidersConfig {
                instances: BTreeMap::from([(
                    "tts".into(),
                    TtsInstanceConfig::ZeroTtsOnnx(ZeroTtsOnnxConfig::default()),
                )]),
            },
            vision: voice_agent_server::config::VisionProvidersConfig::default(),
        },
        workers: WorkersConfig::default(),
        deployment: DeploymentConfig::default(),
        runtime: RuntimeConfig::default(),
        provider_runtime: None,
        llm: LlmConfig::default(),
        tts: TtsConfig::default(),
        speech_output: SpeechOutputConfig::default(),
        barge_in: BargeInConfig::default(),
        mcp: voice_agent_server::config::McpConfig::default(),
        vision: voice_agent_server::config::VisionConfig::default(),
        database: voice_agent_server::config::DatabaseConfig::default(),
        api: voice_agent_server::config::AdminApiConfig::default(),
        shutdown: voice_agent_server::config::ShutdownConfig::default(),
        agent: None,
        effective_agent: EffectiveAgentConfig {
            providers: EffectiveProviderBindings {
                vad: "vad".into(),
                asr: "asr".into(),
                llm: "llm".into(),
                tts: "tts".into(),
                vision: None,
            },
            ..EffectiveAgentConfig::default()
        },
    }
}

fn load_catalog_config(extra: &str) -> Result<AppConfig, voice_agent_server::config::ConfigError> {
    static NEXT_CONFIG: AtomicU64 = AtomicU64::new(0);
    let path = PathBuf::from(format!(
        "/tmp/voice-agent-catalog-{}-{}.toml",
        std::process::id(),
        NEXT_CONFIG.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(
        &path,
        format!(
            r#"
[server]
bind = "127.0.0.1:8000"
public_ws_url = "ws://127.0.0.1:8000/voice/v1/"

[provider_defaults]
vad = "vad_default"
asr = "asr_default"
llm = "openai_primary"
tts = "tts_a"

[providers.vad.instances.vad_default]
adapter = "silero_onnx"

[providers.asr.instances.asr_default]
adapter = "zipformer_sherpa"

[providers.llm.instances.openai_primary]
adapter = "openai"
model = "test-model"

[providers.tts.instances.tts_a]
adapter = "zerotts_onnx"

[providers.tts.instances.tts_b]
adapter = "zerotts_onnx"
voice = "maichi"

{extra}
"#
        ),
    )
    .unwrap();
    let result = AppConfig::load(&path);
    let _ = fs::remove_file(path);
    result
}

#[test]
fn config_load_materializes_defaults_and_agent_tts_override_for_catalog_instances() {
    let config = load_catalog_config("[agent.providers]\ntts = 'tts_b'").unwrap();
    assert_eq!(config.providers.tts.instances.len(), 2);
    assert_eq!(config.effective_agent().providers.tts, "tts_b");
    assert_eq!(config.effective_agent().providers.llm, "openai_primary");
}

#[test]
fn config_load_rejects_a_binding_to_an_unknown_instance() {
    let error = load_catalog_config("[agent.providers]\ntts = 'missing'").unwrap_err();
    assert!(
        error
            .to_string()
            .contains("agent TTS provider `missing` does not exist")
    );
}

#[test]
fn config_rejects_invalid_database_and_shutdown_bounds_when_database_is_enabled() {
    let error = load_catalog_config(
        r#"
[database]
enabled = true
busy_timeout_ms = 0

[shutdown]
grace_ms = 999
"#,
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("database requires a local sqlite URL")
    );

    let error = load_catalog_config(
        r#"
[database]
enabled = true
url = "sqlite::memory:"

[shutdown]
grace_ms = 15000
"#,
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("database requires a local sqlite URL")
    );

    let error = load_catalog_config(
        r#"
[database]
enabled = true
url = "sqlite://?mode=memory"

[shutdown]
grace_ms = 15000
"#,
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("database requires a local sqlite URL")
    );

    let error = load_catalog_config(
        r#"
[database]
enabled = true
url = "sqlite://?mo%64e=memory"

[shutdown]
grace_ms = 15000
"#,
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("database requires a local sqlite URL")
    );
}

#[test]
fn config_rejects_shutdown_grace_outside_the_bounded_drain_window() {
    let error = load_catalog_config("[shutdown]\ngrace_ms = 60001").unwrap_err();
    assert!(
        error
            .to_string()
            .contains("shutdown.grace_ms must be 1000..=60000")
    );
}

#[test]
fn validation_rejects_a_missing_default_even_when_agent_overrides_it() {
    let mut config = valid_config();
    config.provider_defaults.tts = "missing".into();
    config.effective_agent.providers.tts = "tts".into();
    let error = config.validate().unwrap_err();
    assert!(
        error
            .to_string()
            .contains("agent default TTS provider `missing` does not exist")
    );
}

#[test]
fn audio_limit_is_converted_to_an_exact_frame_capacity() {
    let config = valid_config();
    assert_eq!(config.max_capture_frames(), 500);
    assert!(config.validate().is_ok());
}

#[test]
fn catalog_instances_keep_typed_adapter_options() {
    let config = valid_config();
    assert_eq!(
        config.providers.vad.instances["vad"].adapter(),
        "silero_onnx"
    );
    assert_eq!(
        config.providers.asr.instances["asr"].model(),
        "zipformer_vi_streaming"
    );
}

#[test]
fn config_accepts_multiple_asr_instances_with_different_adapters() {
    let config = load_catalog_config(
        r#"
[providers.asr.instances.gipformer_vi]
adapter = "gipformer_sherpa_offline"
model = "gipformer15_vi_int8"
num_threads = 4
decoding_method = "modified_beam_search"
max_active_paths = 4

[agent.providers]
asr = "gipformer_vi"
"#,
    )
    .unwrap();

    assert_eq!(config.providers.asr.instances.len(), 2);
    assert_eq!(
        config.providers.asr.instances["gipformer_vi"].adapter(),
        "gipformer_sherpa_offline"
    );
    assert_eq!(config.effective_agent().providers.asr, "gipformer_vi");
}

#[test]
fn config_rejects_invalid_gipformer_runtime_options() {
    for extra in [
        r#"
[providers.asr.instances.gipformer_vi]
adapter = "gipformer_sherpa_offline"
model = "gipformer15_vi_int8"
num_threads = 0
"#,
        r#"
[providers.asr.instances.gipformer_vi]
adapter = "gipformer_sherpa_offline"
model = "gipformer15_vi_int8"
decoding_method = "unsupported"
"#,
        r#"
[providers.asr.instances.gipformer_vi]
adapter = "gipformer_sherpa_offline"
model = "gipformer15_vi_int8"
max_active_paths = 0
"#,
        r#"
[providers.asr.instances.gipformer_vi]
adapter = "gipformer_sherpa_offline"
model = ""
"#,
        r#"
[providers.asr.instances.gipformer_vi]
adapter = "gipformer_sherpa_offline"
model = "gipformer15_vi_int8"
quantization = "int8"
"#,
    ] {
        assert!(load_catalog_config(extra).is_err());
    }
}

#[test]
fn gipformer_config_defaults_are_qualification_defaults() {
    let config: GipformerSherpaOfflineConfig = toml::from_str("model = 'gipformer15_vi_int8'")
        .expect("Gipformer configuration should parse");
    assert_eq!(config.num_threads, 4);
    assert_eq!(config.decoding_method, "modified_beam_search");
    assert_eq!(config.max_active_paths, 4);
}

#[test]
fn invalid_audio_and_transport_limits_fail_fast() {
    let mut config = valid_config();
    config.audio.max_utterance_ms = 1_000;
    assert!(config.validate().is_err());

    let mut config = valid_config();
    config.websocket.max_frame_bytes = 3_999;
    assert!(config.validate().is_err());
}

#[test]
fn worker_runtime_limits_and_timeouts_are_configured_and_validated() {
    let mut config = valid_config();
    config.workers.asr.max_workers = 3;
    config.workers.asr.command_queue_capacity = 5;
    config.workers.asr.final_timeout_ms = 1_200;
    config.workers.asr.cleanup_grace_ms = 300;
    config.workers.vad.max_workers = 2;
    config.workers.vad.command_queue_capacity = 4;
    config.workers.vad.reset_timeout_ms = 800;
    config.workers.vad.cleanup_grace_ms = 250;
    assert!(config.validate().is_ok());

    config.workers.asr.final_timeout_ms = 0;
    assert!(config.validate().is_err());

    config.workers.asr.final_timeout_ms = 1;
    config.workers.vad.max_workers = 0;
    assert!(config.validate().is_err());
}

#[test]
fn phase_four_delivery_capacity_and_speech_bounds_fail_fast() {
    let mut config = valid_config();
    config.limits.llm_concurrency = 2;
    config.limits.tts_concurrency = 2;
    config.workers.tts.max_workers = 2;
    config.workers.tts.command_queue_capacity = 4;
    config.workers.tts.cleanup_grace_ms = 250;
    config.tts.timeout_ms = 1_000;
    config.speech_output.min_chars = 24;
    config.speech_output.soft_break_min_chars = 48;
    config.speech_output.max_chars = 160;
    config.speech_output.pending_segments = 8;
    assert!(config.validate().is_ok());

    config.limits.tts_concurrency = 1;
    assert!(config.validate().is_ok());
    config.speech_output.soft_break_min_chars = 23;
    assert!(config.validate().is_err());
}

#[test]
fn local_http_llm_endpoint_is_allowed_but_remote_http_is_rejected() {
    let mut config = valid_config();
    {
        let LlmInstanceConfig::Openai(openai) =
            config.providers.llm.instances.get_mut("llm").unwrap();
        openai.base_url = Url::parse("http://localhost:20128/v1").unwrap();
    }
    assert!(config.validate().is_ok());

    let LlmInstanceConfig::Openai(openai) = config.providers.llm.instances.get_mut("llm").unwrap();
    openai.base_url = Url::parse("http://example.test/v1").unwrap();
    assert!(config.validate().is_err());
}

#[test]
fn application_state_preserves_injected_llm_and_tts_test_providers() {
    let providers = ProviderSet::with_llm_tts(Arc::new(FakeLlm), Arc::new(FakeTts));

    let state = AppState::from_provider_set(valid_config(), Arc::new(providers));

    assert_eq!(state.providers.llm("test").unwrap().adapter(), "fake_llm");
    assert_eq!(state.providers.tts("test").unwrap().adapter(), "fake_tts");
}

#[test]
fn application_builds_each_worker_runtime_from_its_own_config() {
    let mut config = valid_config();
    config.workers.asr.max_workers = 3;
    config.workers.asr.command_queue_capacity = 5;
    config.workers.asr.final_timeout_ms = 1_200;
    config.workers.asr.cleanup_grace_ms = 300;
    config.workers.vad.max_workers = 2;
    config.workers.vad.command_queue_capacity = 4;
    config.workers.vad.reset_timeout_ms = 800;
    config.workers.vad.cleanup_grace_ms = 250;

    let state = AppState::from_provider_set(config, Arc::new(ProviderSet::unavailable()));
    let resolved = state
        .runtimes
        .resolve(&state.config.effective_agent().providers)
        .unwrap();
    let asr = resolved.asr.runtime_config();
    let vad = resolved.vad.runtime_config();
    assert_eq!(asr.max_workers, 3);
    assert_eq!(asr.command_capacity, 5);
    assert_eq!(asr.final_timeout.as_millis(), 1_200);
    assert_eq!(asr.cleanup_grace.as_millis(), 300);
    assert_eq!(vad.max_workers, 2);
    assert_eq!(vad.command_capacity, 4);
    assert_eq!(vad.final_timeout.as_millis(), 800);
    assert_eq!(vad.cleanup_grace.as_millis(), 250);
}

#[test]
fn application_rejects_missing_local_model_artifacts_before_binding() {
    assert!(voice_agent_server::app::application(valid_config()).is_err());
}
