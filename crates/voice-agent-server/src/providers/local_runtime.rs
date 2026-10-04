//! Adapter-owned planning for shareable local native resources.

use crate::{
    config::{
        GipformerSherpaOfflineConfig, KokoroViOnnxConfig, SileroOnnxConfig, ZeroTtsOnnxConfig,
        ZipformerSherpaConfig,
    },
    providers::ProviderLoadError,
    services::provider_runtime::LocalRuntimePlan,
};

/// Plans only native state. Provider factories retain construction responsibility.
pub trait LocalRuntimeAdapter: Send + Sync {
    fn adapter_id(&self) -> &'static str;
    fn physical_plan(
        &self,
        effective_config: serde_json::Value,
    ) -> Result<LocalRuntimePlan, ProviderLoadError>;
}

pub struct LocalRuntimeAdapterRegistry {
    adapters: &'static [&'static dyn LocalRuntimeAdapter],
}

impl LocalRuntimeAdapterRegistry {
    pub fn get(&self, adapter: &str) -> Option<&'static dyn LocalRuntimeAdapter> {
        self.adapters
            .iter()
            .copied()
            .find(|candidate| candidate.adapter_id() == adapter)
    }
}

struct SileroOnnxPlanner;
struct ZipformerSherpaPlanner;
struct GipformerSherpaOfflinePlanner;
struct ZeroTtsOnnxPlanner;
struct KokoroViOnnxPlanner;

impl LocalRuntimeAdapter for SileroOnnxPlanner {
    fn adapter_id(&self) -> &'static str {
        "silero_onnx"
    }

    fn physical_plan(
        &self,
        effective_config: serde_json::Value,
    ) -> Result<LocalRuntimePlan, ProviderLoadError> {
        let config: SileroOnnxConfig = serde_json::from_value(effective_config).map_err(|_| {
            ProviderLoadError::Configuration("invalid Silero local runtime config".into())
        })?;
        // Thresholds and capture timing are logical LoadedVad state, not ONNX-session state.
        Ok(LocalRuntimePlan::onnx(
            self.adapter_id(),
            config.model,
            serde_json::json!({}),
        ))
    }
}

#[derive(serde::Serialize)]
struct ZipformerPhysicalSpec {
    decoding_method: crate::config::TransducerDecodingMethod,
}

impl LocalRuntimeAdapter for ZipformerSherpaPlanner {
    fn adapter_id(&self) -> &'static str {
        "zipformer_sherpa"
    }

    fn physical_plan(
        &self,
        effective_config: serde_json::Value,
    ) -> Result<LocalRuntimePlan, ProviderLoadError> {
        let config: ZipformerSherpaConfig =
            serde_json::from_value(effective_config).map_err(|_| {
                ProviderLoadError::Configuration("invalid Zipformer local runtime config".into())
            })?;
        let spec = serde_json::to_value(ZipformerPhysicalSpec {
            decoding_method: config.decoding_method,
        })
        .map_err(|_| ProviderLoadError::Configuration("invalid Zipformer physical spec".into()))?;
        Ok(LocalRuntimePlan::onnx(
            self.adapter_id(),
            config.model,
            spec,
        ))
    }
}

#[derive(serde::Serialize)]
struct GipformerPhysicalSpec {
    decoding_method: crate::config::TransducerDecodingMethod,
    max_active_paths: i32,
}

impl LocalRuntimeAdapter for GipformerSherpaOfflinePlanner {
    fn adapter_id(&self) -> &'static str {
        "gipformer_sherpa_offline"
    }

    fn physical_plan(
        &self,
        effective_config: serde_json::Value,
    ) -> Result<LocalRuntimePlan, ProviderLoadError> {
        let config: GipformerSherpaOfflineConfig = serde_json::from_value(effective_config)
            .map_err(|_| {
                ProviderLoadError::Configuration("invalid Gipformer local runtime config".into())
            })?;
        if config.language != "vi-VN" || !(1..=10_000).contains(&config.max_active_paths) {
            return Err(ProviderLoadError::Configuration(
                "invalid Gipformer local runtime config".into(),
            ));
        }
        let spec = serde_json::to_value(GipformerPhysicalSpec {
            decoding_method: config.decoding_method,
            max_active_paths: config.max_active_paths,
        })
        .map_err(|_| ProviderLoadError::Configuration("invalid Gipformer physical spec".into()))?;
        Ok(LocalRuntimePlan::onnx(
            self.adapter_id(),
            config.model,
            spec,
        ))
    }
}

