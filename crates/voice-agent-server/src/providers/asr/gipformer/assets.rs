//! Gipformer offline Vietnamese ASR model files.

use std::path::{Path, PathBuf};

use crate::providers::assets::{
    Asset, AssetAcquirer, AssetError, ProviderAssetManager, ensure_asset, http_acquirer, is_ready,
    model_path,
};

/// Pinned upstream revision.
pub const MODEL_REVISION: &str = "dd9227dcd8705c13f33bdbe59728d546ab94480f";

pub const CORE_ASSETS: &[Asset] = &[
    asset!("encoder.onnx", "encoder.int8.onnx"),
    asset!("decoder.onnx", "decoder.int8.onnx"),
    asset!("joiner.onnx", "joiner.int8.onnx"),
    asset!("tokens.txt", "tokens.txt"),
];

/// Runtime paths for one offline recogniser.
pub struct GipformerAssets {
    pub encoder: PathBuf,
    pub decoder: PathBuf,
    pub joiner: PathBuf,
    pub tokens: PathBuf,
}

pub fn model_dir() -> PathBuf {
    model_path("ASR", "gipformer")
}

pub struct GipformerAssetManager;

impl ProviderAssetManager for GipformerAssetManager {
    fn ensure_assets(&self) -> Result<(), AssetError> {
        ensure_assets_into(http_acquirer(), &model_dir())
    }

    fn revision(&self) -> &'static str {
        MODEL_REVISION
    }
}

static ASSET_MANAGER: GipformerAssetManager = GipformerAssetManager;

pub static ASSETS: &GipformerAssetManager = &ASSET_MANAGER;

/// Takes the root explicitly so tests install into a temporary directory.
pub fn ensure_assets_into(acquirer: &dyn AssetAcquirer, root: &Path) -> Result<(), AssetError> {
    for asset in CORE_ASSETS {
        ensure_asset(acquirer, root, asset)?;
    }
    Ok(())
}

pub fn resolve_assets_from(root: &Path) -> Result<GipformerAssets, AssetError> {
    Ok(GipformerAssets {
        encoder: ready(root, "encoder.onnx")?,
        decoder: ready(root, "decoder.onnx")?,
        joiner: ready(root, "joiner.onnx")?,
        tokens: ready(root, "tokens.txt")?,
    })
}

pub fn resolve_assets() -> Result<GipformerAssets, AssetError> {
    resolve_assets_from(&model_dir())
}

fn ready(root: &Path, relative: &str) -> Result<PathBuf, AssetError> {
    let path = root.join(relative);
    is_ready(&path)
        .then_some(path.clone())
        .ok_or(AssetError::Missing(path))
}

/// Declares one file: the name it is installed under locally, and its upstream name, which differs
/// only by the quantisation suffix the int8 checkpoint carries.
macro_rules! asset {
    ($path:literal, $remote:literal) => {
        Asset {
            path: $path,
            url: concat!(
                "https://huggingface.co/g-group-ai-lab/gipformer1.5-68M-rnnt/resolve/",
                "dd9227dcd8705c13f33bdbe59728d546ab94480f/",
                $remote
            ),
        }
    };
}

use asset;
