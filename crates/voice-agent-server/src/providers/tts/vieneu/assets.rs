//! Provider-owned, immutable model assets for the VieNeu v3 Turbo and v3 Nano ONNX FP32 runtimes.
//!
//! The Turbo codec files are pinned to the same MOSS Nano ONNX export already deployed
//! by ZeroTTS in this repo, while v3 Nano uses its self-contained 4-graph OT-CFM export.

use std::path::{Path, PathBuf};

use super::config::VieneuModel;
use crate::providers::assets::{
    Asset, AssetAcquirer, AssetError, ProviderAssetManager, ensure_asset, http_acquirer,
    model_path, required,
};

pub const MODEL_REVISION: &str = "8b7e9cffb4b41918cb638b9f62f0a751184d14a6";
pub const CODEC_REVISION: &str = "c2bfbd67dc648cac455077333f7cf5c18a2e3bb4";
pub const PRESET_REVISION: &str = "85344322b7258b4e25479b692e8e3396baf9db34";
pub const NANO_MODEL_REVISION: &str = "aba295eb96a6fa6003ebe417cc1f2802a7adc1dc";
pub const NANO_PRESET_REVISION: &str = "04044e1b8f822f946bafb2c88b716459f4164152";
pub const G2P_REVISION: &str = "e825173f235d08ea19315b2b279fb11153b44cea";
/// All pinned upstream sources contribute to the physical asset profile.
pub const ASSET_REVISION: &str = concat!(
    "8b7e9cffb4b41918cb638b9f62f0a751184d14a6:",
    "c2bfbd67dc648cac455077333f7cf5c18a2e3bb4:",
    "85344322b7258b4e25479b692e8e3396baf9db34:",
    "aba295eb96a6fa6003ebe417cc1f2802a7adc1dc:",
    "04044e1b8f822f946bafb2c88b716459f4164152:",
    "e825173f235d08ea19315b2b279fb11153b44cea"
);

macro_rules! model_asset {
    ($name:literal) => {
        Asset {
            path: concat!("onnx_update/", $name),
            url: concat!(
                "https://huggingface.co/pnnbao-ump/VieNeu-TTS-v3-Turbo/resolve/",
                "8b7e9cffb4b41918cb638b9f62f0a751184d14a6",
                "/onnx_update/",
                $name
            ),
        }
    };
}

macro_rules! codec_asset {
    ($name:literal) => {
        Asset {
            path: concat!("onnx/codec/", $name),
            url: concat!(
                "https://huggingface.co/zeroweight-ai/ZeroTTS/resolve/",
                "c2bfbd67dc648cac455077333f7cf5c18a2e3bb4",
                "/onnx/codec/",
                $name
            ),
        }
    };
}

macro_rules! nano_model_asset {
    ($name:literal) => {
        Asset {
            path: concat!("v3_nano/", $name),
            url: concat!(
                "https://huggingface.co/pnnbao-ump/VieNeu-TTS-v3-Nano/resolve/",
                "aba295eb96a6fa6003ebe417cc1f2802a7adc1dc",
                "/",
                $name
            ),
        }
    };
}

const G2P_ASSET: Asset = Asset {
    path: "sea_g2p.bin",
    url: concat!(
        "https://raw.githubusercontent.com/pnnbao97/sea-g2p/",
        "e825173f235d08ea19315b2b279fb11153b44cea",
        "/python/sea_g2p/sea_g2p.bin"
    ),
};

const NANO_PRESETS_ASSET: Asset = Asset {
    path: "v3_nano/voices_v3_nano.json",
    url: concat!(
        "https://raw.githubusercontent.com/pnnbao97/VieNeu-TTS/",
        "04044e1b8f822f946bafb2c88b716459f4164152",
        "/src/vieneu/assets/voices_v3_nano.json"
    ),
};

pub const CORE_ASSETS: &[Asset] = &[
    model_asset!("config.json"),
    model_asset!("tokenizer.json"),
    model_asset!("vieneu_prefill.onnx"),
    model_asset!("vieneu_decode_step.onnx"),
    model_asset!("vieneu_acoustic_cached.onnx"),
    model_asset!("vieneu_backbone_shared.data"),
    model_asset!("vieneu_v3_heads.npz"),
    codec_asset!("moss_audio_tokenizer_decode_step.onnx"),
    codec_asset!("moss_audio_tokenizer_decode_shared.data"),
    codec_asset!("codec_browser_onnx_meta.json"),
    codec_asset!("LICENSE-Apache-2.0.txt"),
    Asset {
        path: "voices_v3_turbo.json",
        url: concat!(
            "https://raw.githubusercontent.com/pnnbao97/VieNeu-TTS/",
            "85344322b7258b4e25479b692e8e3396baf9db34",
            "/src/vieneu/assets/voices_v3_turbo.json"
        ),
    },
    G2P_ASSET,
];