#[derive(serde::Serialize)]
struct ZeroTtsPhysicalSpec {
    delivery_mode: crate::config::ZeroTtsDeliveryMode,
}

impl LocalRuntimeAdapter for ZeroTtsOnnxPlanner {
    fn adapter_id(&self) -> &'static str {
        "zerotts_onnx"
    }

    fn physical_plan(
        &self,
        effective_config: serde_json::Value,
    ) -> Result<LocalRuntimePlan, ProviderLoadError> {
        let config: ZeroTtsOnnxConfig = serde_json::from_value(effective_config).map_err(|_| {
            ProviderLoadError::Configuration("invalid ZeroTTS local runtime config".into())
        })?;
        let supported = super::tts::zerotts::descriptor::DESCRIPTOR
            .capabilities
            .voices
            .is_some_and(|voices| voices.iter().any(|voice| voice.id == config.voice));
        if !supported || config.language != "vi-VN" || config.num_threads <= 0 {
            return Err(ProviderLoadError::Configuration(
                "invalid ZeroTTS local runtime config".into(),
            ));
        }
        let spec = serde_json::to_value(ZeroTtsPhysicalSpec {
            delivery_mode: config.delivery_mode,
        })
        .map_err(|_| ProviderLoadError::Configuration("invalid ZeroTTS physical spec".into()))?;
        Ok(LocalRuntimePlan::onnx(
            self.adapter_id(),
            config.model,
            spec,
        ))
    }
}

impl LocalRuntimeAdapter for KokoroViOnnxPlanner {
    fn adapter_id(&self) -> &'static str {
        "kokoro_vi_onnx"
    }

    fn physical_plan(
        &self,
        effective_config: serde_json::Value,
    ) -> Result<LocalRuntimePlan, ProviderLoadError> {
        let mut config: KokoroViOnnxConfig =
            serde_json::from_value(effective_config).map_err(|_| {
                ProviderLoadError::Configuration("invalid Kokoro local runtime config".into())
            })?;
        if !config.valid_selection() {
            return Err(ProviderLoadError::Configuration(
                "invalid Kokoro local runtime config".into(),
            ));
        }
        // Voicepack/speed remain physical until Kokoro gains a request-level voice selection.
        config.preload = false;
        let model = config.model.clone();
        let spec = serde_json::to_value(config)
            .map_err(|_| ProviderLoadError::Configuration("invalid Kokoro physical spec".into()))?;
        Ok(LocalRuntimePlan::onnx_with_kokoro_g2p(
            self.adapter_id(),
            model,
            spec,
        ))
    }
}

static SILERO_ONNX_PLANNER: SileroOnnxPlanner = SileroOnnxPlanner;
static ZIPFORMER_SHERPA_PLANNER: ZipformerSherpaPlanner = ZipformerSherpaPlanner;
static GIPFORMER_SHERPA_OFFLINE_PLANNER: GipformerSherpaOfflinePlanner =
    GipformerSherpaOfflinePlanner;
static ZEROTTS_ONNX_PLANNER: ZeroTtsOnnxPlanner = ZeroTtsOnnxPlanner;
static KOKORO_VI_ONNX_PLANNER: KokoroViOnnxPlanner = KokoroViOnnxPlanner;
static LOCAL_RUNTIME_ADAPTERS: [&dyn LocalRuntimeAdapter; 5] = [
    &SILERO_ONNX_PLANNER,
    &ZIPFORMER_SHERPA_PLANNER,
    &GIPFORMER_SHERPA_OFFLINE_PLANNER,
    &ZEROTTS_ONNX_PLANNER,
    &KOKORO_VI_ONNX_PLANNER,
];
static LOCAL_RUNTIME_ADAPTER_REGISTRY: LocalRuntimeAdapterRegistry = LocalRuntimeAdapterRegistry {
    adapters: &LOCAL_RUNTIME_ADAPTERS,
};

pub fn compiled_local_runtime_adapter_registry() -> &'static LocalRuntimeAdapterRegistry {
    &LOCAL_RUNTIME_ADAPTER_REGISTRY
}
