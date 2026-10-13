use super::{PluginError, builder};
use crate::config::BackgroundConfig;
use std::time::Duration;

const OWNER: &str = "agent.job.owner=voice-agent-server";

async fn docker(
    config: &BackgroundConfig,
    args: &[&str],
    timeout: u64,
) -> Result<String, PluginError> {
    let mut command = builder::command(config)?;
    command.args(args);
    let output = tokio::time::timeout(Duration::from_secs(timeout), command.output())
        .await
        .map_err(|_| PluginError::EgressPolicyUnavailable)?
        .map_err(|_| PluginError::EgressPolicyUnavailable)?;
    if !output.status.success() || output.stdout.len() > 4096 {
        return Err(PluginError::EgressPolicyUnavailable);
    }
    String::from_utf8(output.stdout).map_err(|_| PluginError::EgressPolicyUnavailable)
}

fn image(config: &BackgroundConfig) -> Result<&str, PluginError> {
    let image = config
        .plugin_egress_image
        .as_deref()
        .ok_or(PluginError::EgressPolicyUnavailable)?;
    let digest = image
        .strip_prefix("sha256:")
        .ok_or(PluginError::EgressPolicyUnavailable)?;
    if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(PluginError::EgressPolicyUnavailable);
    }
    Ok(image)
}

fn qualified(config: &BackgroundConfig, hosts: &[String]) -> Result<(), PluginError> {
    if hosts.is_empty()
        || hosts.len() > 16
        || hosts
            .iter()
            .any(|host| !config.plugin_egress_qualified_hosts.contains(host))
    {
        return Err(PluginError::EgressPolicyUnavailable);
    }
    Ok(())
}

pub(super) async fn verify_image(config: &BackgroundConfig) -> Result<(), PluginError> {
    let image = image(config)?;
    if docker(
        config,
        &["image", "inspect", "--format", "{{.Id}}", image],
        5,
    )
    .await?
    .trim()
        != image
    {
        return Err(PluginError::EgressPolicyUnavailable);
    }
    let labels: serde_json::Value = serde_json::from_str(
        &docker(
            config,
            &[
                "image",
                "inspect",
                "--format",
                "{{json .Config.Labels}}",
                image,
            ],
            5,
        )
        .await?,
    )
    .map_err(|_| PluginError::EgressPolicyUnavailable)?;
    if labels["agent.egress.policy"] != "1" {
        return Err(PluginError::EgressPolicyUnavailable);
    }
    let expected_source = {
        use sha2::{Digest, Sha256};
        Sha256::digest(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../deploy/agent-egress-proxy/proxy.py"
        )))
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
    };
    if labels["agent.egress.base"] != config.plugin_python_image.as_deref().unwrap_or("")
        || labels["agent.egress.source"] != expected_source
    {
        return Err(PluginError::EgressPolicyUnavailable);
    }
    Ok(())
}

pub(super) async fn preflight(
    config: &BackgroundConfig,
    hosts: &[String],
) -> Result<(), PluginError> {
    let attempt = uuid::Uuid::new_v4().to_string();
    let mut guard = EgressGuard::create(config, &attempt, "preflight", hosts).await?;
    guard.remove().await
}

pub(super) struct EgressGuard {
    config: BackgroundConfig,
    pub(super) internal: String,
    external: String,
    proxy: String,
    internal_created: bool,
    external_created: bool,
    proxy_created: bool,
    cleaned: bool,
}

