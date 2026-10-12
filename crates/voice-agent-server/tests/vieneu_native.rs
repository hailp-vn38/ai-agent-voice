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
adapter = "vieneu_native"
voice = "hai_dang"
language = "vi-VN"
preload = true
"#,
    )
    .unwrap();
    assert_eq!(instance.adapter(), "vieneu_native");
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
    assert_eq!(value["language"], "vi-VN");
    for invalid in [
        json!({"voice":"unknown"}),
        json!({"language":"en-US"}),
        json!({"api_key":"secret"}),
        json!({"num_threads":4}),
        json!({"model":"other"}),
    ] {
        assert!(provider_config::validate(instance.adapter(), &invalid).is_err());
    }
}

#[test]
fn native_voice_changes_share_one_physical_replica() {
    let planner = compiled_local_runtime_adapter_registry()
        .get("vieneu_native")
        .unwrap();
    let plan = |voice, preload| {
        planner
            .physical_plan(json!({"voice":voice,"preload":preload}))
            .unwrap()
    };
    let first = plan("hai_dang", false);
    let second = plan("mai_anh", true);
    assert_eq!(
        first.physical_capacity(),
        voice_agent_server::services::provider_runtime::PhysicalCapacity::Replicas(1)
    );
    assert_eq!(
        first.resource_key("pinned", [1; 32], None, 2, 1).unwrap(),
        second.resource_key("pinned", [1; 32], None, 2, 1).unwrap()
    );
}

#[test]
#[ignore = "requires prepared native VieNeu model files and ONNX Runtime"]
fn retained_native_worker_streams_switches_voice_and_recovers_after_cancellation() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::env::set_current_dir(&root).unwrap();
    let assets =
        voice_agent_server::providers::tts::vieneu_native::assets::resolve_assets().unwrap();
    let frontend =
        voice_agent_server::providers::tts::vieneu_native::text::VieneuTextFrontend::load(
            &assets.g2p_dictionary,
        )
        .unwrap();
    let contract =
        voice_agent_server::providers::tts::vieneu_native::contract::VieneuContract::load(&assets)
            .unwrap();
    let oracle: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/vieneu_native.json")).unwrap();
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
    let instance = TtsInstanceConfig::VieneuNative(VieneuNativeConfig::default());
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
