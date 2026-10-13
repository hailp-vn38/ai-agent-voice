use super::*;

pub(super) async fn acquire_worker_until(
    slots: &Semaphore,
    deadline_at: i64,
) -> Result<tokio::sync::SemaphorePermit<'_>, PluginError> {
    let remaining = remaining_until(deadline_at)?;
    tokio::time::timeout(remaining, slots.acquire())
        .await
        .map_err(|_| {
            PluginError::JobRejected(crate::services::background::JobError::DeadlineExceeded)
        })?
        .map_err(|_| PluginError::WorkerCapacityUnavailable)
}

fn remaining_until(deadline_at: i64) -> Result<std::time::Duration, PluginError> {
    let deadline = std::time::UNIX_EPOCH
        .checked_add(std::time::Duration::from_secs(
            u64::try_from(deadline_at).map_err(|_| {
                PluginError::JobRejected(crate::services::background::JobError::DeadlineExceeded)
            })?,
        ))
        .ok_or(PluginError::JobRejected(
            crate::services::background::JobError::DeadlineExceeded,
        ))?;
    deadline
        .duration_since(std::time::SystemTime::now())
        .map_err(|_| {
            PluginError::JobRejected(crate::services::background::JobError::DeadlineExceeded)
        })
}

pub(super) fn classify_worker_timeout(deadline_at: i64) -> PluginError {
    if remaining_until(deadline_at).is_err() {
        PluginError::WorkerDeadlineExceeded
    } else {
        PluginError::WorkerTimeout
    }
}

#[allow(dead_code)]
#[derive(Clone, Debug, Deserialize)]
pub(crate) struct FrozenChoice {
    pub plugin_id: String,
    pub version: String,
    pub package_digest: String,
    pub image_digest: String,
    pub operation: String,
    pub permission_digest: String,
}

#[allow(dead_code)]
pub(crate) struct ArtifactData {
    pub bytes: Vec<u8>,
    pub media_type: String,
}

#[allow(dead_code)]
pub(crate) struct InvokeResult {
    pub result: serde_json::Value,
    pub artifacts: Vec<ArtifactData>,
}

#[allow(dead_code)]
pub(crate) struct Invocation<'a> {
    pub inputs: serde_json::Value,
    pub choice: &'a FrozenChoice,
    pub arguments: serde_json::Value,
    pub job_id: &'a str,
    pub attempt_id: &'a str,
    pub timeout: std::time::Duration,
    pub deadline_at: i64,
    pub cancellation: tokio_util::sync::CancellationToken,
}

