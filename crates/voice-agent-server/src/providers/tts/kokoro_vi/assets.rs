//! Kokoro VI model files.
//!
//! The upstream release publishes an ONNX graph, a tokenizer config, and one PyTorch voicepack per
//! voice. Turning a voicepack into the flat float table the runtime reads is a Kokoro concern, so
//! the conversion lives here rather than in a generic downloader.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use crate::providers::assets::{
    Asset, AssetAcquirer, AssetError, ProviderAssetManager, ensure_asset, http_acquirer,
    install_bytes, is_ready, model_path, required,
};

mod voicepack_source;

/// Pinned upstream revision.
pub const MODEL_REVISION: &str = "9f210d622209fcc216fe2ac6159fed2ff381cb8a";

/// The graph and tokenizer config are copied verbatim.
pub const CORE_ASSETS: &[Asset] = &[
    Asset {
        path: "kokoro_vi.onnx",
        url: concat!(
            "https://huggingface.co/contextboxai/Kokoro-Vietnamese/resolve/",
            "9f210d622209fcc216fe2ac6159fed2ff381cb8a",
            "/kokoro_vi.onnx"
        ),
    },
    Asset {
        path: "config.json",
        url: concat!(
            "https://huggingface.co/contextboxai/Kokoro-Vietnamese/resolve/",
            "9f210d622209fcc216fe2ac6159fed2ff381cb8a",
            "/config.json"
        ),
    },
];

/// One voice. Unlike a plain file asset, the runtime reads a derived file: upstream publishes a
/// PyTorch archive and the provider installs the converted `.bin` beside it.
pub struct Voicepack {
    pub id: &'static str,
    pub name: &'static str,
    /// Converted file the runtime reads.
    pub path: &'static str,
    /// Upstream archive this is derived from.
    pub source_path: &'static str,
    pub url: &'static str,
}

/// Every voicepack this provider supports.
pub const VOICES: &[Voicepack] = &[
    voice!("diem_trinh", "Diem Trinh"),
    voice!("duc_an", "Duc An"),
    voice!("duc_duy", "Duc Duy"),
    voice!("hung_thinh", "Hung Thinh"),
    voice!("mai_linh", "Mai Linh"),
    voice!("mai_loan", "Mai Loan"),
    voice!("manh_dung", "Manh Dung"),
    voice!("my_yen", "My Yen"),
    voice!("ngoc_huyen", "Ngoc Huyen"),
    voice!("phat_tai", "Phat Tai"),
    voice!("storyvert", "Storyvert"),
    voice!("thanh_dat", "Thanh Dat"),
    voice!("thuc_trinh", "Thuc Trinh"),
    voice!("tuan_ngoc", "Tuan Ngoc"),
];

/// Runtime paths, resolved once the files exist.
pub struct KokoroViAssets {
    pub model: PathBuf,
    pub config: PathBuf,
    pub voicepacks: BTreeMap<String, PathBuf>,
}

impl KokoroViAssets {
    pub fn voicepack(&self, id: &str) -> Option<&Path> {
        self.voicepacks.get(id).map(PathBuf::as_path)
    }
}

pub fn model_dir() -> PathBuf {
    model_path("TTS", "kokoro_vi")
}

pub struct KokoroViAssetManager;

impl ProviderAssetManager for KokoroViAssetManager {
    fn ensure_assets(&self) -> Result<(), AssetError> {
        ensure_assets_into(http_acquirer(), &model_dir())
    }

    fn revision(&self) -> &'static str {
        MODEL_REVISION
    }
}

static ASSET_MANAGER: KokoroViAssetManager = KokoroViAssetManager;

pub static ASSETS: &KokoroViAssetManager = &ASSET_MANAGER;

/// Ensures the graph, the config, and every voicepack under `root`.
///
/// Takes the root explicitly so tests install into a temporary directory.
pub fn ensure_assets_into(acquirer: &dyn AssetAcquirer, root: &Path) -> Result<(), AssetError> {
    for asset in CORE_ASSETS {
        ensure_asset(acquirer, root, asset)?;
    }
    for voice in VOICES {
        ensure_voicepack(acquirer, root, voice)?;
    }
    Ok(())
}

/// Ensures one voicepack, converting it from the upstream archive the first time it is needed.
fn ensure_voicepack(
    acquirer: &dyn AssetAcquirer,
    root: &Path,
    voice: &Voicepack,
) -> Result<(), AssetError> {
    let target = root.join(voice.path);
    if is_ready(&target) {
        return Ok(());
    }
    let source = ensure_asset(
        acquirer,
        root,
        &Asset {
            path: voice.source_path,
            url: voice.url,
        },
    )?;
    let converted = voicepack_source::voicepack_v1(&fs::read(&source)?)?;
    install_bytes(&converted, &target)
}

/// Resolved paths for an already-prepared model root. Reads metadata only.
pub fn resolve_assets_from(root: &Path) -> Result<KokoroViAssets, AssetError> {
    let voicepacks = VOICES
        .iter()
        .map(|voice| Ok((voice.id.to_owned(), required(root, voice.path)?)))
        .collect::<Result<BTreeMap<_, _>, AssetError>>()?;
    Ok(KokoroViAssets {
        model: required(root, "kokoro_vi.onnx")?,
        config: required(root, "config.json")?,
        voicepacks,
    })
}

pub fn resolve_assets() -> Result<KokoroViAssets, AssetError> {
    resolve_assets_from(&model_dir())
}

/// Declares one voice. `path` is the converted form the runtime reads; `url` is the upstream
/// archive it is derived from.
macro_rules! voice {
    ($id:literal, $name:literal) => {
        Voicepack {
            id: $id,
            name: $name,
            path: concat!("voicepacks/", $id, ".bin"),
            source_path: concat!("voicepacks/", $id, ".pt"),
            url: concat!(
                "https://huggingface.co/contextboxai/Kokoro-Vietnamese/resolve/",
                "9f210d622209fcc216fe2ac6159fed2ff381cb8a",
                "/voicepacks/",
                $id,
                ".pt"
            ),
        }
    };
}

use voice;
