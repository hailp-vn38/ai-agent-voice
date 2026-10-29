//! SenseVoiceSmall offline multilingual ASR model files (`zh`, `en`, `ja`, `ko`, `yue`).

use std::path::{Path, PathBuf};

use crate::providers::assets::{
    Asset, AssetAcquirer, AssetError, ProviderAssetManager, ensure_asset, http_acquirer,
    model_path, required,
};

/// Pinned upstream revision from `csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17`.
pub const MODEL_REVISION: &str = "2365baeacb507f821a0c8120fcee3d484dba7a07";

pub const CORE_ASSETS: &[Asset] = &[
    asset!("model.onnx", "model.int8.onnx"),
    asset!("tokens.txt", "tokens.txt"),
];

/// Runtime paths for one offline SenseVoiceSmall recogniser.
pub struct SenseVoiceAssets {
    pub model: PathBuf,
    pub tokens: PathBuf,
}

pub fn model_dir() -> PathBuf {
    model_path("ASR", "sensevoice")
}

pub struct SenseVoiceAssetManager;

impl ProviderAssetManager for SenseVoiceAssetManager {
    fn ensure_assets(&self) -> Result<(), AssetError> {
        ensure_assets_into(http_acquirer(), &model_dir())
    }

    fn revision(&self) -> &'static str {
        MODEL_REVISION
    }
}

static ASSET_MANAGER: SenseVoiceAssetManager = SenseVoiceAssetManager;

pub static ASSETS: &SenseVoiceAssetManager = &ASSET_MANAGER;

/// Takes the root explicitly so tests install into a temporary directory.
pub fn ensure_assets_into(acquirer: &dyn AssetAcquirer, root: &Path) -> Result<(), AssetError> {
    for asset in CORE_ASSETS {
        ensure_asset(acquirer, root, asset)?;
    }
    Ok(())
}

pub fn resolve_assets_from(root: &Path) -> Result<SenseVoiceAssets, AssetError> {
    Ok(SenseVoiceAssets {
        model: required(root, "model.onnx")?,
        tokens: required(root, "tokens.txt")?,
    })
}

pub fn resolve_assets() -> Result<SenseVoiceAssets, AssetError> {
    resolve_assets_from(&model_dir())
}

macro_rules! asset {
    ($path:literal, $remote:literal) => {
        Asset {
            path: $path,
            url: concat!(
                "https://huggingface.co/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/resolve/",
                "2365baeacb507f821a0c8120fcee3d484dba7a07/",
                $remote
            ),
        }
    };
}

use asset;
