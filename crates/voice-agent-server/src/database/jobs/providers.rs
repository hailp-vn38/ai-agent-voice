use super::*;
use crate::services::background::{JobError, ProviderOption, ProviderPin, model::digest};
use crate::{
    config::AppConfig,
    providers::config_catalog::{ProviderOrigin, ProviderRef},
};
use sqlx::SqliteConnection;

/// Uses the existing catalog's canonical refs: config:llm:key or a database Resource Key.
/// IDs and revisions accompany refs so delete/recreate cannot silently select a new provider.
pub(super) async fn pin(
    connection: &mut SqliteConnection,
    config: &AppConfig,
    reference: &str,
) -> Result<ProviderPin, JobError> {
    let parsed =
        ProviderRef::parse(reference, Some("llm")).map_err(|_| JobError::InvalidArguments)?;
    let (incarnation, revision, adapter, configuration) = match parsed.origin {
        ProviderOrigin::Config => {
            let row = parsed
                .snapshot(config)
                .map_err(|_| JobError::ProviderUnavailable)?;
            (0, row.revision, row.adapter, row.config_json)
        }
        ProviderOrigin::Database => {
            let row:Option<(i64,i64,String,String)>=sqlx::query_as("SELECT id,revision,adapter,config_json FROM providers WHERE key=? AND type='llm' AND enabled=1")
                .bind(&parsed.key).fetch_optional(connection).await?;
            row.ok_or(JobError::ProviderUnavailable)?
        }
    };
    Ok(ProviderPin {
        reference: parsed.public_key(),
        incarnation,
        revision,
        digest: digest(&format!("{adapter}\n{configuration}")),
    })
}
/// Capabilities are declared for the exact pinned model + transport; both
/// sources use the same canonical provider configuration.
pub(super) async fn require_hybrid_capability(
    connection: &mut SqliteConnection,
    config: &AppConfig,
    reference: &str,
    required_flag: &str,
) -> Result<(), JobError> {
    if !matches!(required_flag, "supports_structured_outputs" | "supports_function_calling") {
        return Err(JobError::InvalidArguments);
    }
    let parsed = ProviderRef::parse(reference,Some("llm")).map_err(|_|JobError::ProviderUnavailable)?;
    let (adapter, config_json) = match parsed.origin {
        ProviderOrigin::Config => {
            let row = parsed.snapshot(config).map_err(|_|JobError::ProviderUnavailable)?;
            (row.adapter,row.config_json)
        }
        ProviderOrigin::Database => {
            let row: Option<(String,String)> = sqlx::query_as(
                "SELECT adapter,config_json FROM providers WHERE key=? AND type='llm' AND enabled=1")
                .bind(&parsed.key).fetch_optional(connection).await?;
            row.ok_or(JobError::ProviderUnavailable)?
        }
    };
    if !matches!(adapter.as_str(),"openai"|"opencode_free") {return Err(JobError::CapabilityUnavailable);}
    let values: serde_json::Value = serde_json::from_str(&config_json).map_err(|_|JobError::ProviderUnavailable)?;
    if values[required_flag].as_bool()!=Some(true) {return Err(JobError::CapabilityUnavailable);}
    Ok(())
}

pub(super) async fn options(
    connection: &mut SqliteConnection,
    config: &AppConfig,
) -> Result<Vec<ProviderOption>, JobError> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT key,name FROM providers WHERE type='llm' AND enabled=1 ORDER BY key LIMIT 256",
    )
    .fetch_all(connection)
    .await?;
    let mut result: Vec<_> = rows
        .into_iter()
        .map(|(r#ref, name)| ProviderOption {
            r#ref,
            name,
            source: "database",
        })
        .collect();
    result.extend(
        crate::providers::config_catalog::deployment_snapshots(config)
            .into_iter()
            .filter(|r| r.kind == "llm")
            .map(|r| ProviderOption {
                r#ref: format!("config:llm:{}", r.key),
                name: r.key,
                source: "config",
            }),
    );
    Ok(result)
}

