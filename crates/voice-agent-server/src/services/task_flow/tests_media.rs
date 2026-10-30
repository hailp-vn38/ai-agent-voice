use super::*;

#[tokio::test]
async fn catalog_media_backend_and_reasoning_tag_wire_provider_capabilities() {
    use crate::config::SecretString;
    use axum::{
        Json, Router,
        http::{HeaderMap, StatusCode, header},
        response::IntoResponse,
        routing::post,
    };
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use orchestrator_agent_rs::ReasoningEffort;
    use orchestrator_flow_rs::{
        ImageGenerationRequest, ImageGeneratorBackend, TtsSynthesisRequest, TtsSynthesizerBackend,
        VideoGenerationRequest, VideoGeneratorBackend,
    };

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route(
                    "/v1/images/generations",
                    post(|| async {
                        Json(serde_json::json!({
                            "data": [{ "b64_json": STANDARD.encode(b"UPSTREAM_PNG_BYTES") }]
                        }))
                    }),
                )
                .route(
                    "/v1/videos/generations",
                    post(|| async {
                        Json(serde_json::json!({
                            "data": [{ "b64_json": STANDARD.encode(b"UPSTREAM_MP4_BYTES") }]
                        }))
                    }),
                )
                .route(
                    "/v1/audio/speech",
                    post(|| async {
                        let mut headers = HeaderMap::new();
                        headers.insert(header::CONTENT_TYPE, "audio/wav".parse().unwrap());
                        (StatusCode::OK, headers, b"RIFF_UPSTREAM_WAV".to_vec()).into_response()
                    }),
                ),
        )
        .await
        .unwrap();
    });

    let reasoning_provider = ResolvedLlmProvider {
        provider_ref: "llm_reasoner".into(),
        name: "Reasoner".into(),
        source: "database",
        adapter: "openai".into(),
        model: "o3-mini".into(),
        base_url: base_url.clone(),
        api_key: Some(SecretString::new("sk-reasoner")),
        gateway_binding: None,
        timeout_ms: 10_000,
        max_tokens: 8_192,
        context_limit_tokens: 128_000,
        reasoning_effort: None,
        temperature_percent: None,
        tags: vec!["reasoning".into()],
        muse_options: None,
    };
    let media_provider = ResolvedLlmProvider {
        provider_ref: "llm_media".into(),
        name: "Omni Media".into(),
        source: "database",
        adapter: "openai".into(),
        model: "gpt-omni-media".into(),
        base_url,
        api_key: Some(SecretString::new("sk-media")),
        gateway_binding: None,
        timeout_ms: 10_000,
        max_tokens: 4_096,
        context_limit_tokens: 64_000,
        reasoning_effort: None,
        temperature_percent: None,
        tags: vec!["image".into(), "video".into(), "audio".into()],
        muse_options: None,
    };

    let catalog = vec![reasoning_provider, media_provider];
    let settings = PersistedBackgroundSettings {
        orchestrator_llm_provider: "llm_reasoner".into(),
        worker_llm_provider: "llm_reasoner".into(),
        ..PersistedBackgroundSettings::default()
    };
    let (_, defs) =
        build_agent_definitions(&OrchestratorSectionConfig::default(), &settings, &catalog);
    let daily_root = defs.iter().find(|d| d.id == "daily_root").unwrap();
    assert_eq!(
        daily_root.llm.reasoning_effort,
        Some(ReasoningEffort::Medium),
        "reasoning-tagged provider should default AgentDefinition reasoning_effort to Medium"
    );

    let backend = CatalogMediaBackend::from_catalog(catalog).with_settings(settings);
    let img = backend
        .generate_image(&ImageGenerationRequest {
            prompt: "A diagram".into(),
            title: None,
            filename: "diagram.png".into(),
            format: "png".into(),
            width: 256,
            height: 256,
            svg_content: None,
            reference_images: Vec::new(),
            transparent_background: false,
        })
        .await
        .unwrap();
    assert_eq!(img.provider, "llm_media");
    assert_eq!(img.bytes, b"UPSTREAM_PNG_BYTES");

    let vid = backend
        .generate_video(&VideoGenerationRequest {
            prompt: "A rack flythrough".into(),
            title: Some("Flythrough".into()),
            filename: "flythrough.mp4".into(),
            width: 640,
            height: 360,
            duration_secs: 4,
            first_frame: None,
        })
        .await
        .unwrap();
    assert_eq!(vid.provider, "llm_media");
    assert_eq!(vid.bytes, b"UPSTREAM_MP4_BYTES");
    assert_eq!(vid.mime_type, "video/mp4");

    let audio = backend
        .synthesize_tts(&TtsSynthesisRequest {
            text: "Xin chào".into(),
            filename: "greeting.wav".into(),
            voice: "alloy".into(),
            language: "vi".into(),
            speed: 1.0,
        })
        .await
        .unwrap();
    assert_eq!(audio.provider, "llm_media");
    assert_eq!(audio.bytes, b"RIFF_UPSTREAM_WAV");

    // Server native TTS provider (`type = "tts"`, `tags: ["tts"]`, no API key required)
    struct TestServerTts;
    impl crate::providers::TtsProvider for TestServerTts {
        fn adapter(&self) -> &'static str {
            "vieneu"
        }
        fn synthesize_stream(
            &self,
            _text: &str,
            on_pcm: &mut dyn FnMut(
                crate::audio::PcmF32Mono,
            ) -> Result<(), crate::providers::TtsError>,
        ) -> Result<(), crate::providers::TtsError> {
            on_pcm(crate::audio::PcmF32Mono::new(vec![0.15; 2_400], 24_000))
        }
    }

    let tts_runtime = std::sync::Arc::new(crate::workers::TtsWorkerRuntime::new(
        std::sync::Arc::new(TestServerTts),
        crate::workers::WorkerRuntimeConfig {
            max_workers: 1,
            final_timeout: std::time::Duration::from_secs(10),
            cleanup_grace: std::time::Duration::from_secs(5),
            ..crate::workers::WorkerRuntimeConfig::default()
        },
    ));
    let server_tts_provider = ResolvedLlmProvider {
        provider_ref: "tts_vieneu_local".into(),
        name: "VieNeu Local TTS".into(),
        source: "database",
        adapter: "vieneu".into(),
        model: "Binh".into(),
        base_url: String::new(),
        api_key: None,
        gateway_binding: None,
        timeout_ms: 15_000,
        max_tokens: 4_096,
        context_limit_tokens: 64_000,
        reasoning_effort: None,
        temperature_percent: None,
        tags: vec!["tts".into()],
        muse_options: None,
    };
    let tts_backend = CatalogMediaBackend::from_catalog(vec![server_tts_provider])
        .with_tts_runtime("tts_vieneu_local", tts_runtime.clone());
    let server_audio = tts_backend
        .synthesize_tts(&TtsSynthesisRequest {
            text: "Xin chào từ server TTS".into(),
            filename: "server-tts.wav".into(),
            voice: "vi-default".into(),
            language: "vi-VN".into(),
            speed: 1.0,
        })
        .await
        .unwrap();
    assert_eq!(server_audio.provider, "tts_vieneu_local");
    assert_eq!(server_audio.mime_type, "audio/wav");
    assert_eq!(server_audio.sample_rate_hz, 24_000);
    assert!(server_audio.bytes.starts_with(b"RIFF"));
    let _ = tts_runtime.shutdown_acknowledged();

    // Fallback when catalog has no image/video/audio tagged provider
    let empty_backend = CatalogMediaBackend::from_catalog(Vec::new());
    let fallback_img = empty_backend
        .generate_image(&ImageGenerationRequest {
            prompt: "Fallback".into(),
            title: None,
            filename: "fallback.png".into(),
            format: "png".into(),
            width: 64,
            height: 64,
            svg_content: None,
            reference_images: Vec::new(),
            transparent_background: false,
        })
        .await
        .unwrap();
    assert_eq!(fallback_img.provider, "builtin_png_renderer");

    let fallback_vid = empty_backend
        .generate_video(&VideoGenerationRequest {
            prompt: "Fallback video".into(),
            title: None,
            filename: "fallback.mp4".into(),
            width: 64,
            height: 64,
            duration_secs: 2,
            first_frame: None,
        })
        .await
        .unwrap();
    assert_eq!(fallback_vid.provider, "builtin_mp4_renderer");
    assert_eq!(&fallback_vid.bytes[4..8], b"ftyp");

    // Default dimensions when width/height are 0 (1024x1024 for PNG, 640x360 for MP4)
    let default_dim_img = backend
        .generate_image(&ImageGenerationRequest {
            prompt: "Default size diagram".into(),
            title: None,
            filename: "default-size.png".into(),
            format: "png".into(),
            width: 0,
            height: 0,
            svg_content: None,
            reference_images: Vec::new(),
            transparent_background: false,
        })
        .await
        .unwrap();
    assert_eq!(
        default_dim_img.width,
        orchestrator_flow_rs::DEFAULT_PNG_WIDTH
    );
    assert_eq!(
        default_dim_img.height,
        orchestrator_flow_rs::DEFAULT_PNG_HEIGHT
    );

    server.abort();
}

