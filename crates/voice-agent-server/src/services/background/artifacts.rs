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
            if !database.job_artifact_exists(&job_id, &name).await? {
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

/// Remove only the deleted Job's directory, never another Job's in-flight files.
pub(crate) async fn remove_deleted_job(database: &Database, id: &str) -> Result<(), JobError> {
    if uuid::Uuid::parse_str(id).is_err() {
        return Err(JobError::InvalidArguments);
    }
    match tokio::fs::remove_dir_all(root(database)?.join(id)).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(JobError::DatabaseUnavailable),
    }
}

/// Retire bounded batches of terminal Jobs after 30 days. Retry parents remain
/// until their descendants have expired. Startup repairs orphan filesystem data.
pub(crate) async fn expire_terminal(database: &Database, now: i64) -> Result<u64, JobError> {
    let cutoff = now
        .checked_sub(30 * 24 * 60 * 60)
        .ok_or(JobError::ClockUnavailable)?;
    let ids = database.expire_terminal_jobs(cutoff).await?;
    for id in &ids {
        if let Err(error) = remove_deleted_job(database, id).await {
            tracing::warn!(event="background_retention_artifact_cleanup_failed",job_id=%id,code=%error.code());
        }
    }
    Ok(ids.len() as u64)
}
