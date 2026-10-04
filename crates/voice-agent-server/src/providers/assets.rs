//! Provider-owned model assets.
//!
//! Every local provider declares the files it needs in its own `assets.rs`, downloads them on
//! demand, and hands resolved paths to its runtime. This module owns the shared primitives only:
//! where models live, when a file counts as ready, and how a download becomes visible.
//!
//! Integrity is deliberately shallow: a regular file with a non-zero size is ready. Verification
//! belongs to runtime initialization, which fails loudly if a model cannot be loaded.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard, OnceLock},
};

/// Model root, relative to the process working directory. Deliberately not configurable: where
/// models live is an implementation detail, not deployment policy.
pub const MODELS_ROOT: &str = "models";

/// Directory a provider's assets live in, e.g. `model_path("TTS", "zerotts")`.
pub fn model_path(kind: &str, provider: &str) -> PathBuf {
    Path::new(MODELS_ROOT).join(kind).join(provider)
}

/// One file a provider needs before it can build a runtime.
pub struct Asset {
    pub path: &'static str,
    pub url: &'static str,
}

/// One selectable voice, which is also a file the provider must have on disk.
pub struct VoiceAsset {
    pub id: &'static str,
    pub name: &'static str,
    pub path: &'static str,
    pub url: &'static str,
}

#[derive(Debug, thiserror::Error)]
pub enum AssetError {
    #[error("provider asset I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("provider asset download failed: {0}")]
    Download(String),
    #[error("provider asset download produced no content: {0}")]
    InvalidDownload(PathBuf),
    #[error("provider asset transform failed: {0}")]
    Transform(String),
    #[error("provider asset is missing: {0}")]
    Missing(PathBuf),
}

/// A file counts as ready when it exists as a regular file with content. No checksum, no mtime,
/// no parsing: a corrupt model is caught by runtime initialization, not by an asset scan.
pub fn is_ready(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|metadata| metadata.is_file() && metadata.len() > 0)
}

/// Resolves one declared file that must already be installed. A missing file is an error rather
/// than a silent download, so callers can tell "not prepared yet" from "prepared".
pub fn required(root: &Path, relative: &str) -> Result<PathBuf, AssetError> {
    let path = root.join(relative);
    is_ready(&path)
        .then_some(path.clone())
        .ok_or(AssetError::Missing(path))
}

/// Fetches one remote URL to a local destination.
pub trait AssetAcquirer: Send + Sync {
    fn acquire(&self, url: &str, destination: &Path) -> Result<(), AssetError>;
}

pub trait ProviderAssetManager: Send + Sync {
    /// Downloads every missing file this provider needs. Safe to call concurrently and repeatedly.
    fn ensure_assets(&self) -> Result<(), AssetError>;

    /// The pinned upstream revision these files come from. Two deployments on different revisions
    /// must not share one physical runtime, so this participates in resource identity.
    fn revision(&self) -> &'static str;
}

/// Ensures one asset exists, returning its resolved path.
///
/// Reuses an existing file, otherwise downloads to a temporary sibling and renames it into place,
/// so an interrupted transfer can never leave a truncated file looking ready.
pub fn ensure_asset(
    acquirer: &dyn AssetAcquirer,
    root: &Path,
    asset: &Asset,
) -> Result<PathBuf, AssetError> {
    let target = root.join(asset.path);
    if is_ready(&target) {
        tracing::info!(asset = asset.path, destination = %target.display(), "reusing provider asset");
        return Ok(target);
    }
    let _guard = asset_lock(&target);
    // Another caller may have installed it while this one waited.
    if is_ready(&target) {
        tracing::info!(asset = asset.path, destination = %target.display(), "reusing provider asset");
        return Ok(target);
    }
    tracing::info!(asset = asset.path, url = asset.url, destination = %target.display(), "downloading provider asset");
    install(acquirer, asset.url, &target)?;
    tracing::info!(asset = asset.path, destination = %target.display(), "provider asset ready");
    Ok(target)
}

pub(crate) fn http_acquirer() -> &'static dyn AssetAcquirer {
    &crate::assets::HttpAssetAcquirer
}

fn install(acquirer: &dyn AssetAcquirer, url: &str, target: &Path) -> Result<(), AssetError> {
    let parent = target
        .parent()
        .ok_or(AssetError::Missing(target.to_path_buf()))?;
    fs::create_dir_all(parent)?;
    let part = PathBuf::from(format!("{}.part", target.display()));
    let _ = fs::remove_file(&part);
    let result = (|| {
        acquirer.acquire(url, &part)?;
        if !is_ready(&part) {
            return Err(AssetError::InvalidDownload(part.clone()));
        }
        fs::rename(&part, target)?;
        Ok(())
    })();
    // A failed attempt must leave nothing behind, so the next attempt starts clean.
    let _ = fs::remove_file(&part);
    result
}

/// Publishes bytes a provider derived itself, using the same temporary-file discipline as a
/// download so a failed transform never leaves a half-written file looking ready.
pub fn install_bytes(content: &[u8], target: &Path) -> Result<(), AssetError> {
    if content.is_empty() {
        return Err(AssetError::InvalidDownload(target.to_path_buf()));
    }
    let parent = target
        .parent()
        .ok_or(AssetError::Missing(target.to_path_buf()))?;
    fs::create_dir_all(parent)?;
    let part = PathBuf::from(format!("{}.part", target.display()));
    let _ = fs::remove_file(&part);
    let result = (|| {
        fs::write(&part, content)?;
        fs::rename(&part, target)?;
        Ok(())
    })();
    let _ = fs::remove_file(&part);
    result
}

fn asset_lock(path: &Path) -> MutexGuard<'static, ()> {
    static LOCKS: OnceLock<[Mutex<()>; 64]> = OnceLock::new();
    LOCKS.get_or_init(|| std::array::from_fn(|_| Mutex::new(())))[stripe(path)]
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Striped by canonical path so two spellings of the same destination share one lock.
fn stripe(path: &Path) -> usize {
    use std::hash::{Hash, Hasher};
    let mut existing = path;
    let mut missing = Vec::new();
    while !existing.exists() {
        let Some(name) = existing.file_name() else {
            break;
        };
        let Some(parent) = existing.parent() else {
            break;
        };
        missing.push(name.to_owned());
        existing = parent;
    }
    let mut normalized = fs::canonicalize(existing).unwrap_or_else(|_| existing.to_owned());
    for name in missing.into_iter().rev() {
        normalized.push(name);
    }
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    normalized.hash(&mut hash);
    hash.finish() as usize % 64
}
