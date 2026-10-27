use serde_json::json;
use voice_agent_server::{
    config::{TtsInstanceConfig, VifastttsOnnxConfig},
    database::provider_config,
    providers::{
        compiled_local_runtime_adapter_registry, compiled_provider_adapter_registry,
        compiled_provider_registry,
        tts::{
            TtsBinding, TtsDiagnosticRequest, TtsError, TtsProvider,
            vifasttts::{
                ConfiguredVifasttts,
                assets::VifastttsAssets,
                dsp::{fade, istft, sharpen_mel},
                frontend::{chunk_text, features_from_text, normalize_text, pause_seconds},
                runtime::{emit_pcm_chunks, validate_selection},
            },
        },
    },
};

#[test]
fn vifasttts_selection_is_typed_registered_and_credential_free() {
    let instance: TtsInstanceConfig = toml::from_str(
        r#"
adapter = "vifasttts_onnx"
voice = "ngoc_huyen"
language = "vi-VN"
speed_percent = 100
preload = true
"#,
    )
    .unwrap();
    assert_eq!(instance.adapter(), "vifasttts_onnx");
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
    assert_eq!(registration.descriptor.capabilities.streaming, Some(false));
    assert_eq!(
        registration.descriptor.capabilities.provider_output_sample_rates,
        Some(&[24_000][..])
    );

    let canonical = provider_config::validate(
        instance.adapter(),
        &json!({"voice": "ngoc_huyen"}),
    )
    .unwrap();
    let value: serde_json::Value = serde_json::from_str(&canonical).unwrap();
    assert_eq!(value["language"], "vi-VN");
    assert_eq!(value["speed_percent"], 100);

    for invalid in [
        json!({"voice": "unknown"}),
        json!({"language": "en-US"}),
        json!({"speed_percent": 49}),
        json!({"speed_percent": 201}),
        json!({"api_key": "secret"}),
        json!({"num_threads": 4}),
        json!({"model": "other"}),
    ] {
        assert!(provider_config::validate(instance.adapter(), &invalid).is_err());
    }

    let default_cfg = VifastttsOnnxConfig::default();
    assert!(default_cfg.valid_selection());
}

#[test]
fn vifasttts_runtime_plan_excludes_preload_and_shares_pool_across_same_speed() {
    let planner = compiled_local_runtime_adapter_registry()
        .get("vifasttts_onnx")
        .expect("compiled ViFastTts planner");
    let plan = |speed_percent: u16, preload: bool| {
        planner
            .physical_plan(json!({
                "model": "vifasttts_ngochuyen_small",
                "num_threads": 2,
                "voice": "ngoc_huyen",
                "language": "vi-VN",
                "speed_percent": speed_percent,
                "preload": preload,
            }))
            .unwrap()
    };
    let default = plan(100, false);
    let preloaded = plan(100, true);
    let faster = plan(120, false);

    assert_eq!(
        default.resource_key("artifact", [1; 32], None, 2, 1).unwrap(),
        preloaded.resource_key("artifact", [1; 32], None, 2, 1).unwrap()
    );
    assert_ne!(
        default.resource_key("artifact", [1; 32], None, 2, 1).unwrap(),
        faster.resource_key("artifact", [1; 32], None, 2, 1).unwrap()
    );
}

#[test]
fn vifasttts_chunk_text_and_pause_seconds_follow_upstream_rules() {
    assert!(chunk_text("   ", 300).is_empty());
    let short = chunk_text("Xin chào bạn.", 300);
    assert_eq!(short, vec!["Xin chào bạn."]);
    assert_eq!(pause_seconds("Xin chào bạn."), 0.15);
    assert_eq!(pause_seconds("Xin chào,"), 0.08);
    assert_eq!(pause_seconds("Xin chào"), 0.05);

    let split = chunk_text("Câu một khá dài. Câu hai cũng khá dài, nhưng có dấu phẩy; và tiếp tục.", 25);
    assert!(split.len() >= 3);
    assert!(split.iter().all(|c| c.chars().count() <= 25));
}

