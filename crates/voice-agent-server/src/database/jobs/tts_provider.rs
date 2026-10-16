//! Exact TTS provider identity is separate from pinned Plugin Value Settings.
use super::*;
use crate::{
    config::AppConfig,
    providers::config_catalog::{ProviderOrigin, ProviderRef},
    services::background::{ProviderPin, model::digest},
};

/// Return None when Core TTS is not configured for admission. Do not make unrelated
/// Agent Jobs fail because an optional Core TTS provider is unavailable.
pub(super) async fn configured_pin(
    tx: &mut SqliteConnection,
    config: &AppConfig,
) -> Result<Option<ProviderPin>, JobError> {
    let row: Option<(String, String)> = sqlx::query_as(
        "SELECT v.public_json,v.schema_digest FROM plugin_configs c
         JOIN plugin_config_versions v ON c.plugin_key=v.plugin_key
           AND c.current_revision=v.revision WHERE c.plugin_key='core.tts'"
    ).fetch_optional(&mut *tx).await?;
    let Some((settings, digest)) = row else { return Ok(None) };
    if digest != crate::tools::core::tts_settings_digest() {
        return Ok(None);
    }
    let values: serde_json::Value =
        serde_json::from_str(&settings).map_err(|_| JobError::DatabaseUnavailable)?;
    let Some(reference) = values.get("provider_ref").and_then(|value| value.as_str()) else {
        return Ok(None);
    };
    Ok(pin(tx,config,reference).await.ok())
}

/// Same exact-incarnation semantics as Background LLM, qualified to TTS.
pub(super) async fn pin(
    tx: &mut SqliteConnection,
    config: &AppConfig,
    reference: &str,
) -> Result<ProviderPin, JobError> {
    let parsed = ProviderRef::parse(reference, Some("tts"))
        .map_err(|_| JobError::InvalidArguments)?;
    let (incarnation, revision, adapter, configuration) = match parsed.origin {
        ProviderOrigin::Config => {
            let row = parsed.snapshot(config).map_err(|_| JobError::CapabilityUnavailable)?;
            (0, row.revision, row.adapter, row.config_json)
        }
        ProviderOrigin::Database => {
            sqlx::query_as::<_, (i64,i64,String,String)>(
                "SELECT id,revision,adapter,config_json FROM providers
                 WHERE key=? AND type='tts' AND enabled=1"
            ).bind(&parsed.key).fetch_optional(&mut *tx).await?
                .ok_or(JobError::CapabilityUnavailable)?
        }
    };
    Ok(ProviderPin {
        reference: parsed.public_key(),
        incarnation,
        revision,
        digest: digest(&format!("{adapter}\n{configuration}")),
    })
}

pub(super) async fn insert_pin(
    tx: &mut SqliteConnection,
    job_id: &str,
    pin: &ProviderPin,
) -> Result<(), JobError> {
    sqlx::query("INSERT INTO job_tts_provider_pins
      (job_id,provider_ref,incarnation,revision,digest) VALUES(?,?,?,?,?)")
        .bind(job_id).bind(&pin.reference).bind(pin.incarnation)
        .bind(pin.revision).bind(&pin.digest).execute(&mut *tx).await?;
    Ok(())
}

impl Database {
    /// Resolve and verify the *same* runtime identity, not the latest provider selection.
    pub(crate) async fn pinned_tts_provider(
        &self,
        config: &AppConfig,
        job_id: &str,
    ) -> Result<(crate::database::DesiredProvider, ProviderOrigin, String), JobError> {
        let mut tx = self.pool().acquire().await?;
        let pin: Option<(String,i64,i64,String)> = sqlx::query_as(
            "SELECT provider_ref,incarnation,revision,digest
             FROM job_tts_provider_pins WHERE job_id=?"
        ).bind(job_id).fetch_optional(&mut *tx).await?;
        let (reference,incarnation,revision,old_digest) =
            pin.ok_or(JobError::CapabilityUnavailable)?;
        let current = pin(&mut tx, config, &reference).await
            .map_err(|_| JobError::ConfigurationChanged)?;
        if current.incarnation != incarnation || current.revision != revision
            || current.digest != old_digest {
            return Err(JobError::ConfigurationChanged);
        }
        let parsed = ProviderRef::parse(&reference,Some("tts"))
            .map_err(|_| JobError::CapabilityUnavailable)?;
        let snapshot = match parsed.origin {
            ProviderOrigin::Config => parsed.snapshot(config)
                .map_err(|_| JobError::ConfigurationChanged)?,
            ProviderOrigin::Database => {
                let (id,key,kind,adapter,config_json,revision,credential):
                    (i64,String,String,String,String,i64,Option<String>) =
                    sqlx::query_as("SELECT id,key,type,adapter,config_json,revision,credential_json
                                   FROM providers WHERE key=? AND type='tts' AND enabled=1")
                    .bind(&parsed.key).fetch_optional(&mut *tx).await?
                    .ok_or(JobError::ConfigurationChanged)?;
                let secret_ref=crate::database::credentials::reference(
                    &format!("provider:{key}"),credential.as_deref(),
                    crate::database::secrets::provider_secret_env(&key,&adapter)
                );
                crate::database::DesiredProvider{
                    id,key,kind,adapter,config_json,revision,secret_ref
                }
            }
        };
        Ok((snapshot,parsed.origin,parsed.key))
    }
}
