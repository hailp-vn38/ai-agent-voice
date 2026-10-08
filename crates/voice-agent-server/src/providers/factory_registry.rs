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
    providers::{
        AsrProvider, AssetError, LlmProvider, ProviderLoadError, ProviderType, TtsProvider,
        VadProvider, VisionProvider,
        asr::{GipformerAsrProvider, ZipformerAsrProvider, gipformer, zipformer},
        llm::openai::provider::ConfiguredOpenAiLlm,
        tts::{
            ChillAudioWsProvider, ConfiguredZeroTts, ZeroTtsArtifacts,
            kokoro_vi::{ConfiguredKokoroVi, KokoroViArtifacts, assets as kokoro_assets},
            zerotts::assets as zerotts_assets,
        },
        vad::{LoadedSileroVad, silero::assets as silero_assets},
        vision::OpenAiVisionProvider,
    },
};

pub trait VadFactory: Send + Sync {
    fn adapter(&self) -> &'static str;
    fn model_identity<'a>(
        &self,
        config: &'a VadInstanceConfig,
    ) -> Result<&'a str, ProviderLoadError>;
    /// Resolves this adapter's own model files and builds the runtime. Never downloads: the asset
    /// manager has already ensured them by the time a factory runs.
    fn build(
        &self,
        config: &VadInstanceConfig,
        runtime: &RuntimeConfig,
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

    pub fn speaker_factory(
        &self,
        adapter: &str,
    ) -> Result<&'static dyn SpeakerFactory, ProviderLoadError> {
        if adapter == "campplus_sherpa" {
            Ok(&CAMPPLUS_FACTORY)
        } else {
            #[cfg(feature = "qualification-providers")]
            if adapter == "qualification_speaker" {
                return Ok(&QUALIFICATION_SPEAKER_FACTORY);
            }
            Err(ProviderLoadError::UnsupportedAdapter {
                kind: "Speaker",
                adapter: adapter.into(),
            })
        }
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
            VadInstanceConfig::SileroOnnx(_) => {
                Ok(local_model_identity(self.adapter()).expect("compiled local adapter"))
            }
            #[cfg(feature = "qualification-providers")]
            VadInstanceConfig::QualificationVad(_) => Err(ProviderLoadError::Configuration(
                "silero factory received qualification config".into(),
            )),
        }
    }

    fn build(
        &self,
        config: &VadInstanceConfig,
        runtime: &RuntimeConfig,
    ) -> Result<Arc<dyn VadProvider>, ProviderLoadError> {
        validate_threads(self.adapter(), runtime)?;
        #[allow(irrefutable_let_patterns)] // Qualification builds add another VAD variant.
        let VadInstanceConfig::SileroOnnx(_) = config else {
            return Err(ProviderLoadError::Configuration(
                "silero factory received qualification config".into(),
            ));
        };
        let assets = resolve(silero_assets::resolve_assets())?;
        Ok(Arc::new(
            LoadedSileroVad::load(
                path(&assets.model),
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
#[cfg(feature = "qualification-providers")]
struct QualificationVadFactory;
#[cfg(feature = "qualification-providers")]
struct QualificationAsrFactory;
#[cfg(feature = "qualification-providers")]
struct QualificationLlmFactory;
#[cfg(feature = "qualification-providers")]
struct QualificationTtsFactory;

#[cfg(feature = "qualification-providers")]
impl VadFactory for QualificationVadFactory {
    fn adapter(&self) -> &'static str {
        "qualification_vad"
    }
    fn model_identity<'a>(&self, _: &'a VadInstanceConfig) -> Result<&'a str, ProviderLoadError> {
        Err(ProviderLoadError::Configuration(
            "qualification VAD has no model".into(),
        ))
    }
    fn build(
        &self,
        config: &VadInstanceConfig,
        _: &RuntimeConfig,
    ) -> Result<Arc<dyn VadProvider>, ProviderLoadError> {
        match config {
            VadInstanceConfig::QualificationVad(_) => {
                Ok(Arc::new(super::vad::qualification::QualificationVad))
            }
            _ => Err(ProviderLoadError::Configuration(
                "qualification VAD received another config".into(),
            )),
        }
    }
}
#[cfg(feature = "qualification-providers")]
impl AsrFactory for QualificationAsrFactory {
    fn adapter(&self) -> &'static str {
        "qualification_asr"
    }
    fn model_identity<'a>(&self, _: &'a AsrInstanceConfig) -> Result<&'a str, ProviderLoadError> {
        Err(ProviderLoadError::Configuration(
            "qualification ASR has no model".into(),
        ))
    }
    fn build(
        &self,
        config: &AsrInstanceConfig,
        _: &RuntimeConfig,
        _: usize,
    ) -> Result<Arc<dyn AsrProvider>, ProviderLoadError> {
        match config {
            AsrInstanceConfig::QualificationAsr(_) => {
                Ok(Arc::new(super::asr::qualification::QualificationAsr))
            }
            _ => Err(ProviderLoadError::Configuration(
                "qualification ASR received another config".into(),
            )),
        }
    }
}
#[cfg(feature = "qualification-providers")]
impl LlmFactory for QualificationLlmFactory {
    fn adapter(&self) -> &'static str {
        "qualification_llm"
    }
    fn build(&self, config: &LlmInstanceConfig) -> Result<Arc<dyn LlmProvider>, ProviderLoadError> {
        match config {
            LlmInstanceConfig::QualificationLlm(_) => {
                Ok(Arc::new(super::llm::qualification::QualificationLlm))
            }
            _ => Err(ProviderLoadError::Configuration(
                "qualification LLM received another config".into(),
            )),
        }
    }
}
#[cfg(feature = "qualification-providers")]
impl TtsFactory for QualificationTtsFactory {
    fn adapter(&self) -> &'static str {
        "qualification_tts"
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
        _: &RuntimeConfig,
    ) -> Result<Arc<dyn TtsProvider>, ProviderLoadError> {
        match config {
            TtsInstanceConfig::QualificationTts(_) => {
                Ok(Arc::new(super::tts::qualification::QualificationTts))
            }
            _ => Err(ProviderLoadError::Configuration(
                "qualification TTS received another config".into(),
            )),
        }
    }
}

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
        #[allow(irrefutable_let_patterns)] // Qualification builds add another LLM variant.
        let LlmInstanceConfig::Openai(options) = config else {
            return Err(ProviderLoadError::Configuration(
                "OpenAI factory received qualification config".into(),
            ));
        };
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
            TtsInstanceConfig::ZeroTtsOnnx(_) => Ok(local_model_identity(self.adapter())),
            _ => Err(ProviderLoadError::Configuration(
                "zerotts_onnx factory received another adapter config".into(),
            )),
        }
    }

    fn build(
        &self,
        config: &TtsInstanceConfig,
        runtime: &RuntimeConfig,
    ) -> Result<Arc<dyn TtsProvider>, ProviderLoadError> {
        let TtsInstanceConfig::ZeroTtsOnnx(config) = config else {
            return Err(ProviderLoadError::Configuration(
                "zerotts_onnx factory received another adapter config".into(),
            ));
        };
        let mut config = config.clone();
        config.model = local_model_identity(self.adapter())
            .expect("compiled local adapter")
            .into();
        config.num_threads = runtime.onnx.threads_for(self.adapter());
        validate_threads(self.adapter(), runtime)?;
        if !super::tts::zerotts::descriptor::DESCRIPTOR
            .capabilities
            .voices
            .is_some_and(|voices| voices.iter().any(|voice| voice.id == config.voice))
            || config.language != "vi-VN"
            || config.num_threads <= 0
        {
            return Err(ProviderLoadError::Configuration(
                "ZeroTTS requires a supported voice, vi-VN, and positive threads".into(),
            ));
        }
        let assets = resolve(zerotts_assets::resolve_assets())?;
        Ok(Arc::new(
            ConfiguredZeroTts::load(
                ZeroTtsArtifacts {
                    config: &assets.config,
                    tokenizer: &assets.tokenizer,
                    voices_index: &assets.voices_index,
                    voices: assets.voices,
                    text_encoder: &assets.text_encoder,
                    prefix_step: &assets.prefix_step,
                    local_frame_decode: &assets.local_frame_decode,
                    codec_decode_full: &assets.codec_decode_full,
                    codec_decode_step: &assets.codec_decode_step,
                    codec_shared_data: &assets.codec_shared_data,
                    codec_metadata: &assets.codec_metadata,
                    silence_frame: &assets.silence_frame,
                },
                &runtime.onnx.library,
                config.num_threads,
                config.delivery_mode,
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
            TtsInstanceConfig::KokoroViOnnx(_) => Ok(local_model_identity(self.adapter())),
            _ => Err(ProviderLoadError::Configuration(
                "kokoro_vi_onnx factory received another adapter config".into(),
            )),
        }
    }
    fn build(
        &self,
        config: &TtsInstanceConfig,
        runtime: &RuntimeConfig,
    ) -> Result<Arc<dyn TtsProvider>, ProviderLoadError> {
        let TtsInstanceConfig::KokoroViOnnx(options) = config else {
            return Err(ProviderLoadError::Configuration(
                "kokoro_vi_onnx factory received another adapter config".into(),
            ));
        };
        let mut options = options.clone();
        options.model = local_model_identity(self.adapter())
            .expect("compiled local adapter")
            .into();
        options.num_threads = runtime.onnx.threads_for(self.adapter());
        validate_threads(self.adapter(), runtime)?;
        if !options.valid_selection() {
            return Err(ProviderLoadError::Configuration("Kokoro Vietnamese requires vi-VN, a supported voice, positive threads, and speed_percent 50..=200".into()));
        }
        let assets = resolve(kokoro_assets::resolve_assets())?;
        let voicepack = assets.voicepack(&options.voice).ok_or_else(|| {
            ProviderLoadError::MissingArtifact(format!("voicepack_{}", options.voice))
        })?;
        Ok(Arc::new(
            ConfiguredKokoroVi::load(
                &options,
                runtime,
                KokoroViArtifacts {
                    model: &assets.model,
                    config: &assets.config,
                    voicepack,
                },
            )
            .map_err(|error| ProviderLoadError::Provider(error.to_string()))?,
        ))
    }
}

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
    ) -> Result<Arc<dyn TtsProvider>, ProviderLoadError> {
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

impl AsrFactory for ZipformerSherpaFactory {
    fn adapter(&self) -> &'static str {
        "zipformer_sherpa"
    }

    fn model_identity<'a>(
        &self,
        config: &'a AsrInstanceConfig,
    ) -> Result<&'a str, ProviderLoadError> {
        match config {
            AsrInstanceConfig::ZipformerSherpa(_) => {
                Ok(local_model_identity(self.adapter()).expect("compiled local adapter"))
            }
            _ => Err(ProviderLoadError::Configuration(
                "zipformer_sherpa factory received another adapter config".into(),
            )),
        }
    }

    fn build(
        &self,
        config: &AsrInstanceConfig,
        runtime: &RuntimeConfig,
        _: usize,
    ) -> Result<Arc<dyn AsrProvider>, ProviderLoadError> {
        validate_threads(self.adapter(), runtime)?;
        let AsrInstanceConfig::ZipformerSherpa(options) = config else {
            return Err(ProviderLoadError::Configuration(
                "zipformer_sherpa factory received another adapter config".into(),
            ));
        };
        let assets = resolve(zipformer::assets::resolve_assets())?;
        let mut recognizer_config = OnlineRecognizerConfig::default();
        recognizer_config.model_config.transducer.encoder = Some(path(&assets.encoder));
        recognizer_config.model_config.transducer.decoder = Some(path(&assets.decoder));
        recognizer_config.model_config.transducer.joiner = Some(path(&assets.joiner));
        recognizer_config.model_config.tokens = Some(path(&assets.tokens));
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
            AsrInstanceConfig::GipformerSherpaOffline(_) => {
                Ok(local_model_identity(self.adapter()).expect("compiled local adapter"))
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
        max_buffered_samples: usize,
    ) -> Result<Arc<dyn AsrProvider>, ProviderLoadError> {
        let AsrInstanceConfig::GipformerSherpaOffline(options) = config else {
            return Err(ProviderLoadError::Configuration(
                "gipformer_sherpa_offline factory received another adapter config".into(),
            ));
        };
        validate_threads(self.adapter(), runtime)?;
        let mut options = options.clone();
        options.num_threads = runtime.onnx.threads_for(self.adapter());
        let recognizer =
            build_gipformer_recognizer(&options, &resolve(gipformer::assets::resolve_assets())?)?;
        drop(recognizer.create_stream());
        Ok(Arc::new(GipformerAsrProvider {
            recognizer: Arc::new(recognizer),
            max_buffered_samples,
        }))
    }
}