impl PluginStore {
    #[allow(dead_code)]
    pub(crate) async fn invoke<F, Fut, G, Started>(
        &self,
        invocation: Invocation<'_>,
        before_start: F,
        after_start: G,
    ) -> Result<InvokeResult, PluginError>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<(), PluginError>>,
        G: FnOnce() -> Started,
        Started: std::future::Future<Output = Result<(), PluginError>>,
    {
        let Invocation {
            inputs,
            choice,
            arguments,
            job_id,
            attempt_id,
            timeout,
            deadline_at,
            cancellation,
        } = invocation;
        let _worker = tokio::select! {
            _ = cancellation.cancelled() => return Err(PluginError::JobRejected(crate::services::background::JobError::Cancelled)),
            result = acquire_worker_until(&self.worker_slots, deadline_at) => result?,
        };
        let row = self.row(&choice.plugin_id).await?;
        if row.status != "ready"
            || row.version != choice.version
            || row.package_digest != choice.package_digest
            || row.image_digest != choice.image_digest
        {
            return Err(PluginError::PermissionRequired);
        }
        let approval = row.approval()?.ok_or(PluginError::PermissionRequired)?;
        if crate::services::background::model::digest(
            &serde_json::to_string(&approval).map_err(|_| PluginError::DatabaseUnavailable)?,
        ) != choice.permission_digest
        {
            return Err(PluginError::PermissionRequired);
        }
        let manifest = row.manifest()?;
        let operation = manifest
            .operations
            .get(&choice.operation)
            .ok_or(PluginError::PermissionRequired)?;
        if operation.replay != "safe_read" {
            return Err(PluginError::PermissionRequired);
        }
        let package_path = self
            .root
            .join("packages")
            .join(&choice.plugin_id)
            .join(&choice.version)
            .join(&choice.package_digest)
            .join("package.zip");
        let package_bytes = tokio::fs::read(package_path)
            .await
            .map_err(|_| PluginError::StorageUnavailable)?;
        let package = archive::validate(&package_bytes)?;
        if package.digest != choice.package_digest || package.manifest.plugin.id != choice.plugin_id
        {
            return Err(PluginError::InvalidPackage);
        }
        let schema = |path: &str| -> Result<serde_json::Value, PluginError> {
            let bytes = package
                .files
                .iter()
                .find(|(name, _)| name == path)
                .map(|(_, bytes)| bytes)
                .ok_or(PluginError::InvalidPackage)?;
            serde_json::from_slice(bytes).map_err(|_| PluginError::InvalidPackage)
        };
        let input_schema = schema(&operation.input_schema)?;
        let output_schema = schema(&operation.output_schema)?;
        let validator =
            jsonschema::validator_for(&input_schema).map_err(|_| PluginError::InvalidPackage)?;
        if !validator.is_valid(&arguments) {
            return Err(PluginError::InvalidPackage);
        }
        builder::preflight(&self.config, &manifest.runtime).await?;
        builder::verify_image(
            &self.config,
            &choice.image_digest,
            &choice.package_digest,
            &manifest.runtime,
        )
        .await?;
        let workspace = self.root.join("workspaces").join(job_id).join(attempt_id);
        let out = workspace.join("out");
        tokio::fs::create_dir_all(&out)
            .await
            .map_err(|_| PluginError::StorageUnavailable)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&out, std::fs::Permissions::from_mode(0o777))
                .await
                .map_err(|_| PluginError::StorageUnavailable)?;
        }
        let mut egress = if manifest.permissions.network_hosts.is_empty() {
            None
        } else {
            if manifest
                .permissions
                .network_hosts
                .iter()
                .any(|host| !approval.network_hosts.contains(host))
            {
                let _ = tokio::fs::remove_dir_all(&workspace).await;
                return Err(PluginError::PermissionRequired);
            }
            match egress::EgressGuard::create(
                &self.config,
                attempt_id,
                job_id,
                &approval.network_hosts,
            )
            .await
            {
                Ok(guard) => Some(guard),
                Err(error) => {
                    let _ = tokio::fs::remove_dir_all(&workspace).await;
                    return Err(error);
                }
            }
        };
        let prepared = runner::Prepared::create(
            &self.config,
            &choice.image_digest,
            &workspace,
            job_id,
            attempt_id,
            egress.as_ref().map(|guard| guard.internal.as_str()),
        )
        .await;
        let prepared = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                let cleanup = if let Some(ref mut guard) = egress {
                    guard.remove().await
                } else {
                    Ok(())
                };
                if let Err(cleanup) = tokio::fs::remove_dir_all(&workspace).await {
                    tracing::warn!(job_id, attempt_id, %cleanup, "Plugin workspace cleanup failed");
                }
                cleanup?;
                return Err(error);
            }
        };
        let outcome = async {
            let _publication = self.database.tool_security.publication.read().await;
            if cancellation.is_cancelled() { return Err(PluginError::JobRejected(crate::services::background::JobError::Cancelled)); }
            before_start().await?;
            let start_time = std::time::Instant::now();
            prepared.start_container(&self.config).await?;
            tracing::info!(event="background_container_started", job_id, attempt_id, container_start_ms=start_time.elapsed().as_millis() as u64, memory_mib=512, cpu_count=1, pids_limit=64);
            after_start().await?;
            drop(_publication);
            let mut envelope=json!({
                "schema_version":1,"request_id":attempt_id,"operation":choice.operation,
                "arguments":arguments,"artifact_dir":"/work/out"
            });
            if inputs.as_object().is_some_and(|inputs| !inputs.is_empty()) {envelope["inputs"]=inputs;}
            let request = serde_json::to_vec(&envelope)
            .map_err(|_| PluginError::InvalidPackage)?;
            if request.len() > 65535 {
                return Err(PluginError::InvalidPackage);
            }
            let mut request = request;
            request.push(b'\n');
            let execution_limit = timeout.min(
                remaining_until(deadline_at).map_err(|_| PluginError::WorkerDeadlineExceeded)?,
            );
            let bytes = tokio::select! {
                _ = cancellation.cancelled() => Err(PluginError::JobRejected(crate::services::background::JobError::Cancelled)),
                result = prepared.attach(&self.config, &request, execution_limit) => result,
            }
                .map_err(|error| {
                    if error == PluginError::WorkerTimeout {
                        classify_worker_timeout(deadline_at)
                    } else {
                        error
                    }
                })?;
            let response = protocol::parse_response(&bytes, attempt_id, &output_schema)?;
            if !response.ok {
                return Err(PluginError::ApplicationError);
            }
            Ok(response)
        }
        .await;
        let reap_started = std::time::Instant::now();
        let reaped = prepared.remove(&self.config).await;
        let proxy_reaped = if reaped.is_ok() {
            if let Some(ref mut guard) = egress {
                guard.remove().await
            } else {
                Ok(())
            }
        } else {
            Ok(())
        };
        if cancellation.is_cancelled() {
            tracing::info!(
                event = "background_container_cancel_reaped",
                job_id,
                attempt_id,
                reap_ms = reap_started.elapsed().as_millis() as u64,
                reaped = reaped.is_ok()
            );
        }
        if reaped.is_err() || proxy_reaped.is_err() {
            return Err(PluginError::WorkerUnreaped);
        }
        let result = async {
            let response = outcome?;
            let artifacts = read_artifacts(
                &out,
                response.artifacts,
                self.config.limits.artifact_bytes_per_job,
            )
            .await?;
            Ok(InvokeResult {
                result: response.result.ok_or(PluginError::WorkerProtocolError)?,
                artifacts,
            })
        }
        .await;
        if let Err(cleanup) = tokio::fs::remove_dir_all(&workspace).await {
            tracing::warn!(job_id, attempt_id, %cleanup, "Plugin workspace cleanup failed");
        }
        result
    }
}

