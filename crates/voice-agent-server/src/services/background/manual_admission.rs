//! Manual operation admission, artifact import, and idempotent submissions.
//! This is separate from Root/Child Agent orchestration.
use super::{JobEngine, JobError, JobReceipt, JobRequester, engine::now};
use std::sync::atomic::Ordering;

/// A source artifact may enter a manual Job only by checked, bounded import.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactInputBinding {
    pub argument_pointer: String,
    pub source_job_id: String,
    pub source_artifact_id: String,
}

impl JobEngine {
    /// Admin-only, no LLM or Root/Child Sessions. All execution is delegated
    /// to the already durable Plugin Step executor after scheduler admission.
    pub async fn submit_plugin_operation(
        &self,
        key: &str,
        operation: &str,
        arguments: serde_json::Value,
    ) -> Result<JobReceipt, JobError> {
        self.submit_plugin_operation_with_bindings(key, operation, arguments, Vec::new(), None)
            .await
    }

    pub async fn submit_plugin_operation_with_bindings(
        &self,
        key: &str,
        operation: &str,
        mut arguments: serde_json::Value,
        bindings: Vec<ArtifactInputBinding>,
        key_idempotency: Option<&str>,
    ) -> Result<JobReceipt, JobError> {
        if key_idempotency.is_some_and(|key| {
            key.is_empty() || key.len() > 128 || !key.bytes().all(|byte| byte.is_ascii_graphic())
        }) {
            return Err(JobError::InvalidArguments);
        }
        let fingerprint = super::model::digest(
            &serde_json::json!({
                "plugin":key,"operation":operation,"arguments":arguments,
                "artifact_bindings":bindings
            })
            .to_string(),
        );
        // A lost HTTP ACK is replayed before plugin readiness/config checks.
        if let Some(key) = key_idempotency
            && let Some(old) = self
                .database
                .receipt_by_key(&JobRequester::Admin, key)
                .await?
        {
            let row = self.database.job_row(&old.job.id).await?;
            if row.execution_mode != "plugin_operation_v3" || row.request_fingerprint != fingerprint
            {
                return Err(JobError::IdempotencyConflict);
            }
            return Ok(old);
        }
        if !self.running.load(Ordering::Acquire)
            || !arguments.is_object()
            || bindings.len() > 4
            || arguments.to_string().len() > 16_384
        {
            return Err(JobError::InvalidArguments);
        }
        let choice = if crate::tools::core::known_group(key) {
            crate::tools::core::frozen_choices()
                .into_iter()
                .find(|choice| {
                    choice["operation"] == operation
                        && crate::tools::core::group_for(operation) == Some(key)
                })
                .ok_or(JobError::CapabilityUnavailable)?
        } else {
            let installed = self.plugins.get(key).await.map_err(JobError::from_plugin)?;
            if installed.status != "ready"
                || !installed
                    .operations
                    .iter()
                    .any(|declared| declared.name == operation && declared.replay == "safe_read")
            {
                return Err(JobError::CapabilityUnavailable);
            }
            serde_json::json!({
                "plugin_id":key, "operation":operation,
                "version":installed.version,"package_digest":installed.digest,
                "installation_digest":installed.installation_digest
            })
        };
        let contract = self
            .plugins
            .operation_descriptor(key, operation)
            .await
            .map_err(JobError::from_plugin)?;
        self.plugins
            .value_settings_activatable(key)
            .await
            .map_err(JobError::from_plugin)?;

        let requires_artifact = choice["kind"] == "core"
            && (operation.starts_with("artifact.") || operation.starts_with("document."));
        if (requires_artifact && bindings.is_empty())
            || (!bindings.is_empty() && !requires_artifact)
        {
            return Err(JobError::UnsupportedInputBinding);
        }

        let job_id = uuid::Uuid::new_v4().to_string();
        let artifact_root = super::artifacts::root(&self.database)?.join(&job_id);
        let mut imported = Vec::new();
        let mut created_dir = false;
        let result = async {
            if !bindings.is_empty() {
                // A new UUID directory must not be a pre-existing symlink.
                tokio::fs::create_dir(&artifact_root)
                    .await
                    .map_err(|_| JobError::DatabaseUnavailable)?;
                created_dir = true;
            }
            let mut total = 0_u64;
            let mut pointers = std::collections::HashSet::new();
            for binding in bindings {
                let name = binding
                    .argument_pointer
                    .strip_prefix('/')
                    .filter(|name| !name.is_empty() && !name.contains('/') && !name.contains('~'))
                    .ok_or(JobError::UnsupportedInputBinding)?;
                if !pointers.insert(name.to_owned())
                    || contract["input_schema"]["properties"][name]["type"] != "string"
                {
                    return Err(JobError::UnsupportedInputBinding);
                }
                let source = self.get(&binding.source_job_id).await?;
                if !matches!(source.status.as_str(), "succeeded" | "partial_success") {
                    return Err(JobError::UnsupportedInputBinding);
                }
                let artifact = source
                    .artifacts
                    .iter()
                    .find(|artifact| artifact.id == binding.source_artifact_id)
                    .ok_or(JobError::NotFound)?;
                let size =
                    u64::try_from(artifact.size_bytes).map_err(|_| JobError::InvalidArguments)?;
                total = total
                    .checked_add(size)
                    .ok_or(JobError::CapacityUnavailable)?;
                let allowed = self
                    .config
                    .background
                    .limits
                    .artifact_bytes_per_job
                    .min(20 * 1024 * 1024);
                if total > allowed {
                    return Err(JobError::CapacityUnavailable);
                }
                let bytes = self
                    .read_artifact(&binding.source_job_id, &binding.source_artifact_id)
                    .await?;
                let id = uuid::Uuid::new_v4().to_string();
                let staging = artifact_root.join(format!(".{id}.staging"));
                let target = artifact_root.join(&id);
                use tokio::io::AsyncWriteExt;
                let mut output = tokio::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&staging)
                    .await
                    .map_err(|_| JobError::DatabaseUnavailable)?;
                output
                    .write_all(&bytes)
                    .await
                    .map_err(|_| JobError::DatabaseUnavailable)?;
                output
                    .sync_all()
                    .await
                    .map_err(|_| JobError::DatabaseUnavailable)?;
                drop(output);
                tokio::fs::rename(&staging, &target)
                    .await
                    .map_err(|_| JobError::DatabaseUnavailable)?;
                let args = arguments
                    .as_object_mut()
                    .ok_or(JobError::InvalidArguments)?;
                args.insert(name.to_owned(), serde_json::Value::String(id.clone()));
                imported.push(crate::database::jobs::ImportedArtifact {
                    id,
                    filename: artifact.filename.clone(),
                    media_type: artifact.media_type.clone(),
                    size_bytes: artifact.size_bytes,
                    sha256: artifact.sha256.clone(),
                });
            }
            if created_dir {
                super::artifacts::sync_directory(artifact_root.clone())
                    .await
                    .map_err(|_| JobError::DatabaseUnavailable)?;
            }
            let schema = jsonschema::validator_for(&contract["input_schema"])
                .map_err(|_| JobError::CapabilityUnavailable)?;
            if !schema.is_valid(&arguments) || arguments.to_string().len() > 16_384 {
                return Err(JobError::InvalidArguments);
            }
            let _submission = self.submission_gate.read().await;
            self.database
                .submit_plugin_operation(
                    &self.config,
                    key,
                    &choice,
                    &arguments,
                    &contract["output_schema"],
                    now()?,
                    &job_id,
                    &imported,
                    &fingerprint,
                    key_idempotency,
                )
                .await
        }
        .await;
        if (result.is_err() || result.as_ref().is_ok_and(|receipt| receipt.replayed)) && created_dir
        {
            if let Err(error) = tokio::fs::remove_dir_all(&artifact_root).await {
                tracing::warn!(job_id=%job_id,%error,"Manual artifact stage cleanup failed");
            }
        }
        let receipt = result?;
        self.wake.notify_one();
        Ok(receipt)
    }
}
