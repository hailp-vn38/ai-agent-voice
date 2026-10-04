//! Silero VAD model files.

use std::path::{Path, PathBuf};

use crate::providers::assets::{
    Asset, AssetAcquirer, AssetError, ProviderAssetManager, ensure_asset, http_acquirer, model_path,
};

/// Pinned upstream commit. Silero publishes the graph straight from its repository, so the commit
/// is the only immutable reference available.
pub const MODEL_REVISION: &str = "60b7ffa243625ebdc1070275a29f18c87843786a";

pub const CORE_ASSETS: &[Asset] = &[Asset {
    path: "silero_vad.onnx",
    url: concat!(
        "https://raw.githubusercontent.com/snakers4/silero-vad/",
        "60b7ffa243625ebdc1070275a29f18c87843786a",
        "/src/silero_vad/data/silero_vad.onnx"
    ),
}];

/// Runtime paths, resolved once the graph exists.
pub struct SileroAssets {
    pub model: PathBuf,
}

pub fn model_dir() -> PathBuf {
    model_path("VAD", "silero")
}

pub struct SileroAssetManager;

impl ProviderAssetManager for SileroAssetManager {
    fn ensure_assets(&self) -> Result<(), AssetError> {
        ensure_assets_into(http_acquirer(), &model_dir())
    }

    fn revision(&self) -> &'static str {
        MODEL_REVISION
    }
}

static ASSET_MANAGER: SileroAssetManager = SileroAssetManager;

pub static ASSETS: &SileroAssetManager = &ASSET_MANAGER;

/// Takes the root explicitly so tests install into a temporary directory.
pub fn ensure_assets_into(acquirer: &dyn AssetAcquirer, root: &Path) -> Result<(), AssetError> {
    ensure_asset(acquirer, root, &CORE_ASSETS[0]).map(|_| ())
}

pub fn resolve_assets_from(root: &Path) -> Result<SileroAssets, AssetError> {
    Ok(SileroAssets {
        model: ready(root, CORE_ASSETS[0].path)?,
    })
}

pub fn resolve_assets() -> Result<SileroAssets, AssetError> {
    resolve_assets_from(&model_dir())
}

fn ready(root: &Path, relative: &str) -> Result<PathBuf, AssetError> {
    let path = root.join(relative);
    crate::providers::assets::is_ready(&path)
        .then_some(path.clone())
        .ok_or(AssetError::Missing(path))
}