#[test]
fn vifasttts_syllable_parser_handles_tones_qu_gi_diphthongs_and_short_a() {
    // Verify 7-ID layout: [onset, medial, nucleus, coda, tone, boundary, punct]
    // "xin chào." ->
    // "xin": onset=X(12), medial=NONE(0), nucleus=I(5), coda=N(2), tone=NGANG(0), boundary=WORD(1), punct=NONE(0)
    // "chào.": onset=CH(14), medial=NONE(0), nucleus=A(0), coda=W(10), tone=HUYEN(1), boundary=SENTENCE(3), punct=PERIOD(2)
    let ids = features_from_text("Xin chào.");
    assert_eq!(
        ids,
        vec![
            [12, 0, 5, 2, 0, 1, 0],
            [14, 0, 0, 10, 1, 3, 2],
        ]
    );

    // "quốc qua quyển" -> onset=Q(24)
    // "quốc": Q(24), NONE(0), UÔ(12), C(7), SAC(2), WORD(1), NONE(0)
    // "qua": Q(24), W(1), A(0), NONE(0), NGANG(0), WORD(1), NONE(0)
    // "quyển": Q(24), W(1), IÊ(11), N(2), HOI(3), WORD(1), NONE(0)
    let qu_ids = features_from_text("quốc qua quyển");
    assert_eq!(
        qu_ids,
        vec![
            [24, 0, 12, 7, 2, 1, 0],
            [24, 1, 0, 0, 0, 1, 0],
            [24, 1, 11, 2, 3, 1, 0],
        ]
    );

    // "gì giếng già" -> onset=GI(10)
    // "gì": GI(10), NONE(0), I(5), NONE(0), HUYEN(1)
    // "giếng": GI(10), NONE(0), IÊ(11), NG(3), SAC(2)
    // "già": GI(10), NONE(0), A(0), NONE(0), HUYEN(1)
    let gi_ids = features_from_text("gì giếng già");
    assert_eq!(
        gi_ids,
        vec![
            [10, 0, 5, 0, 1, 1, 0],
            [10, 0, 11, 3, 2, 1, 0],
            [10, 0, 0, 0, 1, 1, 0],
        ]
    );

    // "tuy" (W + I + NONE) vs "vui" (NONE + U + J)
    // "tay" (NONE + Ă + J) vs "tai" (NONE + A + J)
    let vowel_ids = features_from_text("tuy vui tay tai?");
    assert_eq!(
        vowel_ids,
        vec![
            [5, 1, 5, 0, 0, 1, 0],   // tuy: T(5), W(1), I(5), NONE(0)
            [4, 0, 9, 9, 0, 1, 0],   // vui: V(4), NONE(0), U(9), J(9)
            [5, 0, 1, 9, 0, 1, 0],   // tay: T(5), NONE(0), Ă(1), J(9)
            [5, 0, 0, 9, 0, 3, 3],   // tai?: T(5), NONE(0), A(0), J(9), NGANG(0), SENTENCE(3), QUESTION(3)
        ]
    );
}

#[test]
fn vifasttts_normalizer_and_oov_expand_numbers_dates_times_and_acronyms() {
    let norm = normalize_text("Hôm nay 25% lúc 10:30, giá 105 đồng.");
    assert!(norm.contains("hai mươi lăm phần trăm"), "got: {norm}");
    assert!(norm.contains("mười giờ ba mươi"), "got: {norm}");
    assert!(norm.contains("một trăm lẻ năm đồng"), "got: {norm}");

    // Acronym / OOV spelling: "GPU," -> ["giê", "pê", "u"] with comma on the last sub-syllable
    let oov_ids = features_from_text("GPU,");
    assert_eq!(oov_ids.len(), 3);
    assert_eq!(oov_ids[0][5..7], [1, 0]); // WORD, NONE
    assert_eq!(oov_ids[1][5..7], [1, 0]); // WORD, NONE
    assert_eq!(oov_ids[2][5..7], [2, 1]); // PHRASE, COMMA
}

