//! ZeroTTS model files.
//!
//! This module is the authoritative declaration of what a ZeroTTS runtime needs: where its files
//! live, which upstream revision they come from, and how to make them exist on disk. Everything
//! else in the provider consumes resolved paths from here rather than guessing them.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use crate::providers::assets::{
    Asset, AssetAcquirer, AssetError, ProviderAssetManager, VoiceAsset, ensure_asset,
    http_acquirer, is_ready, model_path,
};

/// Pinned upstream revision. Embedding it in every URL is what makes a download immutable.
pub const MODEL_REVISION: &str = "c2bfbd67dc648cac455077333f7cf5c18a2e3bb4";

/// One core file: where it is installed locally, and where it comes from.
macro_rules! asset {
    ($path:literal) => {
        Asset {
            path: $path,
            url: concat!(
                "https://huggingface.co/zeroweight-ai/ZeroTTS/resolve/",
                "c2bfbd67dc648cac455077333f7cf5c18a2e3bb4",
                "/",
                $path,
            ),
        }
    };
}

/// One voice. `id` is the Admin-visible identifier, so its file layout and URL both follow from it.
macro_rules! voice {
    ($id:literal, $name:literal) => {
        VoiceAsset {
            id: $id,
            name: $name,
            path: concat!("voices/", $id, "/voice.npz"),
            url: concat!(
                "https://huggingface.co/zeroweight-ai/ZeroTTS/resolve/",
                "c2bfbd67dc648cac455077333f7cf5c18a2e3bb4",
                "/voices/",
                $id,
                "/voice.npz",
            ),
        }
    };
}

/// Directory holding this provider's files inside the shared model root.
pub fn model_dir() -> PathBuf {
    model_path("TTS", "zerotts")
}

/// The model graphs, tokenizer, voice index and bundled codec licence. All of these must exist
/// before a single audio sample can be synthesised.
pub const CORE_ASSETS: &[Asset] = &[
    asset!("config.json"),
    asset!("tokenizer.json"),
    asset!("null_voice_emb.npy"),
    asset!("silence_frame.npy"),
    asset!("voices/index.json"),
    asset!("onnx/text_encoder.onnx"),
    asset!("onnx/prefix_step.onnx"),
    asset!("onnx/local_frame_decode.onnx"),
    asset!("onnx/codec/moss_audio_tokenizer_decode_full.onnx"),
    asset!("onnx/codec/moss_audio_tokenizer_decode_step.onnx"),
    asset!("onnx/codec/moss_audio_tokenizer_decode_shared.data"),
    asset!("onnx/codec/codec_browser_onnx_meta.json"),
    asset!("onnx/codec/LICENSE-Apache-2.0.txt"),
];

/// Every voice this provider supports. One physical engine serves all of them, so all of them must
/// be on disk before any voice can be selected: installing only the configured voice would leave
/// the other voices selectable in the Admin API but unloadable at runtime.
pub const VOICES: &[VoiceAsset] = &[
    voice!("maichi", "Mai Chi"),
    voice!("baotrang", "Bao Trang"),
    voice!("giahuy", "Gia Huy"),
    voice!("hamy", "Ha My"),
    voice!("huuduc", "Huu Duc"),
    voice!("kimoanh", "Kim Oanh"),
    voice!("quangminh", "Quang Minh"),
    voice!("tiendat", "Tien Dat"),
];

/// Runtime paths, resolved once the files exist. The runtime reads these and never downloads.
pub struct ZeroTtsAssets {
    pub config: PathBuf,
    pub tokenizer: PathBuf,
    pub null_voice: PathBuf,
    pub silence_frame: PathBuf,
    pub voices_index: PathBuf,
    pub text_encoder: PathBuf,
    pub prefix_step: PathBuf,
    pub local_frame_decode: PathBuf,
    pub codec_decode_full: PathBuf,
    pub codec_decode_step: PathBuf,
    pub codec_shared_data: PathBuf,
    pub codec_metadata: PathBuf,
    pub codec_license: PathBuf,
    pub voices: BTreeMap<String, PathBuf>,
}

impl ZeroTtsAssets {
    pub fn voice(&self, id: &str) -> Option<&Path> {
        self.voices.get(id).map(PathBuf::as_path)
    }
}

pub struct ZeroTtsAssetManager;

impl ProviderAssetManager for ZeroTtsAssetManager {
    fn ensure_assets(&self) -> Result<(), AssetError> {
        ensure_assets_into(http_acquirer(), &model_dir())
    }
}

static ASSET_MANAGER: ZeroTtsAssetManager = ZeroTtsAssetManager;

pub static ASSETS: &ZeroTtsAssetManager = &ASSET_MANAGER;

/// Ensures every core file and every supported voice under `root`.
///
/// Takes the root explicitly so tests can install into a temporary directory instead of the
/// deployment model root; production callers use [`ensure_assets`].
pub fn ensure_assets_into(acquirer: &dyn AssetAcquirer, root: &Path) -> Result<(), AssetError> {
    for asset in CORE_ASSETS {
        ensure_asset(acquirer, root, asset)?;
    }
    for voice in VOICES {
        ensure_asset(
            acquirer,
            root,
            &Asset {
                path: voice.path,
                url: voice.url,
            },
        )?;
    }
    Ok(())
}

/// Resolved paths for an already-prepared model root. Reads metadata only: a missing file is an
/// error here rather than a silent download.
pub fn resolve_assets_from(root: &Path) -> Result<ZeroTtsAssets, AssetError> {
    let voices = VOICES
        .iter()
        .map(|voice| Ok((voice.id.to_owned(), ready(root, voice.path)?)))
        .collect::<Result<BTreeMap<_, _>, AssetError>>()?;
    Ok(ZeroTtsAssets {
        config: ready(root, "config.json")?,
        tokenizer: ready(root, "tokenizer.json")?,
        null_voice: ready(root, "null_voice_emb.npy")?,
        silence_frame: ready(root, "silence_frame.npy")?,
        voices_index: ready(root, "voices/index.json")?,
        text_encoder: ready(root, "onnx/text_encoder.onnx")?,
        prefix_step: ready(root, "onnx/prefix_step.onnx")?,
        local_frame_decode: ready(root, "onnx/local_frame_decode.onnx")?,
        codec_decode_full: ready(root, "onnx/codec/moss_audio_tokenizer_decode_full.onnx")?,
        codec_decode_step: ready(root, "onnx/codec/moss_audio_tokenizer_decode_step.onnx")?,
        codec_shared_data: ready(root, "onnx/codec/moss_audio_tokenizer_decode_shared.data")?,
        codec_metadata: ready(root, "onnx/codec/codec_browser_onnx_meta.json")?,
        codec_license: ready(root, "onnx/codec/LICENSE-Apache-2.0.txt")?,
        voices,
    })
}

/// Resolved paths from the deployment model root.
pub fn resolve_assets() -> Result<ZeroTtsAssets, AssetError> {
    resolve_assets_from(&model_dir())
}

fn ready(root: &Path, relative: &str) -> Result<PathBuf, AssetError> {
    let path = root.join(relative);
    if is_ready(&path) {
        Ok(path)
    } else {
        Err(AssetError::Missing(path))
    }
}
