use super::DatabaseRuntimeFailure;
use crate::{config::AppConfig, database::DesiredProvider};

/// Credential-free immutable deployment metadata. Identity is supplied separately by the
/// manager; database IDs can never be interpreted as deployment identities.
pub fn deployment_provider_snapshot(
    config: &AppConfig,
    kind: &str,
    key: &str,
) -> Result<DesiredProvider, DatabaseRuntimeFailure> {
    let value = match kind {
        "speaker" => serde_json::to_value(
            config
                .providers
                .speaker
                .instances
                .get(key)
                .ok_or(DatabaseRuntimeFailure::Configuration)?,
        ),
        "vad" => serde_json::to_value(
            config
                .providers
                .vad
                .instances
                .get(key)
                .ok_or(DatabaseRuntimeFailure::Configuration)?,
        ),
        "asr" => serde_json::to_value(
            config
                .providers
                .asr
                .instances
                .get(key)
                .ok_or(DatabaseRuntimeFailure::Configuration)?,
        ),
        "llm" => serde_json::to_value(
            config
                .providers
                .llm
                .instances
                .get(key)
                .ok_or(DatabaseRuntimeFailure::Configuration)?,
        ),
        "tts" => serde_json::to_value(
            config
                .providers
                .tts
                .instances
                .get(key)
                .ok_or(DatabaseRuntimeFailure::Configuration)?,
        ),
        _ => return Err(DatabaseRuntimeFailure::Configuration),
    }
    .map_err(|_| DatabaseRuntimeFailure::Configuration)?;
    let mut object = value
        .as_object()
        .cloned()
        .ok_or(DatabaseRuntimeFailure::Configuration)?;
    let adapter = object
        .remove("adapter")
        .and_then(|v| v.as_str().map(str::to_owned))
        .ok_or(DatabaseRuntimeFailure::Configuration)?;
    object.remove("preload");
    let raw = serde_json::to_string(&object).map_err(|_| DatabaseRuntimeFailure::Configuration)?;
    if raw.len() > crate::database::provider_config::MAX_PROVIDER_CONFIG_BYTES {
        return Err(DatabaseRuntimeFailure::Configuration);
    }
    let config_json = raw;
    Ok(DesiredProvider {
        id: 0,
        key: key.into(),
        kind: kind.into(),
        adapter,
        config_json,
        secret_ref: None,
        revision: 1,
    })
}