impl EgressGuard {
    pub(super) async fn create(
        config: &BackgroundConfig,
        attempt: &str,
        job: &str,
        hosts: &[String],
    ) -> Result<Self, PluginError> {
        if uuid::Uuid::parse_str(attempt).is_err() {
            return Err(PluginError::EgressPolicyUnavailable);
        }
        qualified(config, hosts)?;
        verify_image(config).await?;
        let mut guard = Self {
            config: config.clone(),
            internal: format!("agent-egress-private-{attempt}"),
            external: format!("agent-egress-uplink-{attempt}"),
            proxy: format!("agent-egress-proxy-{attempt}"),
            internal_created: false,
            external_created: false,
            proxy_created: false,
            cleaned: false,
        };
        let result = async {
            docker(config, &["network", "create", "--driver", "bridge", "--internal", "--label", OWNER, "--label", &format!("agent.job.attempt={attempt}"), &guard.internal], 15).await?;
            guard.internal_created = true;
            if docker(config, &["network", "inspect", "--format", "{{.Internal}}", &guard.internal], 5).await?.trim() != "true" {
                return Err(PluginError::EgressPolicyUnavailable);
            }
            docker(config, &["network", "create", "--driver", "bridge", "--label", OWNER, "--label", &format!("agent.job.attempt={attempt}"), &guard.external], 15).await?;
            guard.external_created = true;
            docker(config, &["create", "--name", &guard.proxy, "--pull=never", "--network", &guard.external,
                "--read-only", "--cap-drop=ALL", "--security-opt=no-new-privileges", "--pids-limit=32",
                "--memory=128m", "--cpus=0.5", "--user=65532:65532", "--env", &format!("AGENT_ALLOWED_HOSTS={}", hosts.join(",")),
                "--label", OWNER, "--label", &format!("agent.job.id={job}"), "--label", &format!("agent.job.attempt={attempt}"),
                image(config)?], 15).await?;
            guard.proxy_created = true;
            docker(config, &["start", &guard.proxy], 15).await?;
            docker(config, &["network", "connect", "--alias", "egress", &guard.internal, &guard.proxy], 15).await?;
            if docker(config, &["exec", &guard.proxy, "python", "-c", "import socket; s=socket.create_connection(('127.0.0.1',8080),3); s.close(); print('ready')"], 5).await?.trim() != "ready" {
                return Err(PluginError::EgressPolicyUnavailable);
            }
            Ok(())
        }.await;
        if let Err(error) = result {
            let _ = guard.remove().await;
            return Err(error);
        }
        Ok(guard)
    }

    pub(super) async fn remove(&mut self) -> Result<(), PluginError> {
        for (present, args) in [
            (self.proxy_created, vec!["rm", "--force", &self.proxy]),
            (self.internal_created, vec!["network", "rm", &self.internal]),
            (self.external_created, vec!["network", "rm", &self.external]),
        ] {
            if !present {
                continue;
            }
            let mut command = builder::command(&self.config)?;
            command.args(args);
            command.stdout(std::process::Stdio::null());
            let result = tokio::time::timeout(Duration::from_secs(15), command.status()).await;
            if !matches!(result, Ok(Ok(status)) if status.success()) {
                // A failed remove is retried by the next startup owner-label reaper.
                return Err(PluginError::WorkerUnreaped);
            }
        }
        self.cleaned = true;
        Ok(())
    }
}

impl Drop for EgressGuard {
    fn drop(&mut self) {
        if self.cleaned {
            return;
        }
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let config = self.config.clone();
            let proxy = self.proxy.clone();
            let internal = self.internal.clone();
            let external = self.external.clone();
            runtime.spawn(async move {
                for args in [
                    vec!["rm", "--force", &proxy],
                    vec!["network", "rm", &internal],
                    vec!["network", "rm", &external],
                ] {
                    if let Ok(mut command) = builder::command(&config) {
                        command.args(args);
                        command.stdout(std::process::Stdio::null());
                        let _ =
                            tokio::time::timeout(Duration::from_secs(15), command.status()).await;
                    }
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_hosts_require_explicit_deployment_qualification() {
        let mut config = BackgroundConfig::default();
        let host = vec!["example.com".to_owned()];
        assert_eq!(
            qualified(&config, &host),
            Err(PluginError::EgressPolicyUnavailable)
        );
        config.plugin_egress_qualified_hosts = host.clone();
        assert_eq!(qualified(&config, &host), Ok(()));
        assert_eq!(
            qualified(&config, &["other.example".to_owned()]),
            Err(PluginError::EgressPolicyUnavailable)
        );
    }
}
