//! The fixed startup registry is the only provider selection point.

use std::sync::Arc;

use sherpa_onnx::{OnlineRecognizer, OnlineRecognizerConfig};

use crate::{
    config::{AsrProviderConfig, RuntimeConfig, VadProviderConfig},
    models::ResolvedModel,
    providers::{
        AsrProvider, ProviderLoadError, VadProvider, asr::ZipformerAsrProvider,
        vad::LoadedSileroVad,
    },
};

pub trait VadFactory: Send + Sync {
    fn adapter(&self) -> &'static str;
    fn build(
        &self,
        config: &VadProviderConfig,
        runtime: &RuntimeConfig,
        model: &ResolvedModel,
    ) -> Result<Arc<dyn VadProvider>, ProviderLoadError>;
}

pub trait AsrFactory: Send + Sync {
    fn adapter(&self) -> &'static str;
    fn build(
        &self,
        config: &AsrProviderConfig,
        model: &ResolvedModel,
    ) -> Result<Arc<dyn AsrProvider>, ProviderLoadError>;
}

/// Factories are linked into the binary. There is no runtime code discovery or plugin loading.
pub struct ProviderRegistry {
    vad: &'static [&'static dyn VadFactory],
    asr: &'static [&'static dyn AsrFactory],
}

impl ProviderRegistry {
    pub fn vad_factory(&self, adapter: &str) -> Result<&'static dyn VadFactory, ProviderLoadError> {
        self.vad
            .iter()
            .copied()
            .find(|factory| factory.adapter() == adapter)
            .ok_or_else(|| ProviderLoadError::UnsupportedAdapter {
                kind: "VAD",
                adapter: adapter.into(),
            })
    }

    pub fn asr_factory(&self, adapter: &str) -> Result<&'static dyn AsrFactory, ProviderLoadError> {
        self.asr
            .iter()
            .copied()
            .find(|factory| factory.adapter() == adapter)
            .ok_or_else(|| ProviderLoadError::UnsupportedAdapter {
                kind: "ASR",
                adapter: adapter.into(),
            })
    }
}

struct SileroOnnxFactory;

impl VadFactory for SileroOnnxFactory {
    fn adapter(&self) -> &'static str {
        "silero_onnx"
    }

    fn build(
        &self,
        config: &VadProviderConfig,
        runtime: &RuntimeConfig,
        model: &ResolvedModel,
    ) -> Result<Arc<dyn VadProvider>, ProviderLoadError> {
        validate_model_adapter(model, self.adapter())?;
        let options = config.silero_onnx.as_ref().ok_or_else(|| {
            ProviderLoadError::Configuration("silero_onnx options are required".into())
        })?;
        Ok(Arc::new(
            LoadedSileroVad::load(
                required(model, "vad")?,
                runtime.onnx.library.clone(),
                options.num_threads,
            )
            .map_err(|error| ProviderLoadError::Provider(error.to_string()))?,
        ))
    }
}

struct ZipformerSherpaFactory;

impl AsrFactory for ZipformerSherpaFactory {
    fn adapter(&self) -> &'static str {
        "zipformer_sherpa"
    }

    fn build(
        &self,
        config: &AsrProviderConfig,
        model: &ResolvedModel,
    ) -> Result<Arc<dyn AsrProvider>, ProviderLoadError> {
        validate_model_adapter(model, self.adapter())?;
        let options = config.zipformer_sherpa.as_ref().ok_or_else(|| {
            ProviderLoadError::Configuration("zipformer_sherpa options are required".into())
        })?;
        let mut recognizer_config = OnlineRecognizerConfig::default();
        recognizer_config.model_config.transducer.encoder = Some(required(model, "encoder")?);
        recognizer_config.model_config.transducer.decoder = Some(required(model, "decoder")?);
        recognizer_config.model_config.transducer.joiner = Some(required(model, "joiner")?);
        recognizer_config.model_config.tokens = Some(required(model, "tokens")?);
        recognizer_config.model_config.num_threads = options.num_threads;
        recognizer_config.model_config.provider = Some("cpu".into());
        recognizer_config.decoding_method = Some(options.decoding_method.clone());
        recognizer_config.enable_endpoint = false;
        let recognizer = Arc::new(
            OnlineRecognizer::create(&recognizer_config)
                .ok_or(ProviderLoadError::Initialize("Zipformer ASR"))?,
        );
        drop(recognizer.create_stream());
        Ok(Arc::new(ZipformerAsrProvider { recognizer }))
    }
}

static SILERO_ONNX_FACTORY: SileroOnnxFactory = SileroOnnxFactory;
static ZIPFORMER_SHERPA_FACTORY: ZipformerSherpaFactory = ZipformerSherpaFactory;
static VAD_FACTORIES: [&dyn VadFactory; 1] = [&SILERO_ONNX_FACTORY];
static ASR_FACTORIES: [&dyn AsrFactory; 1] = [&ZIPFORMER_SHERPA_FACTORY];
static COMPILED_PROVIDER_REGISTRY: ProviderRegistry = ProviderRegistry {
    vad: &VAD_FACTORIES,
    asr: &ASR_FACTORIES,
};

pub fn compiled_provider_registry() -> &'static ProviderRegistry {
    &COMPILED_PROVIDER_REGISTRY
}

fn validate_model_adapter(
    model: &ResolvedModel,
    expected: &'static str,
) -> Result<(), ProviderLoadError> {
    if model.adapter() == expected {
        Ok(())
    } else {
        Err(ProviderLoadError::ModelAdapterMismatch {
            model: model.identity().into(),
            expected,
            actual: model.adapter().into(),
        })
    }
}

fn required(model: &ResolvedModel, role: &str) -> Result<String, ProviderLoadError> {
    model
        .artifact(role)
        .map(|path| path.display().to_string())
        .ok_or_else(|| ProviderLoadError::MissingArtifact(role.into()))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::{config::AsrProviderConfig, models::ResolvedModel};

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
            .build(&AsrProviderConfig::default(), &model);

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
            .build(&AsrProviderConfig::default(), &model);

        assert!(matches!(
            result,
            Err(crate::providers::ProviderLoadError::ModelAdapterMismatch { .. })
        ));
    }
}
