use std::path::PathBuf;

use crate::{
    config::{AsrInstanceConfig, GipformerSherpaOfflineConfig, ZipformerSherpaConfig},
    models::ResolvedModel,
};

use super::compiled_provider_registry;

#[test]
fn zipformer_factory_rejects_a_resolved_model_missing_a_required_role() {
    let model = ResolvedModel::for_test(
        "zipformer_vi_streaming",
        "zipformer_sherpa",
        [("encoder", PathBuf::from("encoder.onnx"))],
    );

    let result = compiled_provider_registry()
        .asr_factory("zipformer_sherpa")
        .unwrap()
        .build(
            &AsrInstanceConfig::ZipformerSherpa(ZipformerSherpaConfig::default()),
            &crate::config::RuntimeConfig::default(),
            &model,
            480_000,
        );

    assert!(
        matches!(result, Err(crate::providers::ProviderLoadError::MissingArtifact(role)) if role == "decoder")
    );
}

#[test]
fn factories_reject_a_resolved_model_for_another_adapter() {
    let model = ResolvedModel::for_test("silero_vad_v5", "silero_onnx", []);

    let result = compiled_provider_registry()
        .asr_factory("zipformer_sherpa")
        .unwrap()
        .build(
            &AsrInstanceConfig::ZipformerSherpa(ZipformerSherpaConfig::default()),
            &crate::config::RuntimeConfig::default(),
            &model,
            480_000,
        );

    assert!(matches!(
        result,
        Err(crate::providers::ProviderLoadError::ModelAdapterMismatch { .. })
    ));
}

#[test]
fn gipformer_factory_requires_all_transducer_artifacts() {
    let models = vec![
        (
            "encoder",
            ResolvedModel::for_test("gipformer15_vi_int8", "gipformer_sherpa_offline", []),
        ),
        (
            "decoder",
            ResolvedModel::for_test(
                "gipformer15_vi_int8",
                "gipformer_sherpa_offline",
                [("encoder", PathBuf::from("encoder.onnx"))],
            ),
        ),
        (
            "joiner",
            ResolvedModel::for_test(
                "gipformer15_vi_int8",
                "gipformer_sherpa_offline",
                [
                    ("encoder", PathBuf::from("encoder.onnx")),
                    ("decoder", PathBuf::from("decoder.onnx")),
                ],
            ),
        ),
        (
            "tokens",
            ResolvedModel::for_test(
                "gipformer15_vi_int8",
                "gipformer_sherpa_offline",
                [
                    ("encoder", PathBuf::from("encoder.onnx")),
                    ("decoder", PathBuf::from("decoder.onnx")),
                    ("joiner", PathBuf::from("joiner.onnx")),
                ],
            ),
        ),
    ];

    for (missing_role, model) in models {
        let result = compiled_provider_registry()
            .asr_factory("gipformer_sherpa_offline")
            .unwrap()
            .build(
                &AsrInstanceConfig::GipformerSherpaOffline(GipformerSherpaOfflineConfig {
                    model: "gipformer15_vi_int8".into(),
                    ..Default::default()
                }),
                &crate::config::RuntimeConfig::default(),
                &model,
                480_000,
            );

        assert!(
            matches!(result, Err(crate::providers::ProviderLoadError::MissingArtifact(role)) if role == missing_role)
        );
    }
}

#[test]
fn gipformer_factory_rejects_a_resolved_model_for_another_adapter() {
    let model = ResolvedModel::for_test("zipformer_vi_streaming", "zipformer_sherpa", []);

    let result = compiled_provider_registry()
        .asr_factory("gipformer_sherpa_offline")
        .unwrap()
        .build(
            &AsrInstanceConfig::GipformerSherpaOffline(GipformerSherpaOfflineConfig {
                model: "gipformer15_vi_int8".into(),
                ..Default::default()
            }),
            &crate::config::RuntimeConfig::default(),
            &model,
            480_000,
        );

    assert!(matches!(
        result,
        Err(crate::providers::ProviderLoadError::ModelAdapterMismatch { .. })
    ));
}
