//! Zipformer streaming Vietnamese ASR model files.
//!
//! The upstream checkpoint ships a SentencePiece BPE model, while sherpa-onnx expects a plain
//! token list. That conversion belongs to this provider: it is a property of how Zipformer is
//! consumed, not a generic property of "downloading a model".

use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::providers::assets::{
    Asset, AssetAcquirer, AssetError, ProviderAssetManager, ensure_asset, http_acquirer, is_ready,
    model_path,
};

/// Pinned upstream revision.
pub const MODEL_REVISION: &str = "c122fdc21cea4894fd775e9d3fe66ebbc787e26b";

/// The three transducer graphs are copied verbatim.
pub const CORE_ASSETS: &[Asset] = &[
    Asset {
        path: "encoder.onnx",
        url: concat!(
            "https://huggingface.co/hynt/Zipformer-30M-RNNT-Streaming-6000h/resolve/",
            "c122fdc21cea4894fd775e9d3fe66ebbc787e26b",
            "/encoder-epoch-31-avg-11-chunk-32-left-128.fp16.onnx"
        ),
    },
    Asset {
        path: "decoder.onnx",
        url: concat!(
            "https://huggingface.co/hynt/Zipformer-30M-RNNT-Streaming-6000h/resolve/",
            "c122fdc21cea4894fd775e9d3fe66ebbc787e26b",
            "/decoder-epoch-31-avg-11-chunk-32-left-128.fp16.onnx"
        ),
    },
    Asset {
        path: "joiner.onnx",
        url: concat!(
            "https://huggingface.co/hynt/Zipformer-30M-RNNT-Streaming-6000h/resolve/",
            "c122fdc21cea4894fd775e9d3fe66ebbc787e26b",
            "/joiner-epoch-31-avg-11-chunk-32-left-128.fp16.onnx"
        ),
    },
];

/// The BPE model, converted into the token list sherpa-onnx reads.
pub const TOKENS_SOURCE: Asset = Asset {
    path: "bpe.model",
    url: concat!(
        "https://huggingface.co/hynt/Zipformer-30M-RNNT-Streaming-6000h/resolve/",
        "c122fdc21cea4894fd775e9d3fe66ebbc787e26b",
        "/bpe.model"
    ),
};

/// Installed name of the converted token list.
pub const TOKENS_PATH: &str = "tokens.txt";

/// Runtime paths for one online recogniser.
pub struct ZipformerAssets {
    pub encoder: PathBuf,
    pub decoder: PathBuf,
    pub joiner: PathBuf,
    pub tokens: PathBuf,
}

pub fn model_dir() -> PathBuf {
    model_path("ASR", "zipformer")
}

pub struct ZipformerAssetManager;

impl ProviderAssetManager for ZipformerAssetManager {
    fn ensure_assets(&self) -> Result<(), AssetError> {
        ensure_assets_into(http_acquirer(), &model_dir())
    }

    fn revision(&self) -> &'static str {
        MODEL_REVISION
    }
}

static ASSET_MANAGER: ZipformerAssetManager = ZipformerAssetManager;

pub static ASSETS: &ZipformerAssetManager = &ASSET_MANAGER;

/// Ensures the three graphs and, if the token list is absent, derives it from the BPE model.
///
/// Takes the root explicitly so tests install into a temporary directory.
pub fn ensure_assets_into(acquirer: &dyn AssetAcquirer, root: &Path) -> Result<(), AssetError> {
    for asset in CORE_ASSETS {
        ensure_asset(acquirer, root, asset)?;
    }
    let tokens = root.join(TOKENS_PATH);
    if is_ready(&tokens) {
        return Ok(());
    }
    let source = ensure_asset(acquirer, root, &TOKENS_SOURCE)?;
    let input = fs::read(&source)?;
    let converted = sentencepiece_tokens(&input)?;
    crate::providers::assets::install_bytes(&converted, &tokens)
}

/// Resolved paths for an already-prepared model root. Reads metadata only.
pub fn resolve_assets_from(root: &Path) -> Result<ZipformerAssets, AssetError> {
    Ok(ZipformerAssets {
        encoder: ready(root, "encoder.onnx")?,
        decoder: ready(root, "decoder.onnx")?,
        joiner: ready(root, "joiner.onnx")?,
        tokens: ready(root, TOKENS_PATH)?,
    })
}

pub fn resolve_assets() -> Result<ZipformerAssets, AssetError> {
    resolve_assets_from(&model_dir())
}

fn ready(root: &Path, relative: &str) -> Result<PathBuf, AssetError> {
    let path = root.join(relative);
    is_ready(&path)
        .then_some(path.clone())
        .ok_or(AssetError::Missing(path))
}

/// Reads a SentencePiece BPE model as a plain piece list.
///
/// Only the vocabulary entries are read; the rest of the protobuf is skipped. Output lines are
/// `"<piece> <index>"`, which is the format sherpa-onnx's `tokens` file expects.
fn sentencepiece_tokens(input: &[u8]) -> Result<Vec<u8>, AssetError> {
    let invalid = || AssetError::Transform("incompatible SentencePiece model".into());
    let mut offset = 0;
    let mut tokens = Vec::new();
    while offset < input.len() {
        let tag = read_varint(input, &mut offset).ok_or_else(invalid)?;
        let length = read_varint(input, &mut offset).ok_or_else(invalid)? as usize;
        let end = offset
            .checked_add(length)
            .filter(|end| *end <= input.len())
            .ok_or_else(invalid)?;
        // Field 1 of ModelProto is a SentencePiece message; every other field is skipped.
        if tag == 0x0a {
            let nested = &input[offset..end];
            let mut piece_offset = 0;
            let piece_tag = read_varint(nested, &mut piece_offset).ok_or_else(invalid)?;
            let piece_length = read_varint(nested, &mut piece_offset).ok_or_else(invalid)? as usize;
            let piece_end = piece_offset
                .checked_add(piece_length)
                .filter(|end| *end <= nested.len())
                .ok_or_else(invalid)?;
            if piece_tag != 0x0a {
                return Err(invalid());
            }
            let piece =
                std::str::from_utf8(&nested[piece_offset..piece_end]).map_err(|_| invalid())?;
            tokens.push(format!("{piece} {}\n", tokens.len()));
        }
        offset = end;
    }
    (!tokens.is_empty())
        .then(|| tokens.concat().into_bytes())
        .ok_or_else(invalid)
}

fn read_varint(input: &[u8], offset: &mut usize) -> Option<u64> {
    let mut value = 0_u64;
    for shift in (0..64).step_by(7) {
        let byte = *input.get(*offset)?;
        *offset += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}
