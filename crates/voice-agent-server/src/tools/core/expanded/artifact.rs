use crate::services::background::{JobEngine, plugins::PluginError};
use serde_json::{Value, json};

/// Verify both the Job ownership and the existing durable artifact bytes/hash.
/// The caller cannot supply a filesystem path.
pub(super) async fn owned_artifact(
    engine: &JobEngine,
    job_id: &str,
    artifact_id: &str,
) -> Result<(String, Vec<u8>, String), PluginError> {
    if uuid::Uuid::parse_str(artifact_id).is_err() {
        return Err(PluginError::ApplicationError);
    }
    let metadata = engine
        .database
        .job_artifact_metadata(job_id, artifact_id)
        .await
        .map_err(|_| PluginError::DatabaseUnavailable)?;
    let (media_type, digest) = metadata.ok_or(PluginError::ApplicationError)?;
    let bytes = engine
        .read_artifact(job_id, artifact_id)
        .await
        .map_err(|_| PluginError::ApplicationError)?;
    Ok((media_type, bytes, digest))
}

pub(super) async fn read_owned_text(
    engine: &JobEngine,
    job_id: &str,
    artifact_id: &str,
) -> Result<(String, String, usize), PluginError> {
    let (media_type, bytes, _) = owned_artifact(engine, job_id, artifact_id).await?;
    if !matches!(
        media_type.as_str(),
        "text/plain" | "text/markdown" | "application/json"
    ) {
        return Err(PluginError::ApplicationError);
    }
    let size = bytes.len();
    let content = String::from_utf8(bytes).map_err(|_| PluginError::ApplicationError)?;
    Ok((media_type, content, size))
}

pub(super) async fn inspect(
    engine: &JobEngine,
    arguments: &Value,
    job_id: &str,
) -> Result<Value, PluginError> {
    let id = arguments["artifact_id"]
        .as_str()
        .ok_or(PluginError::ApplicationError)?;
    let (media_type, bytes, sha) = owned_artifact(engine, job_id, id).await?;
    Ok(json!({"artifact_id":id,"media_type":media_type,
        "size_bytes":bytes.len(),"sha256":sha}))
}
