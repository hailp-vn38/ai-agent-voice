use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::json;
use voice_agent_server::{
    config::{AppConfig, TtsInstanceConfig, VieneuNativeConfig},
    database::provider_config,
    providers::{
        TtsBinding, TtsDiagnosticRequest, TtsError, TtsSynthesisRequest,
        compiled_local_runtime_adapter_registry, compiled_provider_adapter_registry,
        compiled_provider_registry,
    },
};

#[test]
fn native_selection_is_typed_registered_and_credential_free() {
    let instance: TtsInstanceConfig = toml::from_str(
        r#"
adapter = "vieneu"
voice = "hai_dang"
language = "vi-VN"
preload = true
"#,
    )
    .unwrap();
    assert_eq!(instance.adapter(), "vieneu");
    assert!(instance.preload());
    assert!(
        compiled_provider_registry()
            .tts_factory(instance.adapter())
            .is_ok()
    );
    let registration = compiled_provider_adapter_registry()
        .registration(instance.adapter())
        .unwrap();
    assert!(registration.assets.is_some());
    assert_eq!(registration.descriptor.capabilities.streaming, Some(true));
    let canonical =
        provider_config::validate(instance.adapter(), &json!({"voice":"hai_dang"})).unwrap();
    let value: serde_json::Value = serde_json::from_str(&canonical).unwrap();
    assert_eq!(value["model"], "vieneu_v3_turbo_fp32");
    assert_eq!(value["language"], "vi-VN");
    let nano_canonical = provider_config::validate(
        instance.adapter(),
        &json!({"model":"vieneu_v3_nano_fp32"}),
    )
    .unwrap();
    let nano_value: serde_json::Value = serde_json::from_str(&nano_canonical).unwrap();
    assert_eq!(nano_value["model"], "vieneu_v3_nano_fp32");
    assert_eq!(nano_value["voice"], "minh_quan");
    assert_eq!(nano_value["language"], "vi-VN");
    let factory = compiled_provider_registry()
        .tts_factory(instance.adapter())
        .unwrap();
    assert_eq!(
        factory.model_identity(&instance).unwrap(),
        Some("vieneu_v3_turbo_fp32")
    );
    let nano_instance = TtsInstanceConfig::Vieneu(VieneuNativeConfig {
        model: "vieneu_v3_nano_fp32".into(),
        voice: "minh_quan".into(),
        ..Default::default()
    });
    assert_eq!(
        factory.model_identity(&nano_instance).unwrap(),
        Some("vieneu_v3_nano_fp32")
    );
    for invalid in [
        json!({"voice":"unknown"}),
        json!({"model":"vieneu_v3_turbo_fp32","voice":"minh_quan"}),
        json!({"model":"vieneu_v3_nano_fp32","voice":"hai_dang"}),
        json!({"language":"en-US"}),
        json!({"api_key":"secret"}),
        json!({"num_threads":4}),
        json!({"model":"other"}),
    ] {
        assert!(provider_config::validate(instance.adapter(), &invalid).is_err());
    }
}

#[test]
fn native_voice_changes_share_configured_worker_pool() {
    let planner = compiled_local_runtime_adapter_registry()
        .get("vieneu")
        .unwrap();
    let plan = |model, voice, preload| {
        planner
            .physical_plan(json!({"model":model,"voice":voice,"preload":preload}))
            .unwrap()
    };
    let first = plan("vieneu_v3_turbo_fp32", "hai_dang", false);
    let second = plan("vieneu_v3_turbo_fp32", "mai_anh", true);
    let nano_first = plan("vieneu_v3_nano_fp32", "minh_quan", false);
    let nano_second = plan("vieneu_v3_nano_fp32", "adam", true);
    assert_eq!(
        first.physical_capacity(),
        voice_agent_server::services::provider_runtime::PhysicalCapacity::FollowsLogicalCapacity
    );
    assert_eq!(first.physical_capacity().resolve(1), 1);
    assert_eq!(first.physical_capacity().resolve(2), 2);
    assert_eq!(
        voice_agent_server::providers::configured_physical_replicas(
            "vieneu",
            json!({"voice":"hai_dang"}),
            &Default::default(),
        ),
        None,
        "deployment startup must also use the configured worker count"
    );
    assert_eq!(
        voice_agent_server::providers::configured_physical_replicas(
            "vieneu",
            json!({"model":"vieneu_v3_nano_fp32","voice":"minh_quan"}),
            &Default::default(),
        ),
        None,
        "Nano model must preserve model in effective_local_config and use configured worker count"
    );
    assert_eq!(
        first.resource_key("pinned", [1; 32], None, 2, 2).unwrap(),
        second.resource_key("pinned", [1; 32], None, 2, 2).unwrap()
    );
    assert_eq!(
        nano_first
            .resource_key("pinned", [1; 32], None, 2, 2)
            .unwrap(),
        nano_second
            .resource_key("pinned", [1; 32], None, 2, 2)
            .unwrap()
    );
    assert_ne!(
        first.resource_key("pinned", [1; 32], None, 2, 2).unwrap(),
        nano_first
            .resource_key("pinned", [1; 32], None, 2, 2)
            .unwrap()
    );
}

