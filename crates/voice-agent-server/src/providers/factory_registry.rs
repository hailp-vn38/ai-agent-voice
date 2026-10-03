//! The fixed startup registry is the only provider selection point.

use std::sync::Arc;

use sherpa_onnx::{
    OfflineRecognizer, OfflineRecognizerConfig, OfflineTransducerModelConfig, OnlineRecognizer,
    OnlineRecognizerConfig,
};

use crate::{
    config::{
        AsrInstanceConfig, GipformerSherpaOfflineConfig, LlmInstanceConfig, RuntimeConfig,
        TtsInstanceConfig, VadInstanceConfig, VisionInstanceConfig,
    },
    models::ResolvedModel,
    providers::{
        AsrProvider, LlmProvider, ProviderLoadError, ProviderType, TtsProvider, VadProvider,
        VisionProvider,
        asr::{GipformerAsrProvider, ZipformerAsrProvider},
        llm::openai::provider::ConfiguredOpenAiLlm,
        tts::{
            ChillAudioWsProvider, ConfiguredZeroTts, ZeroTtsArtifacts,
            kokoro_vi::{ConfiguredKokoroVi, KokoroViArtifacts},
        },
        vad::LoadedSileroVad,
        vision::OpenAiVisionProvider,
    },
};

pub trait VadFactory: Send + Sync {
    fn adapter(&self) -> &'static str;
    fn model_identity<'a>(
        &self,
        config: &'a VadInstanceConfig,
    ) -> Result<&'a str, ProviderLoadError>;
    fn build(
        &self,
        config: &VadInstanceConfig,
        runtime: &RuntimeConfig,
        model: &ResolvedModel,
    ) -> Result<Arc<dyn VadProvider>, ProviderLoadError>;
}

pub trait AsrFactory: Send + Sync {
    fn adapter(&self) -> &'static str;
    fn model_identity<'a>(
        &self,
        config: &'a AsrInstanceConfig,
    ) -> Result<&'a str, ProviderLoadError>;
    fn build(
        &self,
        config: &AsrInstanceConfig,
        runtime: &RuntimeConfig,
        model: &ResolvedModel,
        max_buffered_samples: usize,
    ) -> Result<Arc<dyn AsrProvider>, ProviderLoadError>;
}

pub trait LlmFactory: Send + Sync {
    fn adapter(&self) -> &'static str;
    fn build(&self, config: &LlmInstanceConfig) -> Result<Arc<dyn LlmProvider>, ProviderLoadError>;
}

pub trait TtsFactory: Send + Sync {
    fn adapter(&self) -> &'static str;
    fn model_identity<'a>(
        &self,
        config: &'a TtsInstanceConfig,
    ) -> Result<Option<&'a str>, ProviderLoadError>;
    fn build(
        &self,
        config: &TtsInstanceConfig,
        runtime: &RuntimeConfig,
        model: Option<&ResolvedModel>,
    ) -> Result<Arc<dyn TtsProvider>, ProviderLoadError>;
}
pub trait VisionFactory: Send + Sync {
    fn adapter(&self) -> &'static str;
    fn build(
        &self,
        config: &VisionInstanceConfig,
    ) -> Result<Arc<dyn VisionProvider>, ProviderLoadError>;
}

/// Factories are linked into the binary. There is no runtime code discovery or plugin loading.
pub struct ProviderRegistry {
    vad: &'static [&'static dyn VadFactory],
    asr: &'static [&'static dyn AsrFactory],
    llm: &'static [&'static dyn LlmFactory],
    tts: &'static [&'static dyn TtsFactory],
    vision: &'static [&'static dyn VisionFactory],
}