fn build_gipformer_recognizer(
    options: &GipformerSherpaOfflineConfig,
    assets: &gipformer::assets::GipformerAssets,
) -> Result<OfflineRecognizer, ProviderLoadError> {
    if options.language != "vi-VN" || !(1..=10_000).contains(&options.max_active_paths) {
        return Err(ProviderLoadError::Configuration(
            "Gipformer requires vi-VN and max_active_paths in 1..=10000".into(),
        ));
    }
    let mut config = OfflineRecognizerConfig::default();
    config.model_config.transducer = OfflineTransducerModelConfig {
        encoder: Some(path(&assets.encoder)),
        decoder: Some(path(&assets.decoder)),
        joiner: Some(path(&assets.joiner)),
    };
    config.model_config.tokens = Some(path(&assets.tokens));
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
#[cfg(feature = "qualification-providers")]
static QUALIFICATION_VAD_FACTORY: QualificationVadFactory = QualificationVadFactory;
#[cfg(feature = "qualification-providers")]
static QUALIFICATION_ASR_FACTORY: QualificationAsrFactory = QualificationAsrFactory;
#[cfg(feature = "qualification-providers")]
static QUALIFICATION_LLM_FACTORY: QualificationLlmFactory = QualificationLlmFactory;
#[cfg(feature = "qualification-providers")]
static QUALIFICATION_TTS_FACTORY: QualificationTtsFactory = QualificationTtsFactory;
static VAD_FACTORIES: &[&dyn VadFactory] = &[
    &SILERO_ONNX_FACTORY,
    #[cfg(feature = "qualification-providers")]
    &QUALIFICATION_VAD_FACTORY,
];
static ASR_FACTORIES: &[&dyn AsrFactory] = &[
    &ZIPFORMER_SHERPA_FACTORY,
    &GIPFORMER_SHERPA_OFFLINE_FACTORY,
    #[cfg(feature = "qualification-providers")]
    &QUALIFICATION_ASR_FACTORY,
];
static LLM_FACTORIES: &[&dyn LlmFactory] = &[
    &OPENAI_FACTORY,
    #[cfg(feature = "qualification-providers")]
    &QUALIFICATION_LLM_FACTORY,
];
static TTS_FACTORIES: &[&dyn TtsFactory] = &[
    &ZEROTTS_ONNX_FACTORY,
    &CHILLAUDIO_WS_FACTORY,
    &KOKORO_VI_ONNX_FACTORY,
    #[cfg(feature = "qualification-providers")]
    &QUALIFICATION_TTS_FACTORY,
];
static VISION_FACTORIES: &[&dyn VisionFactory] = &[&OPENAI_VISION_FACTORY];
static COMPILED_PROVIDER_REGISTRY: ProviderRegistry = ProviderRegistry {
    vad: VAD_FACTORIES,
    asr: ASR_FACTORIES,
    llm: LLM_FACTORIES,
    tts: TTS_FACTORIES,
    vision: VISION_FACTORIES,
};

pub fn compiled_provider_registry() -> &'static ProviderRegistry {
    &COMPILED_PROVIDER_REGISTRY
}

