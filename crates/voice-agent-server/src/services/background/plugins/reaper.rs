use super::*;

impl PluginStore {
    /// The Runner owns process groups. A server restart must never blindly reap
    /// them or delete output from an attempt that could still be running.
    pub(crate) async fn reap_owned(&self) -> Result<(), PluginError> {
        self.database.clear_terminal_plugin_workers().await
    }

    #[cfg(test)]
    pub(super) async fn drain_workspaces(&self) -> Result<(), PluginError> {
        while self.clean_workspace_batch().await? == 1000 {
            tokio::task::yield_now().await;
        }
        Ok(())
    }

    /// The deployment lock and completed owned-worker reaping make every prior
    /// process workspace disposable. Reject unexpected entries rather than
    /// following attacker-created links outside the owned directory.
    #[cfg(test)]
    async fn clean_workspace_batch(&self) -> Result<usize, PluginError> {
        let root = self.root.join("workspaces");
        let mut jobs = match tokio::fs::read_dir(root).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(_) => return Err(PluginError::StorageUnavailable),
        };
        let mut count = 0usize;
        while let Some(job) = jobs
            .next_entry()
            .await
            .map_err(|_| PluginError::StorageUnavailable)?
        {
            let name = job.file_name().to_string_lossy().to_string();
            if uuid::Uuid::parse_str(&name).is_err()
                || !job
                    .file_type()
                    .await
                    .map_err(|_| PluginError::StorageUnavailable)?
                    .is_dir()
            {
                return Err(PluginError::StorageUnavailable);
            }
            let mut attempts = tokio::fs::read_dir(job.path())
                .await
                .map_err(|_| PluginError::StorageUnavailable)?;
            while let Some(attempt) = attempts
                .next_entry()
                .await
                .map_err(|_| PluginError::StorageUnavailable)?
            {
                count += 1;
                let name = attempt.file_name().to_string_lossy().to_string();
                if uuid::Uuid::parse_str(&name).is_err()
                    || !attempt
                        .file_type()
                        .await
                        .map_err(|_| PluginError::StorageUnavailable)?
                        .is_dir()
                {
                    return Err(PluginError::StorageUnavailable);
                }
                tokio::fs::remove_dir_all(attempt.path())
                    .await
                    .map_err(|_| PluginError::StorageUnavailable)?;
                if count == 1000 {
                    break;
                }
            }
            if count == 1000 {
                break;
            }
            tokio::fs::remove_dir(job.path())
                .await
                .map_err(|_| PluginError::StorageUnavailable)?;
        }
        Ok(count)
    }
}
