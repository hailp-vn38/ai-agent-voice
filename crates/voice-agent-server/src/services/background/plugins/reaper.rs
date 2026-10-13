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
        Ok(())
    }
}