#[test]
fn vifasttts_dsp_sharpen_mel_istft_and_fade_match_reference_math() {
    // 1. 3x3 impulse unsharp mask with alpha = 1.5
    let mut mel = vec![0.0_f32; 9];
    mel[4] = 1.0;
    sharpen_mel(&mut mel, 3, 3, 1.5);
    assert!((mel[4] - (1.0 + 1.5 * (8.0 / 9.0))).abs() < 1e-5);
    assert!((mel[0] - (-1.5 / 9.0)).abs() < 1e-5);

    // 2. iSTFT reconstruction of a DC signal (bin 0 of windowed constant 1.0)
    // For a constant x[n] = 1.0, each Hann-windowed frame has X[0] = sum(w) = 512.0,
    // X[1] = -256.0, and all other bins 0.0.
    let n_fft = 1024;
    let hop = 256;
    let freq_bins = n_fft / 2 + 1;
    let frames = 9;
    let mut spec_real = vec![0.0_f32; freq_bins * frames];
    let spec_imag = vec![0.0_f32; freq_bins * frames];
    for t in 0..frames {
        spec_real[t] = 512.0;
        spec_real[frames + t] = -256.0;
    }
    let wav = istft(&spec_real, &spec_imag, freq_bins, frames, n_fft, hop);
    assert_eq!(wav.len(), (frames - 1) * hop);
    for &sample in &wav[hop..wav.len() - hop] {
        assert!((sample - 1.0).abs() < 1e-4, "expected ~1.0, got {sample}");
    }

    // 3. Linear fade-in (10ms = 240 samples) and fade-out (20ms = 480 samples) at 24 kHz
    let mut audio = vec![1.0_f32; 1000];
    fade(&mut audio, 24_000, 10.0, 20.0);
    assert!((audio[0] - 0.0).abs() < 1e-6);
    assert!((audio[239] - 1.0).abs() < 1e-6);
    assert!((audio[500] - 1.0).abs() < 1e-6);
    assert!((audio[999] - 0.0).abs() < 1e-6);
}