impl ProviderRegistry {
    pub fn admin_adapters(&self) -> impl Iterator<Item = (ProviderType, &'static str)> + '_ {
        self.vad
            .iter()
            .map(|factory| (ProviderType::Vad, factory.adapter()))
            .chain(
                self.asr
                    .iter()
                    .map(|factory| (ProviderType::Asr, factory.adapter())),
            )
            .chain(
                self.llm
                    .iter()
                    .map(|factory| (ProviderType::Llm, factory.adapter())),
            )
            .chain(
                self.tts
                    .iter()
                    .map(|factory| (ProviderType::Tts, factory.adapter())),
            )
    }

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
    pub fn vision_factory(
        &self,
        adapter: &str,
    ) -> Result<&'static dyn VisionFactory, ProviderLoadError> {
        self.vision
            .iter()
            .copied()
            .find(|factory| factory.adapter() == adapter)
            .ok_or_else(|| ProviderLoadError::UnsupportedAdapter {
                kind: "VISION",
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
        config: &'a VadInstanceConfig,
    ) -> Result<&'a str, ProviderLoadError> {
        match config {
            VadInstanceConfig::SileroOnnx(_) => Ok("silero_vad_v5"),
        }
    }

    fn build(
        &self,
        config: &VadInstanceConfig,
        runtime: &RuntimeConfig,
        model: &ResolvedModel,
    ) -> Result<Arc<dyn VadProvider>, ProviderLoadError> {
        validate_model_adapter(model, self.adapter(), runtime)?;
        let VadInstanceConfig::SileroOnnx(_) = config;
        Ok(Arc::new(
            LoadedSileroVad::load(
                required(model, "vad")?,
                runtime.onnx.library.clone(),
                runtime.onnx.threads_for(self.adapter()),
            )
            .map_err(|error| ProviderLoadError::Provider(error.to_string()))?,
        ))
    }
}

struct ZipformerSherpaFactory;
struct GipformerSherpaOfflineFactory;

struct OpenAiFactory;
struct OpenAiVisionFactory;

impl VisionFactory for OpenAiVisionFactory {
    fn adapter(&self) -> &'static str {
        "openai_vision"
    }
    fn build(
        &self,
        config: &VisionInstanceConfig,
    ) -> Result<Arc<dyn VisionProvider>, ProviderLoadError> {
        let VisionInstanceConfig::OpenAiVision(options) = config;
        OpenAiVisionProvider::new(options.clone())
            .map(|provider| Arc::new(provider) as Arc<dyn VisionProvider>)
            .map_err(|_| {
                ProviderLoadError::Provider(
                    "OpenAI-compatible vision provider initialization failed".into(),
                )
            })
    }
}

impl LlmFactory for OpenAiFactory {
    fn adapter(&self) -> &'static str {
        "openai"
    }

    fn build(&self, config: &LlmInstanceConfig) -> Result<Arc<dyn LlmProvider>, ProviderLoadError> {
        let LlmInstanceConfig::Openai(options) = config;
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
        config: &'a TtsInstanceConfig,
    ) -> Result<Option<&'a str>, ProviderLoadError> {
        match config {
            TtsInstanceConfig::ZeroTtsOnnx(_) => Ok(Some("zerotts_default")),
            _ => Err(ProviderLoadError::Configuration(
                "zerotts_onnx factory received another adapter config".into(),
            )),
        }
    }

    fn build(
        &self,
        config: &TtsInstanceConfig,
        runtime: &RuntimeConfig,
        model: Option<&ResolvedModel>,
    ) -> Result<Arc<dyn TtsProvider>, ProviderLoadError> {
        let TtsInstanceConfig::ZeroTtsOnnx(config) = config else {
            return Err(ProviderLoadError::Configuration(
                "zerotts_onnx factory received another adapter config".into(),
            ));
        };
        let mut config = config.clone();
        config.model = "zerotts_default".into();
        config.num_threads = runtime.onnx.threads_for(self.adapter());
        let model = model.ok_or_else(|| {
            ProviderLoadError::Configuration("zerotts_onnx requires a local model".into())
        })?;
        validate_model_adapter(model, self.adapter(), runtime)?;
        if config.model != "zerotts_default"
            || !super::tts::zerotts::descriptor::DESCRIPTOR
                .capabilities
                .voices
                .is_some_and(|voices| voices.iter().any(|voice| voice.id == config.voice))
            || config.language != "vi-VN"
            || config.num_threads <= 0
        {
            return Err(ProviderLoadError::Configuration(
                "ZeroTTS requires its fixed model, a supported voice, vi-VN, and positive threads"
                    .into(),
            ));
        }
        if model.identity() != config.model {
            return Err(ProviderLoadError::Configuration(
                "ZeroTTS resolved model does not match configured logical model".into(),
            ));
        }
        let voice_role = format!("voice_{}", config.voice);
        required(model, &voice_role)?;
        for role in ZEROTTS_REQUIRED_ARTIFACT_ROLES {
            required(model, role)?;
        }
        Ok(Arc::new(
            ConfiguredZeroTts::load(
                ZeroTtsArtifacts {
                    config: model.artifact("config").expect("required above"),
                    tokenizer: model.artifact("tokenizer").expect("required above"),
                    voice: model.artifact(&voice_role).expect("required above"),
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
                &config.voice,
            )
            .map_err(|error| ProviderLoadError::Provider(error.to_string()))?,
        ))
    }
}

