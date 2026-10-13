use super::{PluginError, builder, protocol::MAX_STDOUT};
use crate::config::BackgroundConfig;
use std::sync::atomic::{AtomicBool, Ordering};
use std::{path::Path, process::Stdio, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub(super) struct Prepared {
    name: String,
    config: BackgroundConfig,
    reaped: AtomicBool,
}

impl Prepared {
    pub(super) async fn create(
        config: &BackgroundConfig,
        image: &str,
        workspace: &Path,
        job: &str,
        attempt: &str,
    ) -> Result<Self, PluginError> {
        let name = format!("agent-job-{attempt}");
        let mount = format!(
            "type=bind,source={},target=/work/out",
            workspace.join("out").display()
        );
        let mut command = builder::command(config)?;
        command.args([
            "create",
            "--interactive",
            "--name",
            &name,
            "--pull=never",
            "--read-only",
            "--cap-drop=ALL",
            "--security-opt=no-new-privileges",
            "--pids-limit=64",
            "--memory=512m",
            "--cpus=1",
            "--user=65532:65532",
            "--ulimit",
            "fsize=20971520:20971520",
            "--mount",
            &mount,
            "--label",
            "agent.job.owner=voice-agent-server",
            "--label",
            &format!("agent.job.id={job}"),
            "--label",
            &format!("agent.job.attempt={attempt}"),
        ]);
        command.arg(image);
        let result = tokio::time::timeout(Duration::from_secs(15), command.output())
            .await
            .map_err(|_| PluginError::SandboxUnavailable)?
            .map_err(|_| PluginError::SandboxUnavailable)?;
        if !result.status.success() || result.stdout.len() > 256 {
            return Err(PluginError::SandboxUnavailable);
        }
        Ok(Self {
            name,
            config: config.clone(),
            reaped: AtomicBool::new(false),
        })
    }

    pub(super) async fn start_container(
        &self,
        config: &BackgroundConfig,
    ) -> Result<(), PluginError> {
        let mut command = builder::command(config)?;
        command.args(["start", &self.name]);
        let output = tokio::time::timeout(Duration::from_secs(15), command.output())
            .await
            .map_err(|_| PluginError::SandboxUnavailable)?
            .map_err(|_| PluginError::SandboxUnavailable)?;
        if output.status.success() && output.stdout == format!("{}\n", self.name).as_bytes() {
            Ok(())
        } else {
            Err(PluginError::SandboxUnavailable)
        }
    }

    pub(super) async fn attach(
        &self,
        config: &BackgroundConfig,
        request: &[u8],
        timeout: Duration,
    ) -> Result<Vec<u8>, PluginError> {
        let mut command = builder::command(config)?;
        command.args(["attach", "--sig-proxy=false", &self.name]);
        command.stdin(Stdio::piped());
        command.stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|_| PluginError::SandboxUnavailable)?;
        let mut stdin = child.stdin.take().ok_or(PluginError::SandboxUnavailable)?;
        let stdout = child.stdout.take().ok_or(PluginError::SandboxUnavailable)?;
        let stderr = child.stderr.take().ok_or(PluginError::SandboxUnavailable)?;
        let output = async {
            stdin
                .write_all(request)
                .await
                .map_err(|_| PluginError::SandboxUnavailable)?;
            drop(stdin);
            let read_output = async {
                let mut output = Vec::new();
                stdout
                    .take((MAX_STDOUT + 1) as u64)
                    .read_to_end(&mut output)
                    .await
                    .map_err(|_| PluginError::SandboxUnavailable)?;
                if output.len() > MAX_STDOUT {
                    return Err(PluginError::WorkerProtocolError);
                }
                Ok(output)
            };
            let read_diagnostics = async {
                let mut diagnostics = Vec::new();
                stderr
                    .take(4097)
                    .read_to_end(&mut diagnostics)
                    .await
                    .map_err(|_| PluginError::SandboxUnavailable)?;
                if diagnostics.len() > 4096 {
                    return Err(PluginError::WorkerProtocolError);
                }
                Ok(())
            };
            let wait = async {
                child
                    .wait()
                    .await
                    .map_err(|_| PluginError::SandboxUnavailable)
            };
            let (output, (), status) = tokio::try_join!(read_output, read_diagnostics, wait)?;
            if !status.success() {
                return Err(PluginError::PluginRuntimeError);
            }
            Ok(output)
        };
        match tokio::time::timeout(timeout, output).await {
            Ok(result) => result,
            Err(_) => Err(PluginError::WorkerTimeout),
        }
    }

    pub(super) async fn remove(&self, config: &BackgroundConfig) -> Result<(), PluginError> {
        let mut command = builder::command(config)?;
        command.args(["rm", "--force", &self.name]);
        command.stdout(Stdio::null());
        let status = tokio::time::timeout(Duration::from_secs(15), command.status())
            .await
            .map_err(|_| PluginError::SandboxUnavailable)?
            .map_err(|_| PluginError::SandboxUnavailable)?;
        if status.success() {
            self.reaped.store(true, Ordering::Release);
            Ok(())
        } else {
            Err(PluginError::SandboxUnavailable)
        }
    }
}

impl Drop for Prepared {
    fn drop(&mut self) {
        if self.reaped.load(Ordering::Acquire) {
            return;
        }
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let name = self.name.clone();
            let config = self.config.clone();
            runtime.spawn(async move {
                if let Ok(mut command) = builder::command(&config) {
                    command.args(["rm", "--force", &name]);
                    command.stdout(Stdio::null());
                    let _ = tokio::time::timeout(Duration::from_secs(15), command.status()).await;
                }
            });
        }
    }
}
