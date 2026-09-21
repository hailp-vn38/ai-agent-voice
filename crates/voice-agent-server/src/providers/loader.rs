use std::sync::Arc;

use sherpa_onnx::{OnlineRecognizer, OnlineRecognizerConfig};

use crate::{
    config::AppConfig,
    models::{Model, load_and_verify},
    providers::{ProviderLoadError, ProviderSet, asr::ZipformerAsrProvider, vad::LoadedSileroVad},
};

/// Startup-only concrete factory. Adding an adapter is one module and one match arm.
pub(crate) fn load_local(config: &AppConfig) -> Result<ProviderSet, ProviderLoadError> {
    let vad = load_vad(config)?;
    let asr = load_asr(config)?;
    Ok(ProviderSet::with_vad(vad, asr))
}

fn load_vad(
    config: &AppConfig,
) -> Result<Arc<dyn crate::providers::VadProvider>, ProviderLoadError> {
    match config.providers.vad.adapter.as_str() {
        "silero_onnx" => {
            let model = load_and_verify(
                &config.deployment.model_manifest,
                &config.providers.vad.model,
                "silero_onnx",
                &config.deployment,
            )?;
            let threads = config
                .providers
                .vad
                .silero_onnx
                .as_ref()
                .expect("validated config")
                .num_threads;
            Ok(Arc::new(
                LoadedSileroVad::load(
                    required(&model, "silero_vad.onnx")?,
                    config.runtime.onnx.library.clone(),
                    threads,
                )
                .map_err(|error| ProviderLoadError::Provider(error.to_string()))?,
            ))
        }
        adapter => Err(ProviderLoadError::UnsupportedAdapter {
            kind: "VAD",
            adapter: adapter.into(),
        }),
    }
}

fn load_asr(
    config: &AppConfig,
) -> Result<Arc<dyn crate::providers::AsrProvider>, ProviderLoadError> {
    match config.providers.asr.adapter.as_str() {
        "zipformer_sherpa" => {
            let model = load_and_verify(
                &config.deployment.model_manifest,
                &config.providers.asr.model,
                "zipformer_sherpa",
                &config.deployment,
            )?;
            let adapter = config
                .providers
                .asr
                .zipformer_sherpa
                .as_ref()
                .expect("validated config");
            let mut recognizer_config = OnlineRecognizerConfig::default();
            recognizer_config.model_config.transducer.encoder =
                Some(required(&model, "encoder.onnx")?);
            recognizer_config.model_config.transducer.decoder =
                Some(required(&model, "decoder.onnx")?);
            recognizer_config.model_config.transducer.joiner =
                Some(required(&model, "joiner.onnx")?);
            recognizer_config.model_config.tokens = Some(required(&model, "tokens.txt")?);
            recognizer_config.model_config.num_threads = adapter.num_threads;
            recognizer_config.model_config.provider = Some("cpu".into());
            recognizer_config.decoding_method = Some(adapter.decoding_method.clone());
            recognizer_config.enable_endpoint = false;
            let recognizer = Arc::new(
                OnlineRecognizer::create(&recognizer_config)
                    .ok_or(ProviderLoadError::Initialize("Zipformer ASR"))?,
            );
            drop(recognizer.create_stream());
            Ok(Arc::new(ZipformerAsrProvider { recognizer }))
        }
        adapter => Err(ProviderLoadError::UnsupportedAdapter {
            kind: "ASR",
            adapter: adapter.into(),
        }),
    }
}

fn required(model: &Model, name: &str) -> Result<String, ProviderLoadError> {
    model
        .artifact(name)
        .map(|artifact| artifact.path.display().to_string())
        .ok_or_else(|| ProviderLoadError::MissingArtifact(name.into()))
}
