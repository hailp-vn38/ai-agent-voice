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
pub(crate) use dispatch::{FrozenChoice, Invocation};

use crate::{config::BackgroundConfig, database::Database};
use archive::{Manifest, Package};
use serde::Serialize;
use serde_json::json;
use sqlx::FromRow;
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
#[derive(FromRow)]
struct Installation {
    plugin_id: String,
    version: String,
    package_digest: String,
    installation_digest: String,
    status: String,
    revision: i64,
    operations_json: String,
    last_error_code: Option<String>,
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
        sqlx::query_as("SELECT * FROM plugin_installations WHERE plugin_id=?")
            .bind(id)
            .fetch_optional(self.database.pool())
            .await?
            .ok_or(PluginError::NotFound)
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
        let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM plugin_installations WHERE (? IS NULL OR status=?) AND (? IS NULL OR plugin_id LIKE ? ESCAPE '\\')")
            .bind(status).bind(status).bind(&pattern).bind(&pattern).fetch_one(self.database.pool()).await?;
        let rows: Vec<Installation> = sqlx::query_as("SELECT * FROM plugin_installations WHERE (? IS NULL OR status=?) AND (? IS NULL OR plugin_id LIKE ? ESCAPE '\\') ORDER BY plugin_id LIMIT ? OFFSET ?")
            .bind(status).bind(status).bind(&pattern).bind(&pattern).bind(size).bind(i64::from(page-1)*i64::from(size)).fetch_all(self.database.pool()).await?;
        Ok((
            rows.into_iter()
                .map(Installation::view)
                .collect::<Result<_, _>>()?,
            total,
        ))
    }
    pub async fn install(&self, bytes: &[u8]) -> Result<PluginView, PluginError> {
        let package = archive::validate(bytes)?;
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
        let installation = builder::build(&self.config, stage, &package).await?;
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
        let _publication = self.database.tool_security.publication.write().await;
        let mut tx = self.database.pool().begin_with("BEGIN IMMEDIATE").await?;
        let pinned: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM jobs WHERE status IN ('queued','planning','running','cancelling') AND EXISTS (SELECT 1 FROM json_each(jobs.plugin_choices_json) WHERE json_extract(value,'$.plugin_id')=?)")
            .bind(id).fetch_one(&mut *tx).await?;
        if pinned != 0 {
            return Err(PluginError::InUse);
        }
        if publish_package {
            if let Some(parent) = target.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|_| PluginError::StorageUnavailable)?;
            }
            // Staging is mounted from the Installer volume; it may be on a
            // different filesystem from packages. Publish through a sibling
            // directory so the final rename is atomic on the package volume.
            let pending = self.root.join("publishing").join(uuid::Uuid::new_v4().to_string());
            for (name, content) in &package.files {
                let path = pending.join(name);
                if let Some(parent) = path.parent() {
                    tokio::fs::create_dir_all(parent).await.map_err(|_| PluginError::StorageUnavailable)?;
                }
                tokio::fs::write(path, content).await.map_err(|_| PluginError::StorageUnavailable)?;
            }
            tokio::fs::write(pending.join("package.zip"), &package.source_zip)
                .await.map_err(|_| PluginError::StorageUnavailable)?;
            tokio::fs::rename(&pending, &target)
                .await.map_err(|_| PluginError::StorageUnavailable)?;
        }
        let now = crate::database::unix_seconds().ok_or(PluginError::DatabaseUnavailable)?;
        sqlx::query("INSERT INTO plugin_installations(plugin_id,version,package_digest,installation_digest,status,revision,operations_json,created_at,updated_at) VALUES(?,?,?,?,'disabled',1,?,?,?) ON CONFLICT(plugin_id) DO UPDATE SET version=excluded.version,package_digest=excluded.package_digest,installation_digest=excluded.installation_digest,status='disabled',revision=plugin_installations.revision+1,operations_json=excluded.operations_json,last_error_code=NULL,updated_at=excluded.updated_at")
            .bind(id).bind(&package.manifest.plugin.version).bind(&package.digest).bind(&installation).bind(manifest).bind(now).bind(now).execute(&mut *tx).await?;
        tx.commit().await?;
        self.get(id).await
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
        let _publication = self.database.tool_security.publication.write().await;
        let result = sqlx::query("UPDATE plugin_installations SET status='ready',revision=revision+1,last_error_code=NULL,updated_at=? WHERE plugin_id=? AND revision=? AND status IN ('disabled','error')")
            .bind(crate::database::unix_seconds()).bind(id).bind(expected).execute(self.database.pool()).await?;
        if result.rows_affected() != 1 {
            return Err(PluginError::RevisionConflict);
        }
        self.get(id).await
    }
    pub async fn disable(&self, id: &str, expected: i64) -> Result<PluginView, PluginError> {
        let _publication = self.database.tool_security.publication.write().await;
        let result = sqlx::query("UPDATE plugin_installations SET status='disabled',revision=revision+1,updated_at=? WHERE plugin_id=? AND revision=?")
            .bind(crate::database::unix_seconds()).bind(id).bind(expected).execute(self.database.pool()).await?;
        if result.rows_affected() != 1 {
            return Err(PluginError::RevisionConflict);
        }
        self.get(id).await
    }
    pub async fn uninstall(&self, id: &str, expected: i64) -> Result<(), PluginError> {
        let _publication = self.database.tool_security.publication.write().await;
        let mut tx = self.database.pool().begin_with("BEGIN IMMEDIATE").await?;
        let row: Option<Installation> =
            sqlx::query_as("SELECT * FROM plugin_installations WHERE plugin_id=?")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?;
        let row = row.ok_or(PluginError::NotFound)?;
        if row.revision != expected {
            return Err(PluginError::RevisionConflict);
        }
        let pinned: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM jobs WHERE status IN ('queued','planning','running','cancelling') AND EXISTS (SELECT 1 FROM json_each(jobs.plugin_choices_json) WHERE json_extract(value,'$.plugin_id')=?)")
            .bind(id).fetch_one(&mut *tx).await?;
        if pinned != 0 {
            return Err(PluginError::InUse);
        }
        sqlx::query("DELETE FROM plugin_installations WHERE plugin_id=?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
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
        let _publication = self.database.tool_security.publication.write().await;
        sqlx::query("UPDATE plugin_installations SET status='error',last_error_code=?,revision=revision+1,updated_at=? WHERE plugin_id=? AND version=? AND package_digest=? AND installation_digest=? AND status='ready'")
            .bind(code).bind(crate::database::unix_seconds()).bind(&choice.plugin_id)
            .bind(&choice.version).bind(&choice.package_digest).bind(&choice.installation_digest)
            .execute(self.database.pool()).await?;
        Ok(())
    }
}

/// Called within the same BEGIN IMMEDIATE transaction that commits the Job.
/// Ready plugin choices are frozen before installation versions can change.
pub(crate) async fn frozen_choices(
    connection: &mut sqlx::SqliteConnection,
    request: &crate::services::background::JobRequest,
) -> Result<String, crate::services::background::JobError> {
    use crate::services::background::JobError;
    let rows: Vec<Installation> = sqlx::query_as(
        "SELECT * FROM plugin_installations WHERE status='ready' ORDER BY plugin_id LIMIT 33",
    )
    .fetch_all(connection)
    .await?;
    if rows.len() > 32 {
        return Err(JobError::CapacityUnavailable);
    }
    let mut choices = Vec::new();
    for row in rows {
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
