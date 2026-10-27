//! Provider-owned model assets for the ViFastTts Ngọc Huyền Small ONNX FP32 runtime.

use std::path::{Path, PathBuf};

use crate::providers::assets::{
    Asset, AssetAcquirer, AssetError, ProviderAssetManager, ensure_asset, http_acquirer,
    model_path, required,
};

/// Pinned upstream revision from `dvchd/vifasttts-ngochuyen-small` with `onset_has_q=true`.
pub const MODEL_REVISION: &str = "66aff06948f88b752a414e9ceec97f5291dad723";

macro_rules! model_asset {
    ($name:literal) => {
        Asset {
            path: $name,
            url: concat!(
                "https://huggingface.co/dvchd/vifasttts-ngochuyen-small/resolve/",
                "66aff06948f88b752a414e9ceec97f5291dad723/",
                $name
            ),
        }
    };
}

pub const CORE_ASSETS: &[Asset] = &[
    model_asset!("onnx/acoustic.onnx"),
    model_asset!("onnx/vocoder.onnx"),
    model_asset!("configs/small_ngochuyen.yaml"),
    model_asset!("LICENSE.md"),
];

#[derive(Clone, Copy, Debug)]
pub struct PresetVoice {
    pub id: &'static str,
    pub name: &'static str,
}

pub const DEFAULT_VOICE: &str = "ngoc_huyen";
pub const VOICES: &[PresetVoice] = &[PresetVoice {
    id: "ngoc_huyen",
    name: "Ngọc Huyền",
}];

#[derive(Clone, Debug)]
pub struct VifastttsAssets {
    pub acoustic: PathBuf,
    pub vocoder: PathBuf,
    pub config: PathBuf,
    pub license: PathBuf,
}

pub fn model_dir() -> PathBuf {
    model_path("TTS", "vifasttts")
}

pub struct VifastttsAssetManager;

impl ProviderAssetManager for VifastttsAssetManager {
    fn ensure_assets(&self) -> Result<(), AssetError> {
        ensure_assets_into(http_acquirer(), &model_dir())
    }

    fn revision(&self) -> &'static str {
        MODEL_REVISION
    }
}

static ASSET_MANAGER: VifastttsAssetManager = VifastttsAssetManager;
pub static ASSETS: &VifastttsAssetManager = &ASSET_MANAGER;

pub fn ensure_assets_into(acquirer: &dyn AssetAcquirer, root: &Path) -> Result<(), AssetError> {
    for asset in CORE_ASSETS {
        ensure_asset(acquirer, root, asset)?;
    }
    Ok(())
}

pub fn resolve_assets_from(root: &Path) -> Result<VifastttsAssets, AssetError> {
    Ok(VifastttsAssets {
        acoustic: required(root, "onnx/acoustic.onnx")?,
        vocoder: required(root, "onnx/vocoder.onnx")?,
        config: required(root, "configs/small_ngochuyen.yaml")?,
        license: required(root, "LICENSE.md")?,
    })
}

pub fn resolve_assets() -> Result<VifastttsAssets, AssetError> {
    resolve_assets_from(&model_dir())
}