struct ChillAudioWsFactory;
struct KokoroViOnnxFactory;

impl TtsFactory for KokoroViOnnxFactory {
    fn adapter(&self) -> &'static str {
        "kokoro_vi_onnx"
    }
    fn model_identity<'a>(
        &self,
        config: &'a TtsInstanceConfig,
    ) -> Result<Option<&'a str>, ProviderLoadError> {
        match config {
            TtsInstanceConfig::KokoroViOnnx(_) => Ok(Some("kokoro_vi_contextbox")),
            _ => Err(ProviderLoadError::Configuration(
                "kokoro_vi_onnx factory received another adapter config".into(),
            )),
        }
    }
    fn build(
        &self,
        config: &TtsInstanceConfig,
        runtime: &RuntimeConfig,
        model: Option<&ResolvedModel>,
    ) -> Result<Arc<dyn TtsProvider>, ProviderLoadError> {
        let TtsInstanceConfig::KokoroViOnnx(options) = config else {
            return Err(ProviderLoadError::Configuration(
                "kokoro_vi_onnx factory received another adapter config".into(),
            ));
        };
        let mut options = options.clone();
        options.model = "kokoro_vi_contextbox".into();
        options.num_threads = runtime.onnx.threads_for(self.adapter());
        let model = model.ok_or_else(|| {
            ProviderLoadError::Configuration("kokoro_vi_onnx requires a local model".into())
        })?;
        validate_model_adapter(model, self.adapter(), runtime)?;
        if !options.valid_selection() {
            return Err(ProviderLoadError::Configuration("Kokoro Vietnamese requires its pinned model, vi-VN, a supported voice, positive threads, and speed_percent 50..=200".into()));
        }
        if model.identity() != options.model {
            return Err(ProviderLoadError::Configuration(
                "Kokoro Vietnamese resolved model does not match configured logical model".into(),
            ));
        }
        for role in KOKORO_VI_REQUIRED_ARTIFACT_ROLES {
            required(model, role)?;
        }
        let voicepack_role = format!("voicepack_{}", options.voice);
        required(model, &voicepack_role)?;
        Ok(Arc::new(
            ConfiguredKokoroVi::load(
                &options,
                runtime,
                KokoroViArtifacts {
                    model: model.artifact("model").expect("required above"),
                    config: model.artifact("config").expect("required above"),
                    voicepack: model.artifact(&voicepack_role).expect("required above"),
                },
            )
            .map_err(|error| ProviderLoadError::Provider(error.to_string()))?,
        ))
    }
}

const KOKORO_VI_REQUIRED_ARTIFACT_ROLES: &[&str] = &["model", "config"];

impl TtsFactory for ChillAudioWsFactory {
    fn adapter(&self) -> &'static str {
        "chillaudio_ws"
    }
    fn model_identity<'a>(
        &self,
        _: &'a TtsInstanceConfig,
    ) -> Result<Option<&'a str>, ProviderLoadError> {
        Ok(None)
    }
    fn build(
        &self,
        config: &TtsInstanceConfig,
        runtime: &RuntimeConfig,
        model: Option<&ResolvedModel>,
    ) -> Result<Arc<dyn TtsProvider>, ProviderLoadError> {
        if model.is_some() {
            return Err(ProviderLoadError::Configuration(
                "chillaudio_ws must not receive a local model".into(),
            ));
        }
        let TtsInstanceConfig::ChillAudioWs(options) = config else {
            return Err(ProviderLoadError::Configuration(
                "chillaudio_ws factory received another adapter config".into(),
            ));
        };
        if !super::tts::chillaudio::descriptor::DESCRIPTOR
            .capabilities
            .voices
            .is_some_and(|voices| voices.iter().any(|voice| voice.id == options.voice))
            || options.language != "vi"
            || runtime.chillaudio.ws_url.scheme() != "wss"
            || runtime.chillaudio.ws_url.host_str().is_none()
            || !(1..=120_000).contains(&runtime.chillaudio.timeout_ms)
        {
            return Err(ProviderLoadError::Configuration(
                "invalid ChillAudio selection or server runtime".into(),
            ));
        }
        let mut options = options.clone();
        options.ws_url = runtime.chillaudio.ws_url.clone();
        options.timeout_ms = runtime.chillaudio.timeout_ms;
        Ok(Arc::new(ChillAudioWsProvider::new(options)))
    }
}

