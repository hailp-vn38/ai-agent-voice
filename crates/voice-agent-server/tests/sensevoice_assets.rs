//! SenseVoiceSmall asset behaviour: missing or zero-byte model files are downloaded, existing
//! files are reused without network calls, and `resolve_assets_from` checks readiness without
//! downloading.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use voice_agent_server::providers::{
    asr::sensevoice::assets,
    assets::{AssetAcquirer, AssetError},
};

#[derive(Clone, Default)]
struct CatalogAcquirer {
    downloads: Arc<Mutex<Vec<String>>>,
}

impl CatalogAcquirer {
    fn new() -> Self {
        Self::default()
    }

    fn requested(&self) -> BTreeSet<String> {
        self.downloads.lock().unwrap().iter().cloned().collect()
    }

    fn count(&self) -> usize {
        self.downloads.lock().unwrap().len()
    }
}

impl AssetAcquirer for CatalogAcquirer {
    fn acquire(&self, url: &str, destination: &Path) -> Result<(), AssetError> {
        self.downloads.lock().unwrap().push(url.to_owned());
        fs::write(destination, url.as_bytes()).map_err(AssetError::Io)
    }
}

fn temp_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "voice-sensevoice-{}-{}",
        name,
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

#[test]
fn missing_sensevoice_assets_are_downloaded_and_reused_idempotently() {
    let root = temp_root("install");

    // Before installation, resolve_assets_from must fail with AssetError::Missing.
    assert!(matches!(
        assets::resolve_assets_from(&root),
        Err(AssetError::Missing(_))
    ));

    // First ensure downloads both model.onnx (from model.int8.onnx) and tokens.txt.
    let first = CatalogAcquirer::new();
    assets::ensure_assets_into(&first, &root).unwrap();
    let requested = first.requested();
    assert_eq!(first.count(), 2);
    for asset in assets::CORE_ASSETS {
        assert!(
            requested.contains(asset.url),
            "asset `{}` was not downloaded",
            asset.path
        );
    }

    let resolved = assets::resolve_assets_from(&root).unwrap();
    assert!(resolved.model.is_file() && resolved.model.metadata().unwrap().len() > 0);
    assert!(resolved.tokens.is_file() && resolved.tokens.metadata().unwrap().len() > 0);

    // Second ensure downloads nothing further.
    let second = CatalogAcquirer::new();
    assets::ensure_assets_into(&second, &root).unwrap();
    assert_eq!(second.count(), 0);

    // Zero-byte model file is detected as not ready and reinstalled.
    fs::write(&resolved.model, b"").unwrap();
    assert!(matches!(
        assets::resolve_assets_from(&root),
        Err(AssetError::Missing(_))
    ));
    let third = CatalogAcquirer::new();
    assets::ensure_assets_into(&third, &root).unwrap();
    assert_eq!(third.count(), 1);
    assert!(assets::resolve_assets_from(&root).is_ok());

    fs::remove_dir_all(root).unwrap();
}
