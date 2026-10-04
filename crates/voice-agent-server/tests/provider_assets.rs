//! Behaviour of the provider-owned asset layer: reuse, atomic download, and download safety.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
};

use voice_agent_server::providers::assets::{
    Asset, AssetAcquirer, AssetError, download_atomic, ensure_asset, is_ready,
};

fn root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("voice-assets-{}-{}", name, uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    root
}

/// An acquirer that records every call and writes fixed bytes. Acquisition never touches the
/// network, so these tests assert *whether* a download happened, not what came back.
struct FixtureAcquirer {
    body: Vec<u8>,
    calls: Arc<AtomicUsize>,
}

impl FixtureAcquirer {
    fn new(body: &[u8]) -> (Self, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        (
            Self {
                body: body.to_vec(),
                calls: Arc::clone(&calls),
            },
            calls,
        )
    }
}

impl AssetAcquirer for FixtureAcquirer {
    fn acquire(&self, _url: &str, destination: &Path) -> Result<(), AssetError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        fs::write(destination, &self.body).map_err(AssetError::Io)
    }
}

/// Fails after writing a partial file, the way an interrupted transfer does.
struct InterruptedAcquirer;

impl AssetAcquirer for InterruptedAcquirer {
    fn acquire(&self, _url: &str, destination: &Path) -> Result<(), AssetError> {
        fs::write(destination, b"partial").map_err(AssetError::Io)?;
        Err(AssetError::Download("connection reset".into()))
    }
}

fn asset() -> Asset {
    Asset {
        path: "onnx/text_encoder.onnx",
        url: "https://example.invalid/text_encoder.onnx",
    }
}

fn write(root: &Path, relative: &str, bytes: &[u8]) -> PathBuf {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, bytes).unwrap();
    path
}

fn parts(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        for entry in fs::read_dir(&directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.to_string_lossy().ends_with(".part") {
                found.push(path);
            }
        }
    }
    found
}

#[test]
fn an_existing_asset_is_reused_without_downloading() {
    let root = root("reuse");
    let existing = write(&root, "onnx/text_encoder.onnx", b"already installed");
    let (acquirer, calls) = FixtureAcquirer::new(b"downloaded");

    let resolved = ensure_asset(&acquirer, &root, &asset()).unwrap();

    assert_eq!(resolved, existing);
    assert_eq!(fs::read(&existing).unwrap(), b"already installed");
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_missing_asset_is_downloaded_and_atomically_installed() {
    let root = root("download");
    let (acquirer, calls) = FixtureAcquirer::new(b"fresh bytes");

    let resolved = ensure_asset(&acquirer, &root, &asset()).unwrap();

    assert_eq!(resolved, root.join("onnx/text_encoder.onnx"));
    assert_eq!(fs::metadata(&resolved).unwrap().len(), 11);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    // The final path never appears until the download finished, so nothing observes a partial file.
    assert!(parts(&root).is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_zero_byte_asset_is_downloaded_again() {
    let root = root("zero-byte");
    let empty = write(&root, "onnx/text_encoder.onnx", b"");
    let (acquirer, calls) = FixtureAcquirer::new(b"repaired");

    let resolved = ensure_asset(&acquirer, &root, &asset()).unwrap();

    assert_eq!(resolved, empty);
    assert_eq!(fs::read(&empty).unwrap(), b"repaired");
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn an_interrupted_download_leaves_no_final_asset_and_no_temporary_file() {
    let root = root("interrupted");

    let failure = ensure_asset(&InterruptedAcquirer, &root, &asset());

    assert!(matches!(failure, Err(AssetError::Download(_))));
    assert!(!root.join("onnx/text_encoder.onnx").exists());
    assert!(parts(&root).is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_download_that_writes_nothing_is_rejected_and_leaves_no_final_asset() {
    let root = root("empty-download");
    let (acquirer, _) = FixtureAcquirer::new(b"");

    let failure = ensure_asset(&acquirer, &root, &asset());

    assert!(matches!(failure, Err(AssetError::InvalidDownload(_))));
    assert!(!root.join("onnx/text_encoder.onnx").exists());
    assert!(parts(&root).is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn concurrent_ensures_download_a_missing_asset_only_once() {
    let root = Arc::new(root("concurrent"));
    let (acquirer, calls) = FixtureAcquirer::new(b"shared bytes");

    let workers: Vec<_> = (0..8)
        .map(|_| {
            let root = Arc::clone(&root);
            let acquirer = FixtureAcquirer {
                body: acquirer.body.clone(),
                calls: Arc::clone(&calls),
            };
            thread::spawn(move || ensure_asset(&acquirer, root.as_path(), &asset()).unwrap())
        })
        .collect();
    let resolved: Vec<PathBuf> = workers.into_iter().map(|w| w.join().unwrap()).collect();

    // Every caller observes the same installed path, and only one of them performed the transfer.
    assert!(
        resolved
            .iter()
            .all(|path| path == &root.join("onnx/text_encoder.onnx"))
    );
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    fs::remove_dir_all(root.as_path()).unwrap();
}

#[test]
fn download_atomic_always_terminates_at_the_declared_destination() {
    let root = root("destination");
    let (acquirer, _) = FixtureAcquirer::new(b"payload");
    let assets = [Asset {
        path: "voices/index.json",
        url: "https://example.invalid/voices/index.json",
    }];

    let resolved = download_atomic(&acquirer, &root, &assets).unwrap();

    assert_eq!(
        resolved,
        BTreeMap::from([(
            "voices/index.json".to_owned(),
            root.join("voices/index.json"),
        )])
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn readiness_depends_only_on_a_non_empty_regular_file() {
    let root = root("ready");
    let populated = write(&root, "model.onnx", b"x");
    let empty = write(&root, "empty.onnx", b"");

    assert!(is_ready(&populated));
    assert!(!is_ready(&empty));
    assert!(!is_ready(&root));
    assert!(!is_ready(&root.join("absent.onnx")));
    fs::remove_dir_all(root).unwrap();
}
