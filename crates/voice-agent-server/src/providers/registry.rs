//! The fixed startup registry is the only provider selection point.

use std::sync::Arc;

use sherpa_onnx::{OnlineRecognizer, OnlineRecognizerConfig};

use crate::{
    config::{
        AsrProviderConfig, LlmProviderConfig, RuntimeConfig, TtsProviderConfig, VadProviderConfig,
        ZeroTtsOnnxConfig,
    },
    models::ResolvedModel,
    providers::{
        AsrProvider, LlmProvider, ProviderLoadError, TtsProvider, VadProvider,
        asr::ZipformerAsrProvider,
        llm::ConfiguredOpenAiLlm,
        tts::{ConfiguredZeroTts, ZeroTtsArtifacts},
        vad::LoadedSileroVad,
    },
};

pub trait VadFactory: Send + Sync {
    fn adapter(&self) -> &'static str;
    fn model_identity<'a>(
        &self,
        config: &'a VadProviderConfig,
    ) -> Result<&'a str, ProviderLoadError>;
    fn build(
        &self,
        config: &VadProviderConfig,
        runtime: &RuntimeConfig,
        model: &ResolvedModel,
    ) -> Result<Arc<dyn VadProvider>, ProviderLoadError>;
}

pub trait AsrFactory: Send + Sync {
    fn adapter(&self) -> &'static str;
    fn model_identity<'a>(
        &self,
        config: &'a AsrProviderConfig,
    ) -> Result<&'a str, ProviderLoadError>;
    fn build(
        &self,
        config: &AsrProviderConfig,
        model: &ResolvedModel,
    ) -> Result<Arc<dyn AsrProvider>, ProviderLoadError>;
}

pub trait LlmFactory: Send + Sync {
    fn adapter(&self) -> &'static str;
    fn build(&self, config: &LlmProviderConfig) -> Result<Arc<dyn LlmProvider>, ProviderLoadError>;
}

pub trait TtsFactory: Send + Sync {
    fn adapter(&self) -> &'static str;
    fn model_identity<'a>(
        &self,
        config: &'a TtsProviderConfig,
    ) -> Result<&'a str, ProviderLoadError>;
    fn build(
        &self,
        config: &ZeroTtsOnnxConfig,
        runtime: &RuntimeConfig,
        model: &ResolvedModel,
    ) -> Result<Arc<dyn TtsProvider>, ProviderLoadError>;
}

/// Factories are linked into the binary. There is no runtime code discovery or plugin loading.
pub struct ProviderRegistry {
    vad: &'static [&'static dyn VadFactory],
    asr: &'static [&'static dyn AsrFactory],
    llm: &'static [&'static dyn LlmFactory],
    tts: &'static [&'static dyn TtsFactory],
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

    pub fn llm_factory(&self, adapter: &str) -> Result<&'static dyn LlmFactory, ProviderLoadError> {
        self.llm
            .iter()
            .copied()
            .find(|factory| factory.adapter() == adapter)
            .ok_or_else(|| ProviderLoadError::UnsupportedAdapter {
                kind: "LLM",
                adapter: adapter.into(),
            })
    }

    pub fn tts_factory(&self, adapter: &str) -> Result<&'static dyn TtsFactory, ProviderLoadError> {
        self.tts
            .iter()
            .copied()
            .find(|factory| factory.adapter() == adapter)
            .ok_or_else(|| ProviderLoadError::UnsupportedAdapter {
                kind: "TTS",
                adapter: adapter.into(),
            })
    }
}

struct SileroOnnxFactory;

impl VadFactory for SileroOnnxFactory {
    fn adapter(&self) -> &'static str {
        "silero_onnx"
    }

    fn model_identity<'a>(
        &self,
        config: &'a VadProviderConfig,
    ) -> Result<&'a str, ProviderLoadError> {
        Ok(&config
            .silero_onnx
            .as_ref()
            .ok_or_else(|| {
                ProviderLoadError::Configuration("silero_onnx options are required".into())
            })?
            .model)
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

struct OpenAiFactory;

impl LlmFactory for OpenAiFactory {
    fn adapter(&self) -> &'static str {
        "openai"
    }

    fn build(&self, config: &LlmProviderConfig) -> Result<Arc<dyn LlmProvider>, ProviderLoadError> {
        let options = config.openai.as_ref().ok_or_else(|| {
            ProviderLoadError::Configuration("openai options are required".into())
        })?;
        if options.model.trim().is_empty() {
            return Err(ProviderLoadError::Configuration(
                "OpenAI model is required".into(),
            ));
        }
        Ok(Arc::new(
            ConfiguredOpenAiLlm::build(
                options.api_key.expose(),
                options.base_url.as_str(),
                &options.model,
                options.timeout_ms.div_ceil(1_000),
            )
            .map_err(|_| {
                ProviderLoadError::Provider("OpenAI provider initialization failed".into())
            })?,
        ))
    }
}

