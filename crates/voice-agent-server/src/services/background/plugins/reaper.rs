use super::*;

impl PluginStore {
    /// Startup reconciliation never touches containers without our owner label.
    pub(crate) async fn reap_owned(&self) -> Result<(), PluginError> {
        if !cfg!(target_os = "linux") || self.config.plugin_docker_host.is_none() {
            return Ok(());
        }
        let mut info = builder::command(&self.config)?;
        info.args(["info", "--format", "{{.OSType}} {{json .SecurityOptions}}"]);
        let observed = tokio::time::timeout(std::time::Duration::from_secs(5), info.output())
            .await
            .map_err(|_| PluginError::SandboxUnavailable)?
            .map_err(|_| PluginError::SandboxUnavailable)?;
        if !observed.status.success()
            || !String::from_utf8_lossy(&observed.stdout).contains("name=rootless")
        {
            return Err(PluginError::SandboxUnavailable);
        }
        let mut list = builder::command(&self.config)?;
        list.args([
            "ps",
            "--all",
            "--quiet",
            "--filter",
            "label=agent.job.owner=voice-agent-server",
        ]);
        let listed = tokio::time::timeout(std::time::Duration::from_secs(5), list.output())
            .await
            .map_err(|_| PluginError::SandboxUnavailable)?
            .map_err(|_| PluginError::SandboxUnavailable)?;
        if !listed.status.success() || listed.stdout.len() > 8192 {
            return Err(PluginError::SandboxUnavailable);
        }
        for id in String::from_utf8(listed.stdout)
            .map_err(|_| PluginError::SandboxUnavailable)?
            .lines()
        {
            if !(12..=64).contains(&id.len()) || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(PluginError::SandboxUnavailable);
            }
            let mut remove = builder::command(&self.config)?;
            remove.args(["rm", "--force", id]);
            let status = tokio::time::timeout(std::time::Duration::from_secs(15), remove.status())
                .await
                .map_err(|_| PluginError::SandboxUnavailable)?
                .map_err(|_| PluginError::SandboxUnavailable)?;
            if !status.success() {
                return Err(PluginError::SandboxUnavailable);
            }
        }
        sqlx::query("UPDATE job_steps SET worker_instance_id=NULL WHERE worker_instance_id IS NOT NULL AND job_id IN (SELECT id FROM jobs WHERE status IN ('failed','cancelled','succeeded','partial_success'))").execute(self.database.pool()).await?;
        self.drain_workspaces().await
    }

    pub(super) async fn drain_workspaces(&self) -> Result<(), PluginError> {
        while self.clean_workspace_batch().await? == 1000 {
            tokio::task::yield_now().await;
        }
        Ok(())
    }

    /// The deployment lock and completed owned-worker reaping make every prior
    /// process workspace disposable. Reject unexpected entries rather than
    /// following attacker-created links outside the owned directory.
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