#[test]
#[ignore = "requires prepared native VieNeu model files and ONNX Runtime"]
fn native_background_pool_runs_successive_jobs() {
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };
    use voice_agent_server::workers::{TtsWorkerEvent, TtsWorkerRuntime, WorkerRuntimeConfig};

    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::env::set_current_dir(&root).unwrap();
    let config = AppConfig::parse_and_resolve(root.join("config.toml")).unwrap();
    let provider = compiled_provider_registry()
        .tts_factory("vieneu")
        .unwrap()
        .build(
            &TtsInstanceConfig::Vieneu(VieneuNativeConfig::default()),
            &config.runtime,
        )
        .unwrap();
    let runtime = TtsWorkerRuntime::new_with_binding(
        Arc::clone(&provider),
        WorkerRuntimeConfig {
            max_workers: 2,
            voice_reserved_capacity: 1,
            final_timeout: Duration::from_secs(60),
            ..Default::default()
        },
        TtsBinding {
            voice: "hai_dang".into(),
            language: "vi-VN".into(),
        },
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
                "native background synthesis stalled"
            );
            match runtime.poll(lease).unwrap() {
                Some(TtsWorkerEvent::Pcm(pcm)) => samples += pcm.samples().len(),
                Some(TtsWorkerEvent::Finished) => break,
                Some(event) => panic!("unexpected native event: {event:?}"),
                None => std::thread::sleep(Duration::from_millis(5)),
            }
        }
        assert!(samples > 0);
        assert_eq!(runtime.active_leases(), 0);
    }
}