struct ZeroTtsOnnxFactory;

impl TtsFactory for ZeroTtsOnnxFactory {
    fn adapter(&self) -> &'static str {
        "zerotts_onnx"
    }

    fn model_identity<'a>(
        &self,
        config: &'a TtsProviderConfig,
    ) -> Result<&'a str, ProviderLoadError> {
        Ok(&config
            .zerotts_onnx
            .as_ref()
            .ok_or_else(|| {
                ProviderLoadError::Configuration("zerotts_onnx options are required".into())
            })?
            .model)
    }

    fn build(
        &self,
        config: &ZeroTtsOnnxConfig,
        runtime: &RuntimeConfig,
        model: &ResolvedModel,
    ) -> Result<Arc<dyn TtsProvider>, ProviderLoadError> {
        validate_model_adapter(model, self.adapter())?;
        if config.model != "zerotts_default" || config.voice != "maichi" || config.num_threads <= 0
        {
            return Err(ProviderLoadError::Configuration(
                "ZeroTTS requires model `zerotts_default`, voice `maichi`, and positive thread count"
                    .into(),
            ));
        }
        if model.identity() != config.model {
            return Err(ProviderLoadError::Configuration(
                "ZeroTTS resolved model does not match configured logical model".into(),
            ));
        }
        for role in ZEROTTS_REQUIRED_ARTIFACT_ROLES {
            required(model, role)?;
        }
        Ok(Arc::new(
            ConfiguredZeroTts::load(
                ZeroTtsArtifacts {
                    config: model.artifact("config").expect("required above"),
                    tokenizer: model.artifact("tokenizer").expect("required above"),
                    voice: model.artifact("voice").expect("required above"),
                    text_encoder: model.artifact("text_encoder").expect("required above"),
                    prefix_step: model.artifact("prefix_step").expect("required above"),
                    local_frame_decode: model
                        .artifact("local_frame_decode")
                        .expect("required above"),
                    codec_decode_full: model.artifact("codec_decode_full").expect("required above"),
                    codec_decode_step: model.artifact("codec_decode_step").expect("required above"),
                    codec_shared_data: model.artifact("codec_shared_data").expect("required above"),
                    codec_metadata: model.artifact("codec_metadata").expect("required above"),
                    silence_frame: model.artifact("silence_frame").expect("required above"),
                },
                &runtime.onnx.library,
                config.num_threads,
                config.delivery_mode,
            )
            .map_err(|error| ProviderLoadError::Provider(error.to_string()))?,
        ))
    }
}

const ZEROTTS_REQUIRED_ARTIFACT_ROLES: &[&str] = &[
    "config",
    "tokenizer",
    "null_voice",
    "voices_index",
    "voice",
    "text_encoder",
    "prefix_step",
    "local_frame_decode",
    "codec_decode_full",
    "codec_decode_step",
    "codec_shared_data",
    "codec_metadata",
    "codec_license",
    "silence_frame",
];

impl AsrFactory for ZipformerSherpaFactory {
    fn adapter(&self) -> &'static str {
        "zipformer_sherpa"
    }

    fn model_identity<'a>(
        &self,
        config: &'a AsrProviderConfig,
    ) -> Result<&'a str, ProviderLoadError> {
        Ok(&config
            .zipformer_sherpa
            .as_ref()
            .ok_or_else(|| {
                ProviderLoadError::Configuration("zipformer_sherpa options are required".into())
            })?
            .model)
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
static OPENAI_FACTORY: OpenAiFactory = OpenAiFactory;
static ZEROTTS_ONNX_FACTORY: ZeroTtsOnnxFactory = ZeroTtsOnnxFactory;
static VAD_FACTORIES: [&dyn VadFactory; 1] = [&SILERO_ONNX_FACTORY];
static ASR_FACTORIES: [&dyn AsrFactory; 1] = [&ZIPFORMER_SHERPA_FACTORY];
static LLM_FACTORIES: [&dyn LlmFactory; 1] = [&OPENAI_FACTORY];
static TTS_FACTORIES: [&dyn TtsFactory; 1] = [&ZEROTTS_ONNX_FACTORY];
static COMPILED_PROVIDER_REGISTRY: ProviderRegistry = ProviderRegistry {
    vad: &VAD_FACTORIES,
    asr: &ASR_FACTORIES,
    llm: &LLM_FACTORIES,
    tts: &TTS_FACTORIES,
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