#[test]
fn vifasttts_provider_validates_diagnostic_requests_and_emits_60ms_chunks() {
    use std::sync::atomic::AtomicBool;
    use voice_agent_server::config::RuntimeConfig;

    let tmp_dir = std::env::temp_dir().join(format!("vifasttts-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(&tmp_dir).unwrap();

    let acoustic = tmp_dir.join("acoustic.onnx");
    let vocoder = tmp_dir.join("vocoder.onnx");
    let config = tmp_dir.join("small_ngochuyen.yaml");
    let license = tmp_dir.join("LICENSE.md");

    // Missing files -> IncompatibleContract
    let missing_res = ConfiguredVifasttts::load(
        &VifastttsOnnxConfig::default(),
        &RuntimeConfig::default(),
        VifastttsAssets {
            acoustic: acoustic.clone(),
            vocoder: vocoder.clone(),
            config: config.clone(),
            license: license.clone(),
        },
    );
    assert!(matches!(missing_res, Err(TtsError::IncompatibleContract(_))));

    std::fs::write(&acoustic, b"dummy").unwrap();
    std::fs::write(&vocoder, b"dummy").unwrap();
    std::fs::write(&config, b"sample_rate: 24000\n").unwrap();
    std::fs::write(&license, b"MIT\n").unwrap();

    let provider = ConfiguredVifasttts::load(
        &VifastttsOnnxConfig::default(),
        &RuntimeConfig::default(),
        VifastttsAssets {
            acoustic,
            vocoder,
            config,
            license,
        },
    )
    .unwrap();

    assert_eq!(provider.adapter(), "vifasttts_onnx");
    assert!(
        provider
            .validate_diagnostic(&TtsDiagnosticRequest {
                text: "Xin chào.".into(),
                voice: None,
                language: None,
            })
            .is_ok()
    );
    assert!(matches!(
        provider.validate_diagnostic(&TtsDiagnosticRequest {
            text: "Xin chào.".into(),
            voice: Some("other_voice".into()),
            language: Some("vi-VN".into()),
        }),
        Err(TtsError::UnsupportedVoice(voice)) if voice == "other_voice"
    ));
    assert!(matches!(
        provider.validate_diagnostic(&TtsDiagnosticRequest {
            text: "Xin chào.".into(),
            voice: Some("ngoc_huyen".into()),
            language: Some("en-US".into()),
        }),
        Err(TtsError::UnsupportedLanguage(lang)) if lang == "en-US"
    ));

    assert!(validate_selection(&TtsBinding::readiness()).is_ok());
    assert!(
        validate_selection(&TtsBinding {
            voice: "ngoc_huyen".into(),
            language: "vi-VN".into(),
        })
        .is_ok()
    );
    assert!(matches!(
        validate_selection(&TtsBinding {
            voice: "unknown".into(),
            language: "vi-VN".into(),
        }),
        Err(TtsError::UnsupportedVoice(_))
    ));

    // 60ms chunking at 24kHz = 1,440 samples per chunk
    let samples = vec![0.25_f32; 3_000];
    let mut chunks = Vec::new();
    emit_pcm_chunks(&samples, &AtomicBool::new(false), &mut |pcm| {
        chunks.push((pcm.sample_rate_hz(), pcm.samples().len()));
        Ok(())
    })
    .unwrap();
    assert_eq!(chunks, vec![(24_000, 1_440), (24_000, 1_440), (24_000, 120)]);

    // Cancellation stops chunk emission immediately
    let mut cancelled_chunks = 0;
    let res = emit_pcm_chunks(&samples, &AtomicBool::new(true), &mut |_pcm| {
        cancelled_chunks += 1;
        Ok(())
    });
    assert!(matches!(res, Err(TtsError::Failed)));
    assert_eq!(cancelled_chunks, 0);

    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
#[ignore = "requires models/TTS/vifasttts and runtime/onnxruntime local assets"]
fn vifasttts_loads_local_onnx_models_and_synthesizes_24khz_pcm() {
    use std::path::PathBuf;
    use voice_agent_server::{
        config::{OnnxRuntimeConfig, RuntimeConfig},
        providers::tts::vifasttts::assets::resolve_assets_from,
    };

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models/TTS/vifasttts");
    let runtime_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../runtime");
    let assets = resolve_assets_from(&root).expect("resolve local ViFastTts assets");
    let provider = ConfiguredVifasttts::load(
        &VifastttsOnnxConfig::default(),
        &RuntimeConfig {
            onnx: OnnxRuntimeConfig {
                library: runtime_root.join(if cfg!(target_os = "macos") {
                    "onnxruntime/libonnxruntime.dylib"
                } else {
                    "onnxruntime/libonnxruntime.so"
                }),
                ..Default::default()
            },
            ..Default::default()
        },
        assets,
    )
    .expect("load prepared local ViFastTts provider");

    let mut chunks = Vec::new();
    provider
        .synthesize_stream("Xin chào bạn, hôm nay thời tiết rất đẹp.", &mut |pcm| {
            chunks.push(pcm);
            Ok(())
        })
        .expect("synthesize a Vietnamese smoke utterance");
    assert!(!chunks.is_empty());
    assert!(chunks.iter().all(|pcm| pcm.sample_rate_hz() == 24_000));
    assert!(
        chunks
            .iter()
            .all(|pcm| !pcm.samples().is_empty() && pcm.samples().len() <= 1_440)
    );
    assert!(
        chunks
            .iter()
            .flat_map(|pcm| pcm.samples())
            .all(|sample| sample.is_finite())
    );
}