#[tokio::test]
async fn asr_provider_catalog_validation_and_server_asr_transcribe_backend() {
    use orchestrator_flow_rs::{AsrTranscriberBackend, AsrTranscriptionRequest, inspect_wav_bytes};
    use std::collections::BTreeMap;

    let cfg = OrchestratorSectionConfig::default();
    let chat_prov = default_orchestrator_provider(&cfg);
    let tts_prov = ResolvedLlmProvider {
        provider_ref: "config:tts:vieneu".into(),
        name: "VieNeu TTS".into(),
        source: "config",
        adapter: "vieneu".into(),
        model: "Binh".into(),
        base_url: String::new(),
        api_key: None,
        gateway_binding: None,
        timeout_ms: 15_000,
        max_tokens: 4_096,
        context_limit_tokens: 64_000,
        reasoning_effort: None,
        temperature_percent: None,
        tags: vec!["tts".into()],
        muse_options: None,
    };
    let asr_prov = ResolvedLlmProvider {
        provider_ref: "config:asr:zipformer_vi".into(),
        name: "Zipformer VI".into(),
        source: "config",
        adapter: "zipformer_sherpa".into(),
        model: "zipformer-vi-30m".into(),
        base_url: String::new(),
        api_key: None,
        gateway_binding: None,
        timeout_ms: 15_000,
        max_tokens: 4_096,
        context_limit_tokens: 64_000,
        reasoning_effort: None,
        temperature_percent: None,
        tags: vec!["asr".into()],
        muse_options: None,
    };

    assert!(asr_prov.is_asr_adapter());
    assert!(!asr_prov.is_tts_adapter());
    assert!(!asr_prov.requires_api_key());
    assert!(asr_prov.supports_tag("asr"));
    assert!(asr_prov.supports_tag("audio"));
    assert!(!asr_prov.supports_tag("tts"));
    assert!(!tts_prov.supports_tag("asr"));

    // Exhaustively verify every `AsrInstanceConfig` variant's `.adapter()` is recognized by `is_asr_adapter()`
    let asr_variants: Vec<crate::config::AsrInstanceConfig> = vec![
        serde_json::from_value(serde_json::json!({ "adapter": "zipformer_sherpa" })).unwrap(),
        serde_json::from_value(serde_json::json!({ "adapter": "gipformer_sherpa_offline" }))
            .unwrap(),
        serde_json::from_value(serde_json::json!({ "adapter": "sensevoice_sherpa_offline" }))
            .unwrap(),
        #[cfg(feature = "qualification-providers")]
        serde_json::from_value(serde_json::json!({ "adapter": "qualification_asr" })).unwrap(),
    ];
    for inst in &asr_variants {
        let adapter_name = match inst {
            crate::config::AsrInstanceConfig::ZipformerSherpa(_) => inst.adapter(),
            crate::config::AsrInstanceConfig::GipformerSherpaOffline(_) => inst.adapter(),
            crate::config::AsrInstanceConfig::SenseVoiceSherpaOffline(_) => inst.adapter(),
            #[cfg(feature = "qualification-providers")]
            crate::config::AsrInstanceConfig::QualificationAsr(_) => inst.adapter(),
        };
        let p = ResolvedLlmProvider {
            adapter: adapter_name.to_owned(),
            tags: vec!["asr".into()],
            ..asr_prov.clone()
        };
        assert!(
            p.is_asr_adapter(),
            "AsrInstanceConfig adapter `{adapter_name}` must return true for is_asr_adapter()"
        );
    }

    let catalog = vec![chat_prov, tts_prov, asr_prov];

    // Assigning a TTS provider to `asr_transcribe` must fail validation with `provider_capability_mismatch`
    let mut bad_agents = BTreeMap::new();
    bad_agents.insert(
        "media_processor".to_owned(),
        AgentConfigOverride {
            enabled: true,
            tool_providers: Some(BTreeMap::from([(
                "asr_transcribe".to_owned(),
                "config:tts:vieneu".to_owned(),
            )])),
            ..Default::default()
        },
    );
    assert_eq!(
        validate_agent_overrides(
            DEFAULT_ORCHESTRATOR_PROVIDER_REF,
            DEFAULT_ORCHESTRATOR_PROVIDER_REF,
            &bad_agents,
            &catalog,
        ),
        Err("provider_capability_mismatch")
    );

    // Assigning the ASR provider to `asr_transcribe` succeeds and `build_agent_definitions` filters out ASR from Orchestrator chat LLM list
    let mut ok_agents = BTreeMap::new();
    ok_agents.insert(
        "media_processor".to_owned(),
        AgentConfigOverride {
            enabled: true,
            tool_providers: Some(BTreeMap::from([(
                "asr_transcribe".to_owned(),
                "config:asr:zipformer_vi".to_owned(),
            )])),
            ..Default::default()
        },
    );
    assert!(
        validate_agent_overrides(
            DEFAULT_ORCHESTRATOR_PROVIDER_REF,
            DEFAULT_ORCHESTRATOR_PROVIDER_REF,
            &ok_agents,
            &catalog,
        )
        .is_ok()
    );

    let settings = PersistedBackgroundSettings {
        orchestrator_llm_provider: DEFAULT_ORCHESTRATOR_PROVIDER_REF.into(),
        worker_llm_provider: DEFAULT_ORCHESTRATOR_PROVIDER_REF.into(),
        agents: ok_agents,
        ..Default::default()
    };
    let (orch_providers, _) = build_agent_definitions(&cfg, &settings, &catalog);
    assert!(
        !orch_providers
            .iter()
            .any(|p| p.id == "config:asr:zipformer_vi"),
        "ASR adapters must not be registered as OpenAI-compatible chat LLM providers"
    );

    // Server native ASR runtime integration via `CatalogMediaBackend`
    struct TestServerAsr;
    struct TestServerAsrSession(usize);
    impl crate::providers::AsrProvider for TestServerAsr {
        fn open(
            &self,
        ) -> Result<Box<dyn crate::providers::AsrSession>, crate::providers::AsrError> {
            Ok(Box::new(TestServerAsrSession(0)))
        }
        fn adapter(&self) -> &'static str {
            "zipformer_sherpa"
        }
    }
    impl crate::providers::AsrSession for TestServerAsrSession {
        fn push_pcm(
            &mut self,
            pcm: &crate::audio::PcmF32Mono,
        ) -> Result<Vec<crate::providers::AsrEvent>, crate::providers::AsrError> {
            assert_eq!(pcm.sample_rate_hz(), 16_000);
            self.0 += pcm.samples().len();
            Ok(Vec::new())
        }
        fn finish(&mut self) -> Result<crate::providers::AsrResult, crate::providers::AsrError> {
            Ok(crate::providers::AsrResult::new(format!(
                "Xin chào từ Zipformer ({} mẫu 16kHz)",
                self.0
            )))
        }
        fn cancel(&mut self) {}
    }

    let asr_runtime = std::sync::Arc::new(crate::workers::AsrWorkerRuntime::new(
        std::sync::Arc::new(TestServerAsr),
        crate::workers::WorkerRuntimeConfig {
            max_workers: 2,
            voice_reserved_capacity: 1,
            final_timeout: std::time::Duration::from_secs(10),
            cleanup_grace: std::time::Duration::from_secs(5),
            ..crate::workers::WorkerRuntimeConfig::default()
        },
    ));

    let backend = CatalogMediaBackend::from_catalog(catalog)
        .with_settings(settings)
        .with_asr_runtime("config:asr:zipformer_vi", asr_runtime.clone());

    // Build a 48 kHz stereo 16-bit WAV (1 second = 48_000 stereo frames) -> resampled to 16_000 samples @ 16 kHz mono
    let mut wav_cursor = std::io::Cursor::new(Vec::new());
    {
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::new(&mut wav_cursor, spec).unwrap();
        for _ in 0..(48_000 * 2) {
            writer.write_sample(1200_i16).unwrap();
        }
        writer.finalize().unwrap();
    }
    let wav_bytes = wav_cursor.into_inner();
    let header = inspect_wav_bytes(&wav_bytes).unwrap();
    let payload = backend
        .transcribe_asr(&AsrTranscriptionRequest {
            wav_bytes,
            header,
            language: Some("vi-VN".into()),
        })
        .await
        .unwrap();
    assert_eq!(payload.text, "Xin chào từ Zipformer (16000 mẫu 16kHz)");
    assert_eq!(
        payload.segments,
        vec![orchestrator_flow_rs::TranscribedAudioSegment {
            start_ms: 0,
            end_ms: 1_000,
            text: "Xin chào từ Zipformer (16000 mẫu 16kHz)".into(),
        }]
    );
    assert_eq!(
        payload.provider_id.as_deref(),
        Some("config:asr:zipformer_vi")
    );
    assert_eq!(payload.model.as_deref(), Some("zipformer-vi-30m"));
    assert_eq!(payload.sample_rate, 48_000);
    assert_eq!(payload.channels, 2);
    let _ = asr_runtime.shutdown_acknowledged();
}