pub(super) async fn read_artifacts(
    out: &std::path::Path,
    refs: Vec<protocol::ArtifactRef>,
    limit: u64,
) -> Result<Vec<ArtifactData>, PluginError> {
    let mut artifacts = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut total = 0usize;
    for artifact in refs {
        if artifact.path.is_empty()
            || !seen.insert(artifact.path.clone())
            || artifact.path.len() > 80
            || !artifact
                .path
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            || !matches!(
                artifact.media_type.as_str(),
                "text/plain" | "application/json" | "text/markdown"
            )
        {
            return Err(PluginError::WorkerProtocolError);
        }
        let path = out.join(&artifact.path);
        let bytes = crate::services::background::artifacts::read_regular(path, limit)
            .await
            .map_err(|_| PluginError::WorkerProtocolError)?;
        total = total
            .checked_add(bytes.len())
            .ok_or(PluginError::WorkerProtocolError)?;
        if total as u64 > limit {
            return Err(PluginError::WorkerProtocolError);
        }
        let sha: String = {
            use sha2::{Digest, Sha256};
            Sha256::digest(&bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect()
        };
        if artifact
            .size_bytes
            .is_some_and(|size| size != bytes.len() as u64)
            || artifact
                .sha256
                .as_ref()
                .is_some_and(|digest| digest != &sha)
        {
            return Err(PluginError::WorkerProtocolError);
        }
        artifacts.push(ArtifactData {
            bytes,
            media_type: artifact.media_type,
        });
    }
    Ok(artifacts)
}