pub(crate) struct ResolvedBackgroundProvider {
    pub origin: ProviderOrigin,
    pub key: String,
    pub snapshot: crate::database::DesiredProvider,
    pub adapter: String,
    pub model: Option<String>,
    pub transport: &'static str,
}
impl Database {
    pub(crate) async fn background_provider_pin(
        &self,
        config: &AppConfig,
        reference: &str,
    ) -> Result<ProviderPin, JobError> {
        let mut connection = self.pool().acquire().await?;
        pin(&mut connection, config, reference).await
    }
    pub(crate) async fn background_provider_options(
        &self,
        config: &AppConfig,
    ) -> Result<Vec<ProviderOption>, JobError> {
        let mut connection = self.pool().acquire().await?;
        options(&mut connection, config).await
    }
    pub(crate) async fn background_resolve_provider(
        &self,
        config: &AppConfig,
        expected: &ProviderPin,
    ) -> Result<ResolvedBackgroundProvider, JobError> {
        let mut connection = self.pool().acquire().await?;
        if &pin(&mut connection, config, &expected.reference).await? != expected {
            return Err(JobError::ConfigurationChanged);
        }
        let parsed = ProviderRef::parse(&expected.reference, Some("llm"))
            .map_err(|_| JobError::ProviderUnavailable)?;
        let snapshot = match parsed.origin {
            ProviderOrigin::Config => parsed
                .snapshot(config)
                .map_err(|_| JobError::ProviderUnavailable)?,
            ProviderOrigin::Database => {
                let (id,key,kind,adapter,config_json,revision,credential): (i64,String,String,String,String,i64,Option<String>) =
                    sqlx::query_as("SELECT id,key,type,adapter,config_json,revision,credential_json FROM providers WHERE key=? AND type='llm' AND enabled=1")
                        .bind(&parsed.key).fetch_optional(&mut *connection).await?.ok_or(JobError::ProviderUnavailable)?;
                // Keep the canonical credential reference on background reload, including
                // ProviderRuntimeManager idle eviction. Same behavior as latest main.
                let secret_ref = crate::database::credentials::reference(
                    &format!("provider:{key}"),
                    credential.as_deref(),
                    crate::database::secrets::provider_secret_env(&key, &adapter),
                );
                crate::database::DesiredProvider {
                    id,
                    key,
                    kind,
                    adapter,
                    config_json,
                    revision,
                    secret_ref,
                }
            }
        };
        let identity: serde_json::Value =
            serde_json::from_str(&snapshot.config_json).unwrap_or_default();
        let adapter = snapshot.adapter.clone();
        let model = identity
            .get("model")
            .and_then(|v| v.as_str())
            .map(str::to_owned)
            .or_else(|| (adapter == "opencode_free").then(|| "nemotron-3-ultra-free".to_owned()));
        let transport = if identity.get("supports_responses").and_then(|v| v.as_bool())
            == Some(true)
            && matches!(adapter.as_str(), "openai" | "opencode_free")
        {
            "responses"
        } else {
            "chat_completions"
        };
        Ok(ResolvedBackgroundProvider {
            origin: parsed.origin,
            key: parsed.key,
            snapshot,
            adapter,
            model,
            transport,
        })
    }
    pub(crate) async fn background_authorize_provider_call(
        &self,
        config: &AppConfig,
        job_id: &str,
        expected: &ProviderPin,
    ) -> Result<(), JobError> {
        let mut connection = self.pool().acquire().await?;
        let row: JobRow = sqlx::query_as("SELECT * FROM jobs WHERE id=?")
            .bind(job_id)
            .fetch_optional(&mut *connection)
            .await?
            .ok_or(JobError::NotFound)?;
        if !matches!(row.status.as_str(), "planning" | "running") {
            return Err(JobError::Cancelled);
        }
        authorize(&mut connection, &row.requester()?).await?;
        let enabled: i64 =
            sqlx::query_scalar("SELECT enabled FROM background_settings WHERE singleton=1")
                .fetch_one(&mut *connection)
                .await?;
        if enabled != 1 {
            return Err(JobError::Disabled);
        }
        if &pin(&mut connection, config, &expected.reference).await? != expected {
            return Err(JobError::ConfigurationChanged);
        }
        Ok(())
    }
}
