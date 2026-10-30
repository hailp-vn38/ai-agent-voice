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

#[test]
fn tts_constraints_and_voice_resolution_across_all_adapters() {
    let make_prov = |adapter: &str, model: &str| ResolvedLlmProvider {
        provider_ref: format!("config:tts:{adapter}"),
        name: format!("Test {adapter}"),
        source: "config",
        adapter: adapter.to_owned(),
        model: model.to_owned(),
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

    // 1. VieNeu v3 Turbo (25 voices: 11 female, 14 male) configured with male default `hai_dang`
    let vieneu_turbo = make_prov("vieneu", "vieneu_v3_turbo_fp32:hai_dang");
    let turbo_c = vieneu_turbo.tts_constraints(None);
    assert_eq!(turbo_c.default_voice, "hai_dang");
    assert_eq!(turbo_c.female_voices.len(), 11);
    assert_eq!(turbo_c.male_voices.len(), 14);
    assert_eq!(
        vieneu_turbo.resolve_tts_voice(None, Some("female")),
        "ngoc_huyen"
    );
    assert_eq!(
        vieneu_turbo.resolve_tts_voice(None, Some("vi-female")),
        "ngoc_huyen"
    );
    assert_eq!(
        vieneu_turbo.resolve_tts_voice(None, Some("giọng nữ")),
        "ngoc_huyen"
    );
    assert_eq!(
        vieneu_turbo.resolve_tts_voice(None, Some("male")),
        "hai_dang"
    );
    assert_eq!(
        vieneu_turbo.resolve_tts_voice(None, Some("truc_ly")),
        "truc_ly"
    );
    assert_eq!(
        vieneu_turbo.resolve_tts_voice(None, Some("alloy")),
        "hai_dang"
    );
    // Cross-variant fallback: Nano-only female voice `ai_han` on Turbo -> Turbo female `ngoc_huyen`
    assert_eq!(
        vieneu_turbo.resolve_tts_voice(None, Some("ai_han")),
        "ngoc_huyen"
    );

    // When VieNeu Turbo is already configured with a female voice (`mai_anh`), requesting `"female"` keeps `"mai_anh"`
    assert_eq!(
        vieneu_turbo.resolve_tts_voice(Some("mai_anh"), Some("female")),
        "mai_anh"
    );

    // 2. VieNeu v3 Nano (11 voices: 4 female, 7 male) configured with overlapping male voice `adam`
    let vieneu_nano = make_prov("vieneu", "vieneu_v3_nano_fp32:adam");
    let nano_c = vieneu_nano.tts_constraints(None);
    assert_eq!(nano_c.default_voice, "adam");
    assert_eq!(
        nano_c.female_voices,
        vec!["my_duyen", "mai_anh", "truc_ly", "ai_han"]
    );
    assert_eq!(nano_c.male_voices.len(), 7);
    assert_eq!(
        vieneu_nano.resolve_tts_voice(None, Some("female")),
        "my_duyen"
    );
    assert_eq!(
        vieneu_nano.resolve_tts_voice(None, Some("ai_han")),
        "ai_han"
    );
    // Cross-variant fallback: Turbo-only female voice `ngoc_huyen` on Nano -> Nano female `my_duyen`
    assert_eq!(
        vieneu_nano.resolve_tts_voice(None, Some("ngoc_huyen")),
        "my_duyen"
    );
    // Cross-variant fallback: Turbo-only male voice `hai_dang` on Nano (when default is `my_duyen`) -> Nano male `minh_quan`
    assert_eq!(
        vieneu_nano.resolve_tts_voice(Some("my_duyen"), Some("hai_dang")),
        "minh_quan"
    );

    // 3. ZeroTTS (`giahuy` male default -> `maichi` female)
    let zerotts = make_prov("zerotts_onnx", "giahuy");
    assert_eq!(zerotts.resolve_tts_voice(None, Some("female")), "maichi");
    assert_eq!(zerotts.resolve_tts_voice(None, Some("male")), "giahuy");

    // 4. Edge TTS (advertises 3 female + 3 male, accepts all 322 voices)
    let edge = make_prov("edge_tts", "vi-VN-NamMinhNeural");
    let edge_c = edge.tts_constraints(None);
    assert_eq!(edge_c.default_voice, "vi-VN-NamMinhNeural");
    assert_eq!(edge_c.female_voices.len(), 3);
    assert_eq!(edge_c.male_voices.len(), 3);
    assert_eq!(
        edge.resolve_tts_voice(None, Some("female")),
        "vi-VN-HoaiMyNeural"
    );
    assert_eq!(
        edge.resolve_tts_voice(None, Some("af-ZA-AdriNeural")),
        "af-ZA-AdriNeural"
    );

    // 5. Kokoro VI (`manh_dung` male default -> `ngoc_huyen` female)
    let kokoro = make_prov("kokoro_vi_onnx", "kokoro_vi_contextbox:manh_dung");
    assert_eq!(
        kokoro.resolve_tts_voice(None, Some("female")),
        "ngoc_huyen"
    );
    assert_eq!(kokoro.resolve_tts_voice(None, Some("male")), "manh_dung");

    // 6. ChillAudio WS (`BV075_streaming` male default -> `BV421_vivn_streaming` female)
    let chill = make_prov("chillaudio_ws", "BV075_streaming");
    assert_eq!(
        chill.resolve_tts_voice(None, Some("female")),
        "BV421_vivn_streaming"
    );
    assert_eq!(
        chill.resolve_tts_voice(None, Some("male")),
        "BV075_streaming"
    );

    // 7. OpenAI AudioSpeech (`onyx` male default -> `nova` female; `"alloy"` is preserved on OpenAI)
    let openai = make_prov("openai", "tts-1:onyx");
    assert_eq!(openai.resolve_tts_voice(None, Some("female")), "nova");
    assert_eq!(openai.resolve_tts_voice(None, Some("alloy")), "alloy");
}

#[tokio::test]
async fn server_tts_runtime_synthesizes_with_female_voice_override_when_bound_to_male_default() {
    use orchestrator_flow_rs::{TtsSynthesisRequest, TtsSynthesizerBackend};

    struct RecordingVieneuTts {
        model: &'static str,
        supported: &'static [&'static str],
        seen_voices: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }
    struct RecordingVieneuWorker {
        seen_voices: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }
    impl crate::providers::TtsWorker for RecordingVieneuWorker {
        fn synthesize(
            &mut self,
            request: &crate::providers::TtsSynthesisRequest,
            _cancelled: &std::sync::atomic::AtomicBool,
            on_pcm: &mut dyn FnMut(crate::audio::PcmF32Mono) -> Result<(), crate::providers::TtsError>,
        ) -> Result<(), crate::providers::TtsError> {
            self.seen_voices
                .lock()
                .unwrap()
                .push(request.selection.voice.clone());
            on_pcm(crate::audio::PcmF32Mono::new(vec![0.1; 2_400], 24_000))
        }
        fn reset(&mut self) -> Result<(), crate::providers::TtsError> {
            Ok(())
        }
    }
    impl crate::providers::TtsProvider for RecordingVieneuTts {
        fn adapter(&self) -> &'static str {
            "vieneu"
        }
        fn model_id(&self) -> Option<&str> {
            Some(self.model)
        }
        fn validate_diagnostic(
            &self,
            request: &crate::providers::TtsDiagnosticRequest,
        ) -> Result<(), crate::providers::TtsError> {
            let v = request.voice.as_deref().unwrap_or("");
            if !self.supported.contains(&v) {
                return Err(crate::providers::TtsError::UnsupportedVoice(v.into()));
            }
            Ok(())
        }
        fn open_worker(
            &self,
        ) -> Result<Box<dyn crate::providers::TtsWorker>, crate::providers::TtsError> {
            Ok(Box::new(RecordingVieneuWorker {
                seen_voices: self.seen_voices.clone(),
            }))
        }
    }

    let worker_cfg = crate::workers::WorkerRuntimeConfig {
        max_workers: 1,
        final_timeout: std::time::Duration::from_secs(10),
        cleanup_grace: std::time::Duration::from_secs(5),
        ..crate::workers::WorkerRuntimeConfig::default()
    };

    // A. VieNeu v3 Turbo runtime bound to male default `hai_dang`
    let turbo_seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let turbo_runtime = std::sync::Arc::new(crate::workers::TtsWorkerRuntime::new_with_binding(
        std::sync::Arc::new(RecordingVieneuTts {
            model: "vieneu_v3_turbo_fp32",
            supported: &["hai_dang", "ngoc_huyen", "mai_anh"],
            seen_voices: turbo_seen.clone(),
        }),
        worker_cfg.clone(),
        crate::providers::TtsBinding {
            voice: "hai_dang".into(),
            language: "vi-VN".into(),
        },
    ));

    let turbo_prov = ResolvedLlmProvider {
        provider_ref: "tts_vieneu_turbo".into(),
        name: "VieNeu Turbo".into(),
        source: "database",
        adapter: "vieneu".into(),
        model: "vieneu_v3_turbo_fp32:hai_dang".into(),
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

    let turbo_backend = CatalogMediaBackend::from_catalog(vec![turbo_prov])
        .with_tts_runtime("tts_vieneu_turbo", turbo_runtime.clone());

    let out_female = turbo_backend
        .synthesize_tts(&TtsSynthesisRequest {
            text: "Xin chào bằng giọng nữ".into(),
            filename: "turbo-female.wav".into(),
            voice: "vi-female".into(),
            language: "vi-VN".into(),
            speed: 1.0,
        })
        .await
        .unwrap();
    assert_eq!(out_female.voice, "ngoc_huyen");

    let out_named = turbo_backend
        .synthesize_tts(&TtsSynthesisRequest {
            text: "Xin chào từ Mai Anh".into(),
            filename: "turbo-maianh.wav".into(),
            voice: "mai_anh".into(),
            language: "vi-VN".into(),
            speed: 1.0,
        })
        .await
        .unwrap();
    assert_eq!(out_named.voice, "mai_anh");
    assert_eq!(
        turbo_seen.lock().unwrap().as_slice(),
        &["ngoc_huyen".to_string(), "mai_anh".to_string()]
    );
    let _ = turbo_runtime.shutdown_acknowledged();

    // B. VieNeu v3 Nano runtime bound to overlapping male voice `adam`
    let nano_seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let nano_runtime = std::sync::Arc::new(crate::workers::TtsWorkerRuntime::new_with_binding(
        std::sync::Arc::new(RecordingVieneuTts {
            model: "vieneu_v3_nano_fp32",
            supported: &["adam", "minh_quan", "my_duyen", "mai_anh", "ai_han"],
            seen_voices: nano_seen.clone(),
        }),
        worker_cfg,
        crate::providers::TtsBinding {
            voice: "adam".into(),
            language: "vi-VN".into(),
        },
    ));

    let nano_prov = ResolvedLlmProvider {
        provider_ref: "tts_vieneu_nano".into(),
        name: "VieNeu Nano".into(),
        source: "runtime",
        adapter: "vieneu".into(),
        model: "vieneu".into(),
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

    let nano_backend = CatalogMediaBackend::from_catalog(vec![nano_prov])
        .with_tts_runtime("tts_vieneu_nano", nano_runtime.clone());
    let nano_constraints = nano_backend.tts_constraints();
    assert_eq!(nano_constraints.default_voice, "adam");
    assert_eq!(
        nano_constraints.female_voices,
        vec!["my_duyen", "mai_anh", "truc_ly", "ai_han"]
    );

    let nano_female = nano_backend
        .synthesize_tts(&TtsSynthesisRequest {
            text: "Xin chào từ VieNeu Nano giọng nữ".into(),
            filename: "nano-female.wav".into(),
            voice: "female".into(),
            language: "vi-VN".into(),
            speed: 1.0,
        })
        .await
        .unwrap();
    assert_eq!(nano_female.voice, "my_duyen");
    assert_eq!(
        nano_seen.lock().unwrap().as_slice(),
        &["my_duyen".to_string()]
    );
    let _ = nano_runtime.shutdown_acknowledged();
}

#[test]
fn chillaudio_validate_and_synthesize_diagnostic_accept_supported_voices() {
    use crate::providers::TtsProvider;
    let provider = crate::providers::tts::ChillAudioWsProvider::new(crate::config::ChillAudioWsConfig {
        ws_url: "ws://127.0.0.1:9/ws".parse().unwrap(),
        app_key: crate::config::SecretString::new("key"),
        token: crate::config::SecretString::new("token"),
        voice: "BV075_streaming".into(),
        language: "vi".into(),
        timeout_ms: 5_000,
        preload: false,
    });
    for valid_voice in [
        "BV421_vivn_streaming",
        "vi_female_huong",
        "BV074_streaming",
        "BV075_streaming",
    ] {
        assert!(
            provider
                .validate_diagnostic(&crate::providers::TtsDiagnosticRequest {
                    text: "Xin chào".into(),
                    voice: Some(valid_voice.into()),
                    language: Some("vi".into()),
                })
                .is_ok(),
            "expected ChillAudio voice `{valid_voice}` to be accepted"
        );
    }
    let bad = provider.validate_diagnostic(&crate::providers::TtsDiagnosticRequest {
        text: "Xin chào".into(),
        voice: Some("unknown_voice".into()),
        language: Some("vi".into()),
    });
    assert!(matches!(
        bad,
        Err(crate::providers::TtsError::UnsupportedVoice(_))
    ));
}
