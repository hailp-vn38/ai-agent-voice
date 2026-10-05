//! ZeroTTS asset behaviour: all supported voices are ensured, and the advertised voice catalog is
//! the same catalog that gets downloaded.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use voice_agent_server::providers::{
    assets::{AssetAcquirer, AssetError},
    tts::zerotts::{assets, descriptor},
};

/// Records every requested URL and writes it as file content, without touching the network.
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

/// A model root for one test, so nothing here reads or writes the real deployment model directory.
fn root(name: &str) -> PathBuf {
    let root =
        std::env::temp_dir().join(format!("voice-zerotts-{}-{}", name, uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    root
}

fn install(name: &str) -> (PathBuf, CatalogAcquirer) {
    let root = root(name);
    let acquirer = CatalogAcquirer::new();
    assets::ensure_assets_into(&acquirer, &root).unwrap();
    (root, acquirer)
}

#[test]
fn every_core_asset_is_ensured() {
    let (root, acquirer) = install("core");
    let requested = acquirer.requested();

    for asset in assets::CORE_ASSETS {
        assert!(
            requested.contains(asset.url),
            "asset `{}` was not downloaded",
            asset.path
        );
        let installed = root.join(asset.path);
        assert!(
            installed.is_file() && installed.metadata().unwrap().len() > 0,
            "asset `{}` is not installed at {}",
            asset.path,
            installed.display()
        );
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn every_supported_voice_is_ensured_not_only_the_selected_one() {
    let (root, acquirer) = install("voices");
    let requested = acquirer.requested();

    for voice in assets::VOICES {
        assert!(
            requested.contains(voice.url),
            "voice `{}` was not downloaded",
            voice.id
        );
        let installed = root.join(voice.path);
        assert!(
            installed.is_file() && installed.metadata().unwrap().len() > 0,
            "voice `{}` is not installed at {}",
            voice.id,
            installed.display()
        );
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn the_advertised_voice_catalog_is_the_downloaded_voice_catalog() {
    let (root, _) = install("catalog");
    let resolved = assets::resolve_assets_from(&root).unwrap();

    let advertised: BTreeSet<&str> = descriptor::DESCRIPTOR
        .capabilities
        .voices
        .expect("ZeroTTS advertises a static voice catalog")
        .iter()
        .map(|voice| voice.id)
        .collect();
    let ensured: BTreeSet<&str> = resolved.voices.keys().map(String::as_str).collect();

    // A voice a user may select must be a voice the provider can actually load, and the reverse.
    assert_eq!(advertised, ensured);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_second_ensure_downloads_nothing_further() {
    let (root, first) = install("idempotent");

    let second = CatalogAcquirer::new();
    assets::ensure_assets_into(&second, &root).unwrap();

    assert_eq!(second.count(), 0);
    assert!(first.count() > 0);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn resolving_assets_never_downloads() {
    let (root, _) = install("resolve-only");

    let probe = CatalogAcquirer::new();
    assets::resolve_assets_from(&root).unwrap();

    assert_eq!(probe.count(), 0);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_zero_byte_voice_is_reinstalled() {
    let (root, _) = install("zero-byte");
    let voice = &assets::VOICES[0];
    fs::write(root.join(voice.path), b"").unwrap();

    let again = CatalogAcquirer::new();
    assets::ensure_assets_into(&again, &root).unwrap();

    assert!(again.requested().contains(voice.url));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn concurrent_ensures_install_every_asset_exactly_once() {
    let root = Arc::new(root("concurrent"));
    let acquirer = CatalogAcquirer::new();

    let workers: Vec<_> = (0..4)
        .map(|_| {
            let root = Arc::clone(&root);
            let acquirer = acquirer.clone();
            std::thread::spawn(move || assets::ensure_assets_into(&acquirer, root.as_path()))
        })
        .collect();
    for worker in workers {
        worker.join().unwrap().unwrap();
    }

    assert_eq!(
        acquirer.count(),
        assets::CORE_ASSETS.len() + assets::VOICES.len()
    );
    fs::remove_dir_all(root.as_path()).unwrap();
}

#[test]
fn codec_external_data_stays_a_sibling_of_its_graphs() {
    let (root, _) = install("codec-siblings");
    let resolved = assets::resolve_assets_from(&root).unwrap();

    // ONNX resolves external data by sibling filename at session-commit time, so the shared data
    // file has to sit in the same directory as both codec graphs.
    assert_eq!(
        resolved.codec_shared_data.parent(),
        resolved.codec_decode_full.parent()
    );
    assert_eq!(
        resolved.codec_shared_data.parent(),
        resolved.codec_decode_step.parent()
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn resolving_reports_a_missing_asset_instead_of_downloading_it() {
    let root = root("missing");
    let acquirer = CatalogAcquirer::new();

    let failure = assets::resolve_assets_from(&root);

    assert!(matches!(failure, Err(AssetError::Missing(_))));
    assert_eq!(acquirer.count(), 0);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn assets_live_under_the_shared_models_directory() {
    assert_eq!(
        assets::model_dir(),
        Path::new("models").join("TTS").join("zerotts")
    );
}

#[test]
fn every_url_is_pinned_to_the_declared_upstream_revision() {
    assert_eq!(assets::MODEL_REVISION.len(), 40);
    assert!(
        assets::MODEL_REVISION
            .chars()
            .all(|c| c.is_ascii_hexdigit())
    );
    for asset in assets::CORE_ASSETS {
        assert!(
            asset.url.contains(assets::MODEL_REVISION),
            "asset `{}` is not pinned",
            asset.path
        );
    }
    for voice in assets::VOICES {
        assert!(
            voice.url.contains(assets::MODEL_REVISION),
            "voice `{}` is not pinned",
            voice.id
        );
    }
}