const ZEROTTS_REQUIRED_ARTIFACT_ROLES: &[&str] = &[
    "config",
    "tokenizer",
    "null_voice",
    "voices_index",
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
        config: &'a AsrInstanceConfig,
    ) -> Result<&'a str, ProviderLoadError> {
        match config {
            AsrInstanceConfig::ZipformerSherpa(_) => Ok("zipformer_vi_streaming"),
            _ => Err(ProviderLoadError::Configuration(
                "zipformer_sherpa factory received another adapter config".into(),
            )),
        }
    }

    fn build(
        &self,
        config: &AsrInstanceConfig,
        runtime: &RuntimeConfig,
        model: &ResolvedModel,
        _: usize,
    ) -> Result<Arc<dyn AsrProvider>, ProviderLoadError> {
        validate_model_adapter(model, self.adapter(), runtime)?;
        let AsrInstanceConfig::ZipformerSherpa(options) = config else {
            return Err(ProviderLoadError::Configuration(
                "zipformer_sherpa factory received another adapter config".into(),
            ));
        };
        let mut recognizer_config = OnlineRecognizerConfig::default();
        recognizer_config.model_config.transducer.encoder = Some(required(model, "encoder")?);
        recognizer_config.model_config.transducer.decoder = Some(required(model, "decoder")?);
        recognizer_config.model_config.transducer.joiner = Some(required(model, "joiner")?);
        recognizer_config.model_config.tokens = Some(required(model, "tokens")?);
        recognizer_config.model_config.num_threads = runtime.onnx.threads_for(self.adapter());
        recognizer_config.model_config.provider = Some("cpu".into());
        recognizer_config.decoding_method = Some(options.decoding_method.as_str().into());
        recognizer_config.enable_endpoint = false;
        let recognizer = Arc::new(
            OnlineRecognizer::create(&recognizer_config)
                .ok_or(ProviderLoadError::Initialize("Zipformer ASR"))?,
        );
        drop(recognizer.create_stream());
        Ok(Arc::new(ZipformerAsrProvider { recognizer }))
    }
}

impl AsrFactory for GipformerSherpaOfflineFactory {
    fn adapter(&self) -> &'static str {
        "gipformer_sherpa_offline"
    }

    fn model_identity<'a>(
        &self,
        config: &'a AsrInstanceConfig,
    ) -> Result<&'a str, ProviderLoadError> {
        match config {
            AsrInstanceConfig::GipformerSherpaOffline(options)
                if options.model == "gipformer15_vi_int8" =>
            {
                Ok("gipformer15_vi_int8")
            }
            _ => Err(ProviderLoadError::Configuration(
                "gipformer_sherpa_offline factory received another adapter config".into(),
            )),
        }
    }

    fn build(
        &self,
        config: &AsrInstanceConfig,
        runtime: &RuntimeConfig,
        model: &ResolvedModel,
        max_buffered_samples: usize,
    ) -> Result<Arc<dyn AsrProvider>, ProviderLoadError> {
        let AsrInstanceConfig::GipformerSherpaOffline(options) = config else {
            return Err(ProviderLoadError::Configuration(
                "gipformer_sherpa_offline factory received another adapter config".into(),
            ));
        };
        validate_model_adapter(model, self.adapter(), runtime)?;
        let mut options = options.clone();
        options.num_threads = runtime.onnx.threads_for(self.adapter());
        let recognizer = build_gipformer_recognizer(&options, model)?;
        drop(recognizer.create_stream());
        Ok(Arc::new(GipformerAsrProvider {
            recognizer: Arc::new(recognizer),
            max_buffered_samples,
        }))
    }
}

fn build_gipformer_recognizer(
    options: &GipformerSherpaOfflineConfig,
    model: &ResolvedModel,
) -> Result<OfflineRecognizer, ProviderLoadError> {
    if options.language != "vi-VN" || !(1..=10_000).contains(&options.max_active_paths) {
        return Err(ProviderLoadError::Configuration(
            "Gipformer requires vi-VN and max_active_paths in 1..=10000".into(),
        ));
    }
    let mut config = OfflineRecognizerConfig::default();
    config.model_config.transducer = OfflineTransducerModelConfig {
        encoder: Some(required(model, "encoder")?),
        decoder: Some(required(model, "decoder")?),
        joiner: Some(required(model, "joiner")?),
    };
    config.model_config.tokens = Some(required(model, "tokens")?);
    config.model_config.num_threads = options.num_threads;
    config.model_config.provider = Some("cpu".into());
    config.model_config.model_type = Some("transducer".into());
    config.feat_config.sample_rate = 16_000;
    config.feat_config.feature_dim = 80;
    config.decoding_method = Some(options.decoding_method.as_str().into());
    config.max_active_paths = options.max_active_paths;
    OfflineRecognizer::create(&config).ok_or(ProviderLoadError::Initialize("Gipformer ASR"))
}

