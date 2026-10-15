mod archive;
mod builder;
mod contracts;
mod dispatch;
#[allow(dead_code)]
mod protocol;
pub mod qualification;
mod reaper;
#[allow(dead_code)]
mod runner;
#[cfg(test)]
mod tests;
pub(crate) use contracts::{compile_child_tools, normalize_tool_arguments};
pub(crate) use dispatch::{FrozenChoice, Invocation, InvokeResult};

/// Decode either a previously persisted External choice or a frozen Core choice.
/// External JSON is unchanged; Core never receives a Plugin Store installation.
pub(crate) fn decode_choice(raw: &serde_json::Value) -> Result<FrozenChoice, PluginError> {
    if raw["kind"] == "core" {
        crate::tools::core::decode(raw)
    } else if raw.get("kind").is_none() {
        serde_json::from_value(raw.clone()).map_err(|_| PluginError::PermissionRequired)
    } else {
        Err(PluginError::PermissionRequired)
    }
}

use crate::database::plugins::Installation;
use crate::{config::BackgroundConfig, database::Database};
use archive::{Manifest, Package};
use serde::Serialize;
use serde_json::json;
use std::{path::PathBuf, sync::Arc};
use tokio::sync::Semaphore;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginError {
    InvalidPackage,
    SandboxUnavailable,
    ImageBuildFailed,
    BuildTimeout,
    PermissionRequired,
    RevisionConflict,
    InUse,
    NotFound,
    DatabaseUnavailable,
    StorageUnavailable,
    WorkerProtocolError,
    PluginRuntimeError,
    ApplicationError,
    WorkerTimeout,
    WorkerDeadlineExceeded,
    WorkerCapacityUnavailable,
    WorkerUnreaped,
    JobRejected(crate::services::background::JobError),
}
impl PluginError {
    pub fn code(self) -> &'static str {
        match self {
            Self::InvalidPackage => "invalid_plugin_package",
            Self::SandboxUnavailable => "sandbox_unavailable",
            Self::ImageBuildFailed => "image_build_failed",
            Self::BuildTimeout => "build_timeout",
            Self::PermissionRequired => "permission_required",
            Self::RevisionConflict => "revision_conflict",
            Self::InUse => "plugin_in_use",
            Self::NotFound => "not_found",
            Self::DatabaseUnavailable => "database_unavailable",
            Self::StorageUnavailable => "plugin_storage_unavailable",
            Self::WorkerProtocolError => "worker_protocol_error",
            Self::PluginRuntimeError => "plugin_runtime_error",
            Self::ApplicationError => "plugin_application_error",
            Self::WorkerTimeout => "worker_timeout",
            Self::WorkerDeadlineExceeded => "deadline_exceeded",
            Self::WorkerCapacityUnavailable => "background_capacity_unavailable",
            Self::WorkerUnreaped => "worker_unreaped",
            Self::JobRejected(error) => error.code(),
        }
    }
}
impl From<sqlx::Error> for PluginError {
    fn from(_: sqlx::Error) -> Self {
        Self::DatabaseUnavailable
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct PluginView {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub digest: String,
    pub installation_digest: String,
    pub status: String,
    pub revision: i64,
    pub runtime: RuntimeView,
    pub operations: Vec<OperationView>,
    pub last_error_code: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
pub struct RuntimeView {
    pub kind: &'static str,
    pub language: &'static str,
}
#[derive(Clone, Debug, Serialize)]
pub struct OperationView {
    pub name: String,
    pub description: String,
    pub replay: String,
}
impl Installation {
    fn manifest(&self) -> Result<Manifest, PluginError> {
        serde_json::from_str(&self.operations_json).map_err(|_| PluginError::DatabaseUnavailable)
    }
    fn view(self) -> Result<PluginView, PluginError> {
        let manifest = self.manifest()?;
        Ok(PluginView {
            id: self.plugin_id,
            name: manifest.plugin.name,
            description: manifest.plugin.description,
            version: self.version,
            digest: self.package_digest,
            installation_digest: self.installation_digest,
            status: self.status,
            revision: self.revision,
            runtime: RuntimeView {
                kind: "shared_process",
                language: if manifest.runtime.kind == "rust" {
                    "rust"
                } else {
                    "python"
                },
            },
            operations: manifest
                .operations
                .into_iter()
                .map(|(name, operation)| OperationView {
                    name,
                    description: operation.description,
                    replay: operation.replay,
                })
                .collect(),
            last_error_code: self.last_error_code,
        })
    }
}

pub struct PluginStore {
    database: Arc<Database>,
    config: BackgroundConfig,
    root: PathBuf,
    build_slot: Semaphore,
    #[allow(dead_code)]
    worker_slots: Semaphore,
}

impl PluginStore {
    /// Shared bounded capacity for Core and External work, without any runner preflight.
    pub(crate) async fn acquire_core_slot(
        &self,
        deadline_at: i64,
    ) -> Result<tokio::sync::SemaphorePermit<'_>, PluginError> {
        dispatch::acquire_worker_until(&self.worker_slots, deadline_at).await
    }

    pub fn new(database: Arc<Database>, config: BackgroundConfig) -> Self {
        let worker_limit = config.limits.max_container_workers as usize;
        let root = database
            .durable_path()
            .and_then(std::path::Path::parent)
            .unwrap_or_else(|| std::path::Path::new("data"))
            .join("plugins");
        Self {
            database,
            config,
            root,
            build_slot: Semaphore::new(1),
            worker_slots: Semaphore::new(worker_limit),
        }
    }
    async fn row(&self, id: &str) -> Result<Installation, PluginError> {
        self.database.plugin_installation(id).await
    }
    pub async fn get(&self, id: &str) -> Result<PluginView, PluginError> {
        self.row(id).await?.view()
    }
    pub async fn list(
        &self,
        page: u32,
        size: u32,
        status: Option<&str>,
        q: Option<&str>,
    ) -> Result<(Vec<PluginView>, i64), PluginError> {
        if page == 0
            || !(1..=100).contains(&size)
            || q.is_some_and(|q| q.len() > 128)
            || status.is_some_and(|s| !matches!(s, "ready" | "disabled" | "error"))
        {
            return Err(PluginError::InvalidPackage);
        }
        let pattern = q.map(|s| {
            format!(
                "%{}%",
                s.replace('\\', "\\\\")
                    .replace('%', "\\%")
                    .replace('_', "\\_")
            )
        });
        let (rows, total) = self
            .database
            .list_plugin_installations(page, size, status, pattern.as_deref())
            .await?;
        Ok((
            rows.into_iter()
                .map(Installation::view)
                .collect::<Result<_, _>>()?,
            total,
        ))
    }
    pub async fn install(&self, bytes: &[u8]) -> Result<PluginView, PluginError> {
        let package = archive::validate(bytes)?;
        if package.manifest.plugin.id == "core" {
            return Err(PluginError::InvalidPackage);
        }
        let _permit = self
            .build_slot
            .try_acquire()
            .map_err(|_| PluginError::SandboxUnavailable)?;
        builder::preflight(&self.config, &package.manifest.runtime).await?;
        let stage = self
            .root
            .join("staging")
            .join(uuid::Uuid::new_v4().to_string());
        tokio::fs::create_dir_all(&stage)
            .await
            .map_err(|_| PluginError::StorageUnavailable)?;
        let result = self.install_staged(&package, &stage).await;
        let _ = tokio::fs::remove_dir_all(&stage).await;
        result
    }
    pub async fn install_curated(&self, id: &str) -> Result<PluginView, PluginError> {
        if id.is_empty()
            || id.len() > 64
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        {
            return Err(PluginError::InvalidPackage);
        }
        let directory = self
            .config
            .plugin_curated_dir
            .as_deref()
            .ok_or(PluginError::SandboxUnavailable)?;
        let path = directory.join(format!("{id}.zip"));
        let metadata = tokio::fs::symlink_metadata(&path)
            .await
            .map_err(|_| PluginError::NotFound)?;
        if !metadata.file_type().is_file() || metadata.len() > 20 * 1024 * 1024 {
            return Err(PluginError::InvalidPackage);
        }
        let bytes = tokio::fs::read(path)
            .await
            .map_err(|_| PluginError::StorageUnavailable)?;
        self.install(&bytes).await
    }
    async fn install_staged(
        &self,
        package: &Package,
        stage: &std::path::Path,
    ) -> Result<PluginView, PluginError> {
        for (name, content) in &package.files {
            let path = stage.join(name);
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|_| PluginError::StorageUnavailable)?;
            }
            tokio::fs::write(path, content)
                .await
                .map_err(|_| PluginError::StorageUnavailable)?;
        }
        let installation = builder::build(&self.config, stage, package).await?;
        tokio::fs::write(stage.join("package.zip"), &package.source_zip)
            .await
            .map_err(|_| PluginError::StorageUnavailable)?;
        let id = &package.manifest.plugin.id;
        let target = self
            .root
            .join("packages")
            .join(id)
            .join(&package.manifest.plugin.version)
            .join(&package.digest);
        let publish_package = !target.exists();
        if !publish_package {
            let old = tokio::fs::read(target.join("package.zip"))
                .await
                .map_err(|_| PluginError::StorageUnavailable)?;
            if archive::validate(&old)?.digest != package.digest {
                return Err(PluginError::InvalidPackage);
            }
        }
        let manifest =
            serde_json::to_string(&package.manifest).map_err(|_| PluginError::InvalidPackage)?;
        if manifest.len() > 65536 {
            return Err(PluginError::InvalidPackage);
        }
        self.database
            .publish_plugin_installation(
                id,
                &package.manifest.plugin.version,
                &package.digest,
                &installation,
                &manifest,
                || async {
                    if publish_package {
                        if let Some(parent) = target.parent() {
                            tokio::fs::create_dir_all(parent)
                                .await
                                .map_err(|_| PluginError::StorageUnavailable)?;
                        }
                        // Staging is mounted from the Installer volume; it may be on a
                        // different filesystem from packages. Publish through a sibling
                        // directory so the final rename is atomic on the package volume.
                        let pending = self
                            .root
                            .join("publishing")
                            .join(uuid::Uuid::new_v4().to_string());
                        for (name, content) in &package.files {
                            let path = pending.join(name);
                            if let Some(parent) = path.parent() {
                                tokio::fs::create_dir_all(parent)
                                    .await
                                    .map_err(|_| PluginError::StorageUnavailable)?;
                            }
                            tokio::fs::write(path, content)
                                .await
                                .map_err(|_| PluginError::StorageUnavailable)?;
                        }
                        tokio::fs::write(pending.join("package.zip"), &package.source_zip)
                            .await
                            .map_err(|_| PluginError::StorageUnavailable)?;
                        tokio::fs::rename(&pending, &target)
                            .await
                            .map_err(|_| PluginError::StorageUnavailable)?;
                    }
                    Ok(())
                },
            )
            .await?
            .view()
    }
    pub async fn activate(&self, id: &str, expected: i64) -> Result<PluginView, PluginError> {
        let row = self.row(id).await?;
        if row.revision != expected {
            return Err(PluginError::RevisionConflict);
        }
        let manifest = row.manifest()?;
        builder::verify_image(
            &self.config,
            &row.installation_digest,
            &row.package_digest,
            &manifest.runtime,
        )
        .await?;
        let package = self
            .root
            .join("packages")
            .join(&row.plugin_id)
            .join(&row.version)
            .join(&row.package_digest);
        let source = tokio::fs::read(package.join("package.zip"))
            .await
            .map_err(|_| PluginError::StorageUnavailable)?;
        let checked = archive::validate(&source)?;
        if checked.digest != row.package_digest || checked.manifest.plugin.id != row.plugin_id {
            return Err(PluginError::InvalidPackage);
        }
        self.database
            .activate_plugin_installation(id, expected)
            .await?
            .view()
    }
    pub async fn disable(&self, id: &str, expected: i64) -> Result<PluginView, PluginError> {
        self.database
            .disable_plugin_installation(id, expected)
            .await?
            .view()
    }
    pub async fn uninstall(&self, id: &str, expected: i64) -> Result<(), PluginError> {
        self.database.uninstall_plugin(id, expected).await
    }
    pub async fn test(&self, id: &str, expected: i64) -> Result<(), PluginError> {
        let row = self.row(id).await?;
        if row.revision != expected {
            return Err(PluginError::RevisionConflict);
        }
        let manifest = row.manifest()?;
        builder::verify_image(
            &self.config,
            &row.installation_digest,
            &row.package_digest,
            &manifest.runtime,
        )
        .await
    }

