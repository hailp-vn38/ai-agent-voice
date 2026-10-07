//! Official sherpa-onnx CAM++ zh/en advanced model, release asset 198893102.
use crate::providers::assets::{
    Asset, AssetError, ProviderAssetManager, ensure_asset, http_acquirer, model_path, required,
};
use std::path::PathBuf;
pub const MODEL_REVISION: &str = "speaker-recongition-models-asset-198893102";
pub const MODEL: Asset = Asset {
    path: "campplus.onnx",
    url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/3dspeaker_speech_campplus_sv_zh_en_16k-common_advanced.onnx",
};
pub struct CampPlusAssets;
impl ProviderAssetManager for CampPlusAssets {
    fn ensure_assets(&self) -> Result<(), AssetError> {
        ensure_asset(
            http_acquirer(),
            &model_path("Speaker", "campplus-198893102"),
            &MODEL,
        )
        .map(|_| ())
    }
    fn revision(&self) -> &'static str {
        MODEL_REVISION
    }
}
pub static ASSETS: CampPlusAssets = CampPlusAssets;
pub fn resolve_assets() -> Result<PathBuf, AssetError> {
    required(&model_path("Speaker", "campplus-198893102"), MODEL.path)
}