pub const NANO_ASSETS: &[Asset] = &[
    nano_model_asset!("config.json"),
    nano_model_asset!("constants.npz"),
    nano_model_asset!("text_encoder.onnx"),
    nano_model_asset!("duration_predictor.onnx"),
    nano_model_asset!("vector_estimator.onnx"),
    nano_model_asset!("codec_decoder.onnx"),
    NANO_PRESETS_ASSET,
    G2P_ASSET,
];

pub const ALL_ASSETS: &[Asset] = &[
    model_asset!("config.json"),
    model_asset!("tokenizer.json"),
    model_asset!("vieneu_prefill.onnx"),
    model_asset!("vieneu_decode_step.onnx"),
    model_asset!("vieneu_acoustic_cached.onnx"),
    model_asset!("vieneu_backbone_shared.data"),
    model_asset!("vieneu_v3_heads.npz"),
    codec_asset!("moss_audio_tokenizer_decode_step.onnx"),
    codec_asset!("moss_audio_tokenizer_decode_shared.data"),
    codec_asset!("codec_browser_onnx_meta.json"),
    codec_asset!("LICENSE-Apache-2.0.txt"),
    Asset {
        path: "voices_v3_turbo.json",
        url: concat!(
            "https://raw.githubusercontent.com/pnnbao97/VieNeu-TTS/",
            "85344322b7258b4e25479b692e8e3396baf9db34",
            "/src/vieneu/assets/voices_v3_turbo.json"
        ),
    },
    nano_model_asset!("config.json"),
    nano_model_asset!("constants.npz"),
    nano_model_asset!("text_encoder.onnx"),
    nano_model_asset!("duration_predictor.onnx"),
    nano_model_asset!("vector_estimator.onnx"),
    nano_model_asset!("codec_decoder.onnx"),
    NANO_PRESETS_ASSET,
    G2P_ASSET,
];

/// Stable database IDs are independent of the Unicode upstream source-name.
#[derive(Clone, Copy, Debug)]
pub struct PresetVoice {
    pub id: &'static str,
    pub name: &'static str,
    pub source_name: &'static str,
}

macro_rules! voice {
    ($id:literal, $name:literal) => {
        PresetVoice {
            id: $id,
            name: $name,
            source_name: $name,
        }
    };
}

pub const TURBO_DEFAULT_VOICE: &str = "hai_dang";
pub const DEFAULT_VOICE: &str = TURBO_DEFAULT_VOICE;
pub const NANO_DEFAULT_VOICE: &str = "minh_quan";

pub const TURBO_VOICES: &[PresetVoice] = &[
    voice!("adam_bua", "Adam bựa"),
    voice!("truc_ly", "Trúc Ly"),
    voice!("thien_minh", "Thiện Minh"),
    voice!("mai_anh", "Mai Anh"),
    voice!("hai_dang", "Hải Đăng"),
    voice!("thuy_dung", "Thùy Dung"),
    voice!("thien_tam_duc", "Thiền Tâm Đức"),
    voice!("ngoc_huyen", "Ngọc Huyền"),
    voice!("quang_son", "Quang Sơn"),
    voice!("ngoc_tran", "Ngọc Trân"),
    voice!("minh_duc", "Minh Đức"),
    voice!("pham_tuyen", "Phạm Tuyên"),
    voice!("thai_son", "Thái Sơn"),
    voice!("xuan_vinh", "Xuân Vĩnh"),
    voice!("thanh_binh", "Thanh Bình"),
    voice!("ngoc_linh", "Ngọc Linh"),
    voice!("doan_trang", "Đoan Trang"),
    voice!("thuc_doan", "Thục Đoan"),
    voice!("minh_triet", "Minh Triết"),
    voice!("my_duyen", "Mỹ Duyên"),
    voice!("quynh_anh", "Quỳnh Anh"),
    voice!("duc_tri", "Đức Trí"),
    voice!("kim_thanh", "Kim Thanh"),
    voice!("adam", "Adam"),
    voice!("quoc_tuan", "Quốc Tuấn"),
];
pub const VOICES: &[PresetVoice] = TURBO_VOICES;

pub const NANO_VOICES: &[PresetVoice] = &[
    voice!("minh_quan", "Minh Quân"),
    voice!("adam", "Adam"),
    voice!("my_duyen", "Mỹ Duyên"),
    voice!("duc_tri", "Đức Trí"),
    voice!("mai_anh", "Mai Anh"),
    voice!("truc_ly", "Trúc Ly"),
    voice!("ai_han", "Ái Hân"),
    voice!("huu_quan", "Hữu Quân"),
    voice!("xuan_tien", "Xuân Tiên"),
    voice!("anh_khoi", "Anh Khôi"),
    voice!("manh_dung", "Mạnh Dũng"),
];

pub const fn voices_for_model(model: VieneuModel) -> &'static [PresetVoice] {
    match model {
        VieneuModel::V3TurboFp32 => TURBO_VOICES,
        VieneuModel::V3NanoFp32 => NANO_VOICES,
    }
}

