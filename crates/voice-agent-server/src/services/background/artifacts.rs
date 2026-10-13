use crate::{database::Database, services::background::JobError};
use std::{
    fs::OpenOptions,
    io::{self, Read},
    path::PathBuf,
};

pub(crate) fn root(database: &Database) -> Result<PathBuf, JobError> {
    let mut path = database
        .durable_path()
        .ok_or(JobError::DatabaseUnavailable)?
        .as_os_str()
        .to_owned();
    path.push(".job-artifacts");
    Ok(PathBuf::from(path))
}
#[cfg(test)]
mod tests;

/// Verify the immutable artifact through one descriptor, then stream that same inode.
pub(crate) async fn open_checked(
    path: PathBuf,
    max_bytes: u64,
    expected_size: u64,
    expected_sha: String,
) -> io::Result<std::fs::File> {
    tokio::task::spawn_blocking(move || {
        use sha2::{Digest, Sha256};
        use std::io::{Seek, SeekFrom};
        #[cfg(unix)]
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        options.custom_flags(libc::O_NOFOLLOW);
        let mut file = options.open(path)?;
        let metadata = file.metadata()?;
        #[cfg(unix)]
        let single_link = metadata.nlink() == 1;
        #[cfg(not(unix))]
        let single_link = true;
        if !metadata.is_file()
            || !single_link
            || metadata.len() != expected_size
            || expected_size > max_bytes
        {
            return Err(io::Error::other("artifact_file_invalid"));
        }
        let mut hash = Sha256::new();
        let mut buffer = [0u8; 65536];
        let mut size = 0u64;
        loop {
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            size = size
                .checked_add(count as u64)
                .ok_or_else(|| io::Error::other("artifact_size_invalid"))?;
            if size > max_bytes {
                return Err(io::Error::other("artifact_size_invalid"));
            }
            hash.update(&buffer[..count]);
        }
        let actual: String = hash
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        if size != expected_size
            || actual != expected_sha
            || file.metadata()?.len() != expected_size
        {
            return Err(io::Error::other("artifact_hash_invalid"));
        }
        file.seek(SeekFrom::Start(0))?;
        Ok(file)
    })
    .await
    .map_err(|_| io::Error::other("artifact_open_task_failed"))?
}

/// Open the final component without following a link, then validate the same
/// open inode before reading. Callers also require a server-owned parent path.
pub(crate) async fn read_regular(path: PathBuf, max_bytes: u64) -> io::Result<Vec<u8>> {
    tokio::task::spawn_blocking(move || {
        #[cfg(unix)]
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        options.custom_flags(libc::O_NOFOLLOW);
        let file = options.open(path)?;
        let metadata = file.metadata()?;
        #[cfg(unix)]
        let single_link = metadata.nlink() == 1;
        #[cfg(not(unix))]
        let single_link = true;
        if !metadata.is_file() || !single_link || metadata.len() > max_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "artifact_file_invalid",
            ));
        }
        let mut bytes = Vec::new();
        file.take(max_bytes + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 != metadata.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "artifact_size_changed",
            ));
        }
        Ok(bytes)
    })
    .await
    .map_err(|_| io::Error::other("artifact_read_task_failed"))?
}

pub(crate) async fn sync_directory(path: PathBuf) -> io::Result<()> {
    tokio::task::spawn_blocking(move || std::fs::File::open(path)?.sync_all())
        .await
        .map_err(|_| io::Error::other("artifact_sync_task_failed"))?
}

/// After the owned-worker reaper, remove files whose filesystem rename had no
/// corresponding SQLite commit. The deployment lock excludes another writer.
pub(crate) async fn reconcile(database: &Database) -> Result<(), JobError> {
    let root = root(database)?;
    let mut jobs = match tokio::fs::read_dir(&root).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(JobError::DatabaseUnavailable),
    };
    let mut scanned = 0usize;
    while let Some(job) = jobs
        .next_entry()
        .await
        .map_err(|_| JobError::DatabaseUnavailable)?
    {
        let job_id = job.file_name().to_string_lossy().to_string();
        if uuid::Uuid::parse_str(&job_id).is_err()
            || !job
                .file_type()
                .await
                .map_err(|_| JobError::DatabaseUnavailable)?
                .is_dir()
        {
            return Err(JobError::DatabaseUnavailable);
        }
        let mut entries = tokio::fs::read_dir(job.path())
            .await
            .map_err(|_| JobError::DatabaseUnavailable)?;
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|_| JobError::DatabaseUnavailable)?
        {
            scanned += 1;
            if scanned > 10000 {
                return Err(JobError::CapacityUnavailable);
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let retained: i64 = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM job_artifacts WHERE job_id=? AND id=?)",
            )
            .bind(&job_id)
            .bind(&name)
            .fetch_one(database.pool())
            .await?;
            if retained == 0 {
                let kind = entry
                    .file_type()
                    .await
                    .map_err(|_| JobError::DatabaseUnavailable)?;
                if !kind.is_file() && !kind.is_symlink() {
                    return Err(JobError::DatabaseUnavailable);
                }
                tokio::fs::remove_file(entry.path())
                    .await
                    .map_err(|_| JobError::DatabaseUnavailable)?;
            }
        }
        // A directory may have become empty after an interrupted publication or
        // after retention deleted its SQLite rows. Keep the next scan bounded.
        let _ = tokio::fs::remove_dir(job.path()).await;
    }
    Ok(())
}

/// Retire bounded batches of terminal Jobs after 30 days. Retry descendants keep
/// their parent until they have themselves expired, preserving visible lineage.
/// Remove database references first; `reconcile` safely removes orphan files if
/// the process stops between the commit and filesystem cleanup.
pub(crate) async fn expire_terminal(database: &Database, now: i64) -> Result<u64, JobError> {
    let cutoff = now
        .checked_sub(30 * 24 * 60 * 60)
        .ok_or(JobError::ClockUnavailable)?;
    let mut tx = database.pool().begin_with("BEGIN IMMEDIATE").await?;
    let ids: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM jobs WHERE status IN ('succeeded','partial_success','failed','cancelled') AND updated_at<? AND NOT EXISTS (SELECT 1 FROM jobs AS child WHERE child.retry_of_job_id=jobs.id) ORDER BY updated_at,id LIMIT 100",
    )
    .bind(cutoff)
    .fetch_all(&mut *tx)
    .await?;
    for id in &ids {
        sqlx::query("DELETE FROM job_artifacts WHERE job_id=?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM job_steps WHERE job_id=?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM jobs WHERE id=?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    if !ids.is_empty() {
        reconcile(database).await?;
    }
    Ok(ids.len() as u64)
}