/// Local execution width is deployment-owned and bounded. A value outside that range would silently
/// change a runtime's cost profile, so it is refused rather than clamped.
fn validate_threads(adapter: &str, runtime: &RuntimeConfig) -> Result<(), ProviderLoadError> {
    if (1..=128).contains(&runtime.onnx.threads_for(adapter)) {
        Ok(())
    } else {
        Err(ProviderLoadError::Configuration(
            "local provider requires server threads in 1..=128".into(),
        ))
    }
}

/// sherpa-onnx takes model files as strings.
fn path(path: &std::path::Path) -> String {
    path.display().to_string()
}

/// A resolved asset set is a precondition of building, so failure here means the asset manager was
/// skipped or the files vanished underneath us.
fn resolve<T>(assets: Result<T, AssetError>) -> Result<T, ProviderLoadError> {
    assets.map_err(ProviderLoadError::from)
}

/// Logical requirements come from the compiled adapter contract, never DB internal fields.
pub fn local_model_identity(adapter: &str) -> Option<&'static str> {
    match adapter {
        "campplus_sherpa" => Some("campplus_zh_en_advanced"),
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

pub trait SpeakerFactory: Send + Sync {
    fn adapter(&self) -> &'static str;
    fn build(
        &self,
        runtime: &RuntimeConfig,
    ) -> Result<Box<dyn super::speaker::SpeakerProvider>, ProviderLoadError>;
}
struct CampPlusFactory;
static CAMPPLUS_FACTORY: CampPlusFactory = CampPlusFactory;
impl SpeakerFactory for CampPlusFactory {
    fn adapter(&self) -> &'static str {
        "campplus_sherpa"
    }
    fn build(
        &self,
        runtime: &RuntimeConfig,
    ) -> Result<Box<dyn super::speaker::SpeakerProvider>, ProviderLoadError> {
        validate_threads(self.adapter(), runtime)?;
        super::speaker::build(runtime.onnx.threads_for(self.adapter()))
            .map_err(|_| ProviderLoadError::Initialize("CAM++ Speaker"))
    }
}

/// Compile-time Qualification Speaker factory (ADR 0068). No ONNX threads, no
/// model files; the deterministic provider is the whole product.
#[cfg(feature = "qualification-providers")]
struct QualificationSpeakerFactory;
#[cfg(feature = "qualification-providers")]
static QUALIFICATION_SPEAKER_FACTORY: QualificationSpeakerFactory = QualificationSpeakerFactory;
#[cfg(feature = "qualification-providers")]
impl SpeakerFactory for QualificationSpeakerFactory {
    fn adapter(&self) -> &'static str {
        "qualification_speaker"
    }
    fn build(
        &self,
        _runtime: &RuntimeConfig,
    ) -> Result<Box<dyn super::speaker::SpeakerProvider>, ProviderLoadError> {
        super::speaker::qualification::build(1)
            .map_err(|_| ProviderLoadError::Initialize("Qualification Speaker"))
    }
}