pub fn model_dir() -> PathBuf {
    model_path("TTS", "vieneu")
}

/// A resolved, already-prepared Turbo artifact set. No download occurs during construction.
pub struct VieneuNativeAssets {
    pub config: PathBuf,
    pub tokenizer: PathBuf,
    pub prefill: PathBuf,
    pub decode_step: PathBuf,
    pub acoustic: PathBuf,
    pub backbone_data: PathBuf,
    pub heads: PathBuf,
    pub codec_step: PathBuf,
    pub codec_data: PathBuf,
    pub codec_metadata: PathBuf,
    pub codec_license: PathBuf,
    pub presets: PathBuf,
    pub g2p_dictionary: PathBuf,
}

/// A resolved, already-prepared Nano artifact set. No download occurs during construction.
pub struct VieneuNanoAssets {
    pub config: PathBuf,
    pub constants: PathBuf,
    pub text_encoder: PathBuf,
    pub duration_predictor: PathBuf,
    pub vector_estimator: PathBuf,
    pub codec_decoder: PathBuf,
    pub presets: PathBuf,
    pub g2p_dictionary: PathBuf,
}

pub enum VieneuModelAssets {
    Turbo(VieneuNativeAssets),
    Nano(VieneuNanoAssets),
}

pub struct VieneuNativeAssetManager;
pub static ASSETS: &VieneuNativeAssetManager = &VieneuNativeAssetManager;

impl ProviderAssetManager for VieneuNativeAssetManager {
    fn ensure_assets(&self) -> Result<(), AssetError> {
        ensure_assets_into(http_acquirer(), &model_dir())
    }
    fn revision(&self) -> &'static str {
        ASSET_REVISION
    }
}

pub fn ensure_assets_into(acquirer: &dyn AssetAcquirer, root: &Path) -> Result<(), AssetError> {
    for asset in ALL_ASSETS {
        ensure_asset(acquirer, root, asset)?;
    }
    Ok(())
}

pub fn resolve_assets_from(root: &Path) -> Result<VieneuNativeAssets, AssetError> {
    Ok(VieneuNativeAssets {
        config: required(root, "onnx_update/config.json")?,
        tokenizer: required(root, "onnx_update/tokenizer.json")?,
        prefill: required(root, "onnx_update/vieneu_prefill.onnx")?,
        decode_step: required(root, "onnx_update/vieneu_decode_step.onnx")?,
        acoustic: required(root, "onnx_update/vieneu_acoustic_cached.onnx")?,
        backbone_data: required(root, "onnx_update/vieneu_backbone_shared.data")?,
        heads: required(root, "onnx_update/vieneu_v3_heads.npz")?,
        codec_step: required(root, "onnx/codec/moss_audio_tokenizer_decode_step.onnx")?,
        codec_data: required(root, "onnx/codec/moss_audio_tokenizer_decode_shared.data")?,
        codec_metadata: required(root, "onnx/codec/codec_browser_onnx_meta.json")?,
        codec_license: required(root, "onnx/codec/LICENSE-Apache-2.0.txt")?,
        presets: required(root, "voices_v3_turbo.json")?,
        g2p_dictionary: required(root, "sea_g2p.bin")?,
    })
}

pub fn resolve_nano_assets_from(root: &Path) -> Result<VieneuNanoAssets, AssetError> {
    Ok(VieneuNanoAssets {
        config: required(root, "v3_nano/config.json")?,
        constants: required(root, "v3_nano/constants.npz")?,
        text_encoder: required(root, "v3_nano/text_encoder.onnx")?,
        duration_predictor: required(root, "v3_nano/duration_predictor.onnx")?,
        vector_estimator: required(root, "v3_nano/vector_estimator.onnx")?,
        codec_decoder: required(root, "v3_nano/codec_decoder.onnx")?,
        presets: required(root, "v3_nano/voices_v3_nano.json")?,
        g2p_dictionary: required(root, "sea_g2p.bin")?,
    })
}

pub fn resolve_model_assets_from(
    root: &Path,
    model: VieneuModel,
) -> Result<VieneuModelAssets, AssetError> {
    match model {
        VieneuModel::V3TurboFp32 => resolve_assets_from(root).map(VieneuModelAssets::Turbo),
        VieneuModel::V3NanoFp32 => resolve_nano_assets_from(root).map(VieneuModelAssets::Nano),
    }
}

pub fn resolve_assets() -> Result<VieneuNativeAssets, AssetError> {
    resolve_assets_from(&model_dir())
}

pub fn resolve_nano_assets() -> Result<VieneuNanoAssets, AssetError> {
    resolve_nano_assets_from(&model_dir())
}

pub fn resolve_model_assets(model: VieneuModel) -> Result<VieneuModelAssets, AssetError> {
    resolve_model_assets_from(&model_dir(), model)
}

#[cfg(test)]
#[path = "assets_tests.rs"]
mod tests;
