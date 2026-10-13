use super::model::{JobError, ProviderOption, ProviderPin, digest};
use crate::{
    config::AppConfig,
    providers::config_catalog::{ProviderOrigin, ProviderRef},
};
use sqlx::SqliteConnection;

/// Uses the existing catalog's canonical refs: config:llm:key or a database Resource Key.
/// IDs and revisions accompany refs so delete/recreate cannot silently select a new provider.
pub(crate) async fn pin(
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
pub(crate) async fn options(
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
