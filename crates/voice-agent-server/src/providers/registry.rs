use std::{path::Path, sync::Arc};

use sherpa_onnx::{
    OnlineRecognizer, OnlineRecognizerConfig, SileroVadModelConfig, VadModelConfig,
    VoiceActivityDetector,
};

use crate::{
    config::ProvidersConfig,
    providers::{asr::ZipformerAsrProvider, vad::LoadedSileroVad, ProviderLoadError, ProviderSet},
};

/// Validates artifacts and warms the fixed Phase-3 local adapter pair before socket bind.
pub(crate) fn load_local(config: &ProvidersConfig) -> Result<ProviderSet, ProviderLoadError> {
    if config.vad.adapter != "silero_onnx" {
        return Err(ProviderLoadError::UnsupportedAdapter {
            kind: "VAD",
            adapter: config.vad.adapter.clone(),
        });
    }
    if config.asr.adapter != "zipformer_sherpa" {
        return Err(ProviderLoadError::UnsupportedAdapter {
            kind: "ASR",
            adapter: config.asr.adapter.clone(),
        });
    }
    require_file(&config.vad.model)?;
    for artifact in ["encoder.onnx", "decoder.onnx", "joiner.onnx", "tokens.txt"] {
        require_file(&config.asr.model_dir.join(artifact))?;
    }
    let vad_config = VadModelConfig {
        sample_rate: 16_000,
        num_threads: config.vad.num_threads,
        provider: Some("cpu".into()),
        silero_vad: SileroVadModelConfig {
            model: Some(config.vad.model.display().to_string()),
            threshold: 0.5,
            min_silence_duration: 0.6,
            min_speech_duration: 0.18,
            window_size: 512,
            max_speech_duration: 30.0,
        },
        ..Default::default()
    };
    drop(
        VoiceActivityDetector::create(&vad_config, 60.0)
            .ok_or(ProviderLoadError::Initialize("Silero VAD"))?,
    );

    let mut asr_config = OnlineRecognizerConfig::default();
    asr_config.model_config.transducer.encoder = Some(path(&config.asr.model_dir, "encoder.onnx"));
    asr_config.model_config.transducer.decoder = Some(path(&config.asr.model_dir, "decoder.onnx"));
    asr_config.model_config.transducer.joiner = Some(path(&config.asr.model_dir, "joiner.onnx"));
    asr_config.model_config.tokens = Some(path(&config.asr.model_dir, "tokens.txt"));
    asr_config.model_config.num_threads = config.asr.num_threads;
    asr_config.model_config.provider = Some("cpu".into());
    asr_config.decoding_method = Some("greedy_search".into());
    asr_config.enable_endpoint = false;
    let recognizer = Arc::new(
        OnlineRecognizer::create(&asr_config)
            .ok_or(ProviderLoadError::Initialize("Zipformer ASR"))?,
    );
    drop(recognizer.create_stream());
    Ok(ProviderSet::with_vad(
        Arc::new(LoadedSileroVad { config: vad_config }),
        Arc::new(ZipformerAsrProvider { recognizer }),
    ))
}

fn require_file(path: &Path) -> Result<(), ProviderLoadError> {
    path.is_file()
        .then_some(())
        .ok_or_else(|| ProviderLoadError::MissingArtifact(path.display().to_string()))
}
fn path(directory: &Path, file: &str) -> String {
    directory.join(file).display().to_string()
}