#[test]
#[ignore = "requires prepared native VieNeu model files and ONNX Runtime"]
fn retained_native_worker_streams_switches_voice_and_recovers_after_cancellation() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::env::set_current_dir(&root).unwrap();
    let assets =
        voice_agent_server::providers::tts::vieneu::assets::resolve_assets().unwrap();
    let frontend =
        voice_agent_server::providers::tts::vieneu::text::VieneuTextFrontend::load(
            &assets.g2p_dictionary,
        )
        .unwrap();
    let contract =
        voice_agent_server::providers::tts::vieneu::contract::VieneuContract::load(&assets)
            .unwrap();
    let oracle: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/vieneu.json")).unwrap();
    for case in oracle["cases"].as_array().unwrap() {
        let phonemes = frontend.phonemize(case["text"].as_str().unwrap()).unwrap();
        assert_eq!(phonemes, case["phonemes"].as_str().unwrap());
        let tokens = contract.tokenizer.encode(phonemes.as_str(), false).unwrap();
        assert_eq!(
            serde_json::to_value(tokens.get_ids()).unwrap(),
            case["token_ids"]
        );
    }
    let mut config = AppConfig::parse_and_resolve(root.join("config.toml")).unwrap();
    config.runtime.onnx.library = root.join("runtime/onnxruntime/libonnxruntime.so");
    let instance = TtsInstanceConfig::Vieneu(VieneuNativeConfig::default());
    let provider = compiled_provider_registry()
        .tts_factory(instance.adapter())
        .unwrap()
        .build(&instance, &config.runtime)
        .unwrap();
    provider
        .validate_diagnostic(&TtsDiagnosticRequest {
            text: "Xin chào.".into(),
            voice: Some("mai_anh".into()),
            language: Some("vi-VN".into()),
        })
        .unwrap();
    let mut worker = provider.open_worker().unwrap();
    worker.warmup().unwrap();
    let request = |voice: &str| TtsSynthesisRequest {
        text: "Xin chào, tôi là trợ lý giọng nói tiếng Việt.".into(),
        selection: TtsBinding {
            voice: voice.into(),
            language: "vi-VN".into(),
        },
    };
    let cancelled = AtomicBool::new(true);
    assert!(
        worker
            .synthesize(&request("hai_dang"), &cancelled, &mut |_| panic!(
                "cancelled request emitted PCM"
            ))
            .is_err()
    );
    cancelled.store(false, Ordering::Release);
    assert!(
        worker
            .synthesize(&request("hai_dang"), &cancelled, &mut |_| {
                cancelled.store(true, Ordering::Release);
                Ok(())
            })
            .is_err()
    );
    cancelled.store(false, Ordering::Release);
    assert!(
        worker
            .synthesize(&request("hai_dang"), &cancelled, &mut |_| Err(
                TtsError::Failed
            ))
            .is_err()
    );
    let mut samples = Vec::new();
    for voice in ["hai_dang", "mai_anh"] {
        let mut chunks = 0;
        let mut utterance = Vec::new();
        worker
            .synthesize(&request(voice), &cancelled, &mut |pcm| {
                assert_eq!(pcm.sample_rate_hz(), 48_000);
                assert!(!pcm.samples().is_empty());
                assert!(pcm.samples().len() <= 4 * 3840);
                assert!(pcm.samples().iter().all(|value| value.is_finite()));
                utterance.extend_from_slice(pcm.samples());
                chunks += 1;
                Ok(())
            })
            .unwrap();
        assert!(chunks > 1);
        let rms = (utterance.iter().map(|value| value * value).sum::<f32>()
            / utterance.len() as f32)
            .sqrt();
        assert!(rms > 0.001, "{voice}: silent synthesis");
        eprintln!(
            "{voice}: {chunks} chunks, {:.2}s PCM, RMS={rms:.4}",
            utterance.len() as f32 / 48000.0
        );
        samples.extend(utterance);
        worker.reset().unwrap();
    }
    if let Some(path) = std::env::var_os("VIENEU_NATIVE_QUALIFICATION_WAV") {
        let mut writer = hound::WavWriter::create(
            path,
            hound::WavSpec {
                channels: 1,
                sample_rate: 48_000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for sample in samples {
            writer
                .write_sample((sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
                .unwrap();
        }
        writer.finalize().unwrap();
    }
}

#[test]
#[ignore = "requires prepared native VieNeu v3 Nano model files and ONNX Runtime"]
fn retained_nano_worker_synthesizes_24khz_switches_voice_and_recovers_after_cancellation() {
    use voice_agent_server::providers::tts::vieneu::config::NANO_MODEL_ID;

    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::env::set_current_dir(&root).unwrap();
    let assets =
        voice_agent_server::providers::tts::vieneu::assets::resolve_nano_assets().unwrap();
    let contract =
        voice_agent_server::providers::tts::vieneu::contract::NanoContract::load(&assets).unwrap();
    assert_eq!(contract.config.sample_rate, 24_000);
    let mut config = AppConfig::parse_and_resolve(root.join("config.toml")).unwrap();
    config.runtime.onnx.library = root.join("runtime/onnxruntime/libonnxruntime.so");
    let instance = TtsInstanceConfig::Vieneu(VieneuNativeConfig {
        model: NANO_MODEL_ID.into(),
        voice: "minh_quan".into(),
        ..Default::default()
    });
    let provider = compiled_provider_registry()
        .tts_factory(instance.adapter())
        .unwrap()
        .build(&instance, &config.runtime)
        .unwrap();
    provider
        .validate_diagnostic(&TtsDiagnosticRequest {
            text: "Xin chào.".into(),
            voice: Some("adam".into()),
            language: Some("vi-VN".into()),
        })
        .unwrap();
    assert!(
        provider
            .validate_diagnostic(&TtsDiagnosticRequest {
                text: "Xin chào.".into(),
                voice: Some("hai_dang".into()),
                language: Some("vi-VN".into()),
            })
            .is_err()
    );
    let mut worker = provider.open_worker().unwrap();
    worker.warmup().unwrap();
    let request = |voice: &str| TtsSynthesisRequest {
        text: "Xin chào, tôi là trợ lý giọng nói tiếng Việt.".into(),
        selection: TtsBinding {
            voice: voice.into(),
            language: "vi-VN".into(),
        },
    };
    let cancelled = AtomicBool::new(true);
    assert!(
        worker
            .synthesize(&request("minh_quan"), &cancelled, &mut |_| panic!(
                "cancelled request emitted PCM"
            ))
            .is_err()
    );
    cancelled.store(false, Ordering::Release);
    for voice in ["minh_quan", "adam"] {
        let mut chunks = 0;
        let mut utterance = Vec::new();
        worker
            .synthesize(&request(voice), &cancelled, &mut |pcm| {
                assert_eq!(pcm.sample_rate_hz(), 24_000);
                assert!(!pcm.samples().is_empty());
                assert!(pcm.samples().iter().all(|value| value.is_finite()));
                utterance.extend_from_slice(pcm.samples());
                chunks += 1;
                Ok(())
            })
            .unwrap();
        assert!(chunks >= 1);
        let rms = (utterance.iter().map(|value| value * value).sum::<f32>()
            / utterance.len() as f32)
            .sqrt();
        assert!(rms > 0.001, "{voice}: silent synthesis");
        worker.reset().unwrap();
    }
}

