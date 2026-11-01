//! Behaviour through the `MemoryCore` interface, on a real temporary directory and real SQLite.
//! Files are only read directly where the vault on disk is the contract (ADR 0119): a vault owner
//! opens these files, and `MEMORY.md` and `tombstones.json` are specified by name.

use super::apply::{FailAction, FailPoint};
use super::*;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

mod durability;
mod files;
mod permissions;
mod reads;
mod writes;

const DEVICE_A_DIR: &str = "dd5e8641af47e250"; // sha256("device-a")[..16], computed with shasum.

struct Fixture {
    root: PathBuf,
    core: MemoryCore,
    scope: MemoryScope,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn config(root: &Path) -> MemorySectionConfig {
    MemorySectionConfig {
        enabled: true,
        vault_dir: root.to_path_buf(),
        ..MemorySectionConfig::default()
    }
}

fn fixture_with(adjust: impl FnOnce(&mut MemorySectionConfig)) -> Fixture {
    let root = std::env::temp_dir().join(format!("memory-core-{}", uuid::Uuid::new_v4()));
    let mut cfg = config(&root);
    adjust(&mut cfg);
    Fixture {
        core: MemoryCore::open(&cfg).expect("the vault opens"),
        scope: MemoryScope::for_device("device-a", DeviceKind::Physical),
        root,
    }
}

fn fixture() -> Fixture {
    fixture_with(|_| {})
}

impl Fixture {
    /// A second core on the same vault: what a process restart sees.
    fn reopen(&self) -> MemoryCore {
        MemoryCore::open(&config(&self.root)).expect("the vault reopens")
    }

    fn device_dir(&self) -> PathBuf {
        self.root.join("devices").join(DEVICE_A_DIR)
    }

    fn fact_paths(&self) -> Vec<PathBuf> {
        let Ok(entries) = fs::read_dir(self.device_dir().join("facts")) else {
            return Vec::new();
        };
        let mut paths: Vec<PathBuf> = entries
            .map(|entry| entry.unwrap().path())
            .filter(|path| !path.file_name().unwrap().to_string_lossy().starts_with('.'))
            .collect();
        paths.sort();
        paths
    }

    fn the_fact_path(&self) -> PathBuf {
        let paths = self.fact_paths();
        assert_eq!(paths.len(), 1, "exactly one fact file: {paths:?}");
        paths.into_iter().next().unwrap()
    }

    fn summary_file(&self) -> String {
        fs::read_to_string(self.device_dir().join("MEMORY.md")).unwrap_or_default()
    }

    fn tombstones_file(&self) -> String {
        fs::read_to_string(self.device_dir().join("tombstones.json")).unwrap_or_default()
    }

    fn index_path(&self) -> PathBuf {
        self.device_dir().join(".index.sqlite")
    }

    async fn remember(&self, title: &str, body: &str) -> MemoryReceipt {
        remember_on(&self.core, &self.scope, title, body).await
    }

    async fn forget(&self, receipt: &MemoryReceipt) -> Result<MemoryReceipt, MemoryError> {
        self.core
            .apply(
                &self.scope,
                MemoryActor::Admin,
                MemoryChange::Forget {
                    id: receipt.fact_id.clone().unwrap(),
                    expected_revision: receipt.revision.unwrap(),
                },
            )
            .await
    }

    async fn recall(&self, text: &str) -> Vec<MemoryFact> {
        recall_on(&self.core, &self.scope, text).await
    }

    async fn context_rows(&self) -> String {
        context_text(&self.core, &self.scope).await
    }

    async fn only_fact(&self) -> MemoryFact {
        let overview = self
            .core
            .inspect(&self.scope, InspectFilter::default())
            .await
            .unwrap();
        assert_eq!(overview.facts.len(), 1);
        overview.facts.into_iter().next().unwrap()
    }

    fn arm(&self, point: FailPoint, action: FailAction) {
        *self.core.inner.fail_point.lock().unwrap() = Some((point, action));
    }
}

fn remember_change(title: &str, body: &str) -> MemoryChange {
    MemoryChange::Remember {
        title: title.into(),
        hook: format!("hook of {title}"),
        body: body.into(),
        kind: FactKind::Business,
        source_ref: None,
    }
}

async fn remember_on(
    core: &MemoryCore,
    scope: &MemoryScope,
    title: &str,
    body: &str,
) -> MemoryReceipt {
    core.apply(scope, MemoryActor::Admin, remember_change(title, body))
        .await
        .expect("remember succeeds")
}

async fn recall_on(core: &MemoryCore, scope: &MemoryScope, text: &str) -> Vec<MemoryFact> {
    core.recall(
        scope,
        RecallQuery {
            text: text.into(),
            limit: 10,
        },
    )
    .await
    .expect("recall succeeds")
    .items
}

async fn context_text(core: &MemoryCore, scope: &MemoryScope) -> String {
    core.context(scope).await.current().rows.join("\n")
}

fn correct_change(receipt: &MemoryReceipt, body: &str) -> MemoryChange {
    MemoryChange::Correct {
        id: receipt.fact_id.clone().unwrap(),
        expected_revision: receipt.revision.unwrap(),
        title: None,
        hook: None,
        body: body.into(),
    }
}

fn voice(mode: MemoryMode, intent: ExplicitIntent) -> MemoryActor {
    MemoryActor::VoiceUser {
        device_db_id: 7,
        mode,
        intent,
    }
}

async fn eventually(what: &str, mut condition: impl AsyncFnMut() -> bool) {
    for _ in 0..500 {
        if condition().await {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("never happened: {what}");
}

/// Reads frontmatter as the vault owner's tools would, to check a value without the module's parser.
fn frontmatter(path: &Path) -> serde_yaml::Mapping {
    let text = fs::read_to_string(path).unwrap();
    let yaml = text
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---\n"))
        .expect("the file has frontmatter")
        .0;
    serde_yaml::from_str(yaml).unwrap()
}

/// A directory without read permission cannot be opened for its `fsync`, yet files can still be
/// created, renamed and removed in it: a real failure of the last step of a durable write.
struct DirSyncDenied(PathBuf);

impl Drop for DirSyncDenied {
    fn drop(&mut self) {
        fs::set_permissions(&self.0, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

/// `None` when permissions do not bind, as under root, where the failure cannot be produced.
fn deny_dir_sync(dir: &Path) -> Option<DirSyncDenied> {
    fs::set_permissions(dir, fs::Permissions::from_mode(0o300)).unwrap();
    let denied = DirSyncDenied(dir.to_path_buf());
    fs::File::open(dir).is_err().then_some(denied)
}

fn mode_bits(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}