static SILERO_ONNX_FACTORY: SileroOnnxFactory = SileroOnnxFactory;
static ZIPFORMER_SHERPA_FACTORY: ZipformerSherpaFactory = ZipformerSherpaFactory;
static GIPFORMER_SHERPA_OFFLINE_FACTORY: GipformerSherpaOfflineFactory =
    GipformerSherpaOfflineFactory;
static OPENAI_FACTORY: OpenAiFactory = OpenAiFactory;
static OPENAI_VISION_FACTORY: OpenAiVisionFactory = OpenAiVisionFactory;
static ZEROTTS_ONNX_FACTORY: ZeroTtsOnnxFactory = ZeroTtsOnnxFactory;
static CHILLAUDIO_WS_FACTORY: ChillAudioWsFactory = ChillAudioWsFactory;
static KOKORO_VI_ONNX_FACTORY: KokoroViOnnxFactory = KokoroViOnnxFactory;
static VAD_FACTORIES: [&dyn VadFactory; 1] = [&SILERO_ONNX_FACTORY];
static ASR_FACTORIES: [&dyn AsrFactory; 2] =
    [&ZIPFORMER_SHERPA_FACTORY, &GIPFORMER_SHERPA_OFFLINE_FACTORY];
static LLM_FACTORIES: [&dyn LlmFactory; 1] = [&OPENAI_FACTORY];
static TTS_FACTORIES: [&dyn TtsFactory; 3] = [
    &ZEROTTS_ONNX_FACTORY,
    &CHILLAUDIO_WS_FACTORY,
    &KOKORO_VI_ONNX_FACTORY,
];
static VISION_FACTORIES: [&dyn VisionFactory; 1] = [&OPENAI_VISION_FACTORY];
static COMPILED_PROVIDER_REGISTRY: ProviderRegistry = ProviderRegistry {
    vad: &VAD_FACTORIES,
    asr: &ASR_FACTORIES,
    llm: &LLM_FACTORIES,
    tts: &TTS_FACTORIES,
    vision: &VISION_FACTORIES,
};

pub fn compiled_provider_registry() -> &'static ProviderRegistry {
    &COMPILED_PROVIDER_REGISTRY
}

fn validate_model_adapter(
    model: &ResolvedModel,
    expected: &'static str,
    runtime: &RuntimeConfig,
) -> Result<(), ProviderLoadError> {
    if model.adapter() == expected {
        if local_model_identity(expected) != Some(model.identity())
            || !(1..=128).contains(&runtime.onnx.threads_for(expected))
        {
            return Err(ProviderLoadError::Configuration(
                "local provider requires its pinned model and server threads in 1..=128".into(),
            ));
        }
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
}

/// Logical requirements come from the compiled adapter contract, never DB internal fields.
pub fn local_model_identity(adapter: &str) -> Option<&'static str> {
    match adapter {
        "silero_onnx" => Some("silero_vad_v5"),
        "zipformer_sherpa" => Some("zipformer_vi_streaming"),
        "gipformer_sherpa_offline" => Some("gipformer15_vi_int8"),
        "zerotts_onnx" => Some("zerotts_default"),
        "kokoro_vi_onnx" => Some("kokoro_vi_contextbox"),
        _ => None,
    }
}
/// Internal effective configuration is distinct from canonical Admin desired configuration.
pub(crate) fn effective_local_config(
    adapter: &str,
    mut value: serde_json::Value,
    runtime: &RuntimeConfig,
) -> Result<serde_json::Value, ProviderLoadError> {
    let object = value.as_object_mut().ok_or_else(|| {
        ProviderLoadError::Configuration("provider config must be an object".into())
    })?;
    if let Some(model) = local_model_identity(adapter) {
        object.insert("model".into(), model.into());
        object.insert(
            "num_threads".into(),
            runtime.onnx.threads_for(adapter).into(),
        );
    }
    if adapter == "chillaudio_ws" {
        object.insert("ws_url".into(), runtime.chillaudio.ws_url.as_str().into());
        object.insert("timeout_ms".into(), runtime.chillaudio.timeout_ms.into());
    }
    Ok(value)
}