    /// Package-attributable failures stop later dispatches even when the Job
    /// concurrently becomes terminal. The frozen installation identity prevents a
    /// late worker from poisoning an upgraded installation.
    pub(crate) async fn fail_package(
        &self,
        choice: &FrozenChoice,
        code: &str,
    ) -> Result<(), PluginError> {
        self.database.fail_plugin_installation(choice, code).await
    }
}

/// Freeze ready installation rows read by the Job submission write transaction.
/// The database retains that transaction until the Job is committed.
pub(crate) fn frozen_choices(
    rows: Vec<Installation>,
    request: &crate::services::background::JobRequest,
    enabled_core_groups: &[String],
) -> Result<String, crate::services::background::JobError> {
    use crate::services::background::JobError;
    if rows.len() > 32 {
        return Err(JobError::CapacityUnavailable);
    }
    let mut choices: Vec<_> = crate::tools::core::frozen_choices()
        .into_iter()
        .filter(|choice| {
            crate::tools::core::group_for(choice["operation"].as_str().unwrap_or(""))
                .is_some_and(|group| enabled_core_groups.iter().any(|enabled| enabled == group))
        })
        .collect();
    for row in rows {
        // Core is a reserved namespace, never an External Store installation.
        if row.plugin_id == "core" {
            return Err(JobError::CapabilityUnavailable);
        }
        let manifest = row.manifest().map_err(|_| JobError::DatabaseUnavailable)?;
        for (name, operation) in manifest.operations {
            if operation.replay != "safe_read" {
                continue;
            }
            choices.push(json!({
                "plugin_id": row.plugin_id, "version": row.version,
                "package_digest": row.package_digest, "installation_digest": row.installation_digest,
                "operation": name,
                "provides": operation.provides, "sources": operation.sources,
            }));
            if choices.len() > 64 {
                return Err(JobError::CapacityUnavailable);
            }
        }
    }
    let covered = |field: &str, requirement: &str| {
        choices.iter().any(|choice| {
            choice[field]
                .as_array()
                .is_some_and(|values| values.iter().any(|value| value == requirement))
        })
    };
    if request
        .required_capabilities
        .iter()
        .any(|v| !covered("provides", v))
        || request
            .required_sources
            .iter()
            .any(|v| !covered("sources", v))
    {
        return Err(JobError::CapabilityUnavailable);
    }
    serde_json::to_string(&choices).map_err(|_| JobError::DatabaseUnavailable)
}
