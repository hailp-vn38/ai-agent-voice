//! Provider desired-state reads and atomic audited mutations; runtime work stays in services.
use super::{
    Database, DatabaseError,
    audit::{AuditOutcome, audit, audit_conflict},
    credentials, map_sqlx_error, secrets,
    writes::WriteError,
};
use serde::Serialize;
use serde_json::Value;
use sqlx::{FromRow, QueryBuilder, Sqlite, SqlitePool};
#[derive(Serialize, FromRow)]
pub(crate) struct Provider {
    pub(crate) id: i64,
    pub(crate) key: String,
    pub(crate) name: String,
    #[serde(rename = "type")]
    pub(crate) kind: String,
    pub(crate) adapter: String,
    pub(crate) config_json: String,
    pub(crate) enabled: i64,
    pub(crate) revision: i64,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
    #[serde(skip)]
    pub(crate) credential_json: Option<String>,
}
pub(crate) async fn provider_by(database: &Database, key: &str) -> Result<Provider, sqlx::Error> {
    sqlx::query_as("SELECT id,key,name,type AS kind,adapter,config_json,enabled,revision,created_at,updated_at,credential_json FROM providers WHERE key=?").bind(key).fetch_one(&database.pool).await
}
pub(crate) struct ProviderFilters {
    enabled: Option<bool>,
    q: Option<String>,
    kind: Option<String>,
}
fn append_provider_filters(
    builder: &mut QueryBuilder<Sqlite>,
    filters: &ProviderFilters,
    include_kind: bool,
) {
    let mut first = true;
    let mut clause = |builder: &mut QueryBuilder<Sqlite>| {
        builder.push(if first { " WHERE " } else { " AND " });
        first = false;
    };
    if let Some(enabled) = filters.enabled {
        clause(builder);
        builder.push("enabled=").push_bind(i64::from(enabled));
    }
    if include_kind && let Some(kind) = &filters.kind {
        clause(builder);
        builder.push("type=").push_bind(kind.clone());
    }
    if let Some(q) = &filters.q {
        clause(builder);
        let q = format!(
            "%{}%",
            q.replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        );
        builder
            .push("(key LIKE ")
            .push_bind(q.clone())
            .push(" ESCAPE '\\' OR name LIKE ")
            .push_bind(q)
            .push(" ESCAPE '\\')");
    }
}
async fn provider_count(
    pool: &SqlitePool,
    filters: &ProviderFilters,
    include_kind: bool,
) -> Result<i64, sqlx::Error> {
    let mut builder = QueryBuilder::<Sqlite>::new("SELECT COUNT(*) FROM providers");
    append_provider_filters(&mut builder, filters, include_kind);
    builder.build_query_scalar().fetch_one(pool).await
}
async fn provider_facets(
    pool: &SqlitePool,
    filters: &ProviderFilters,
) -> Result<Value, sqlx::Error> {
    let mut builder = QueryBuilder::<Sqlite>::new("SELECT type, COUNT(*) FROM providers");
    append_provider_filters(&mut builder, filters, false);
    builder.push(" GROUP BY type");
    let counts: Vec<(String, i64)> = builder.build_query_as().fetch_all(pool).await?;
    let mut facets = serde_json::Map::new();
    for kind in ["vad", "asr", "llm", "tts"] {
        facets.insert(
            kind.into(),
            Value::from(
                counts
                    .iter()
                    .find(|(value, _)| value == kind)
                    .map(|(_, count)| *count)
                    .unwrap_or(0),
            ),
        );
    }
    Ok(Value::Object(facets))
}
impl ProviderFilters {
    pub(crate) fn new(
        enabled: Option<bool>,
        q: Option<String>,
        kind: Option<String>,
    ) -> Result<Self, ()> {
        if kind
            .as_deref()
            .is_some_and(|value| !matches!(value, "vad" | "asr" | "llm" | "tts"))
        {
            return Err(());
        }
        let q = q.filter(|value| !value.is_empty());
        if q.as_ref().is_some_and(|value| value.len() > 128) {
            return Err(());
        }
        Ok(Self { enabled, q, kind })
    }
}

pub(crate) struct NewProvider<'a> {
    pub key: &'a str,
    pub name: &'a str,
    pub kind: &'a str,
    pub adapter: &'a str,
    pub config: &'a str,
    pub credential: Option<&'a str>,
}
pub(crate) struct ProviderChanges<'a> {
    pub name: &'a str,
    pub adapter: &'a str,
    pub config: &'a str,
    pub credential: Option<&'a str>,
    pub enabled: i64,
}
pub(crate) struct ProviderPage {
    pub items: Vec<Provider>,
    pub total: i64,
    pub facets: Value,
}
impl Database {
    pub(crate) async fn create_provider(
        &self,
        input: NewProvider<'_>,
        request_id: &str,
    ) -> Result<(), WriteError> {
        let pool = &self.pool;
        let mut tx = match pool.begin().await {
            Ok(v) => v,
            Err(_) => {
                return Err(WriteError::Unavailable);
            }
        };
        let r=sqlx::query("INSERT INTO providers(key,name,type,adapter,config_json,credential_json,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?)").bind(input.key).bind(input.name).bind(input.kind).bind(input.adapter).bind(input.config).bind(input.credential).bind(crate::database::unix_seconds().unwrap_or_default()).bind(crate::database::unix_seconds().unwrap_or_default()).execute(&mut *tx).await;
        let provider_id = match r {
            Ok(v) => v.last_insert_rowid(),
            Err(e) => return Err(WriteError::Mutation(e)),
        };
        if audit(
            &mut *tx,
            request_id,
            "provider",
            Some(provider_id),
            "create",
            None,
            Some(1),
            AuditOutcome::Success,
            1,
        )
        .await
        .is_err()
            || tx.commit().await.is_err()
        {
            return Err(WriteError::Unavailable);
        };
        Ok(())
    }
    pub(crate) async fn update_provider(
        &self,
        provider_id: i64,
        expected: i64,
        changes: ProviderChanges<'_>,
        request_id: &str,
    ) -> Result<(), WriteError> {
        let pool = &self.pool;
        let mut tx = match pool.begin().await {
            Ok(v) => v,
            Err(_) => {
                return Err(WriteError::Unavailable);
            }
        };
        let changed = sqlx::query("UPDATE providers SET name=?,adapter=?,config_json=?,credential_json=?,enabled=?,revision=revision+1,updated_at=? WHERE id=? AND revision=?")
        .bind(changes.name).bind(changes.adapter).bind(changes.config).bind(changes.credential).bind(changes.enabled).bind(crate::database::unix_seconds().unwrap_or_default()).bind(provider_id).bind(expected)
        .execute(&mut *tx).await.map(|v| v.rows_affected() == 1).unwrap_or(false);
        if !changed {
            let _ = tx.rollback().await;
            return Err(WriteError::Conflict("revision_conflict"));
        }
        if audit(
            &mut *tx,
            request_id,
            "provider",
            Some(provider_id),
            "update",
            Some(expected),
            Some(expected + 1),
            AuditOutcome::Success,
            1,
        )
        .await
        .is_err()
            || tx.commit().await.is_err()
        {
            return Err(WriteError::Unavailable);
        }
        Ok(())
    }
    pub(crate) async fn provider_conflict(&self, request_id: &str, id: i64, expected: i64) {
        audit_conflict(&self.pool, request_id.into(), "provider", id, expected).await;
    }
    pub(crate) async fn list_providers(
        &self,
        filters: &ProviderFilters,
        sort: Option<&str>,
        page: u32,
        size: u32,
    ) -> Result<ProviderPage, sqlx::Error> {
        let order = match sort.unwrap_or("key") {
            "-key" => "key DESC",
            "name" => "name ASC, key ASC",
            "-name" => "name DESC, key ASC",
            _ => "key ASC",
        };
        let total = provider_count(&self.pool, filters, true).await?;
        let facets = provider_facets(&self.pool, filters).await?;
        let mut builder = QueryBuilder::<Sqlite>::new(
            "SELECT id,key,name,type AS kind,adapter,config_json,enabled,revision,created_at,updated_at,credential_json FROM providers",
        );
        append_provider_filters(&mut builder, filters, true);
        builder
            .push(" ORDER BY ")
            .push(order)
            .push(" LIMIT ")
            .push_bind(i64::from(size))
            .push(" OFFSET ")
            .push_bind(i64::from((page - 1) * size));
        Ok(ProviderPage {
            items: builder.build_query_as().fetch_all(&self.pool).await?,
            total,
            facets,
        })
    }
    pub(crate) async fn provider_templates(
        &self,
        id: i64,
        page: u32,
        size: u32,
    ) -> Result<(i64, Vec<(String, String, String, i64)>), sqlx::Error> {
        let total = sqlx::query_scalar(
            "SELECT COUNT(*) FROM template_provider_bindings WHERE provider_id=?",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await?;
        let rows = sqlx::query_as("SELECT t.key,t.name,b.provider_type,t.enabled FROM template_provider_bindings b JOIN agent_templates t ON t.id=b.template_id WHERE b.provider_id=? ORDER BY t.key LIMIT ? OFFSET ?").bind(id).bind(i64::from(size)).bind(i64::from((page-1)*size)).fetch_all(&self.pool).await?;
        Ok((total, rows))
    }
}
/// Desired provider copied out of SQLite before runtime construction.  It contains no resolved
/// credential and is deliberately independent from the read-only runtime catalog.
#[derive(Clone, PartialEq, Eq)]
pub struct DesiredProvider {
    pub id: i64,
    pub key: String,
    pub kind: String,
    pub adapter: String,
    pub config_json: String,
    pub secret_ref: Option<String>,
    pub revision: i64,
}

impl std::fmt::Debug for DesiredProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DesiredProvider")
            .field("id", &self.id)
            .field("revision", &self.revision)
            .field("kind", &self.kind)
            .field("adapter", &self.adapter)
            .finish_non_exhaustive()
    }
}

impl Database {
    /// Reads desired state only.  Caller chooses whether an outcome is required, optional, or
    /// unbound; this database seam never constructs a provider or resolves a secret.
    pub async fn enabled_provider_rows(&self) -> Result<Vec<DesiredProvider>, DatabaseError> {
        let rows = sqlx::query_as::<_, (i64, String, String, String, String, i64, Option<String>)>(
            "SELECT id,key,type,adapter,config_json,revision,credential_json FROM providers WHERE enabled=1 ORDER BY id",
        ).fetch_all(&self.pool).await.map_err(map_sqlx_error)?;
        Ok(rows
            .into_iter()
            .map(
                |(id, key, kind, adapter, config_json, revision, credential)| {
                    let secret_ref = credentials::reference(
                        &format!("provider:{key}"),
                        credential.as_deref(),
                        secrets::provider_secret_env(&key, &adapter),
                    );
                    DesiredProvider {
                        id,
                        key,
                        kind,
                        adapter,
                        config_json,
                        secret_ref,
                        revision,
                    }
                },
            )
            .collect())
    }
}
impl Database {
    pub(crate) async fn prewarm_provider_rows(
        &self,
        provider: Option<(i64, i64)>,
        template: Option<i64>,
    ) -> Result<Vec<DesiredProvider>, sqlx::Error> {
        // Bounds are checked in SQL before copying persisted text. The one SELECT ends before build.
        type PrewarmRow = (i64, String, String, String, String, i64, Option<String>);
        let rows: Vec<PrewarmRow> = sqlx::query_as(
        "SELECT DISTINCT p.id,p.key,p.type,p.adapter,p.config_json,p.revision,p.credential_json FROM providers p JOIN template_provider_bindings b ON b.provider_id=p.id JOIN agent_templates t ON t.id=b.template_id WHERE p.enabled=1 AND t.enabled=1 AND (? IS NULL OR (p.id=? AND p.revision=?)) AND (? IS NULL OR t.id=?) AND length(CAST(p.config_json AS BLOB))<=65536 AND length(CAST(p.key AS BLOB))<=128 AND length(CAST(p.type AS BLOB))<=16 AND length(CAST(p.adapter AS BLOB))<=64 LIMIT 4"
    ).bind(provider.map(|v|v.0)).bind(provider.map(|v|v.0)).bind(provider.map(|v|v.1)).bind(template).bind(template).fetch_all(&self.pool).await?;
        Ok(rows
            .into_iter()
            .filter_map(|(id, key, kind, adapter, raw, revision, credential)| {
                let config_json =
                    crate::database::provider_config::validate_raw(&adapter, &raw).ok()?;
                let secret_ref = crate::database::credentials::reference(
                    &format!("provider:{key}"),
                    credential.as_deref(),
                    crate::database::secrets::provider_secret_env(&key, &adapter),
                );
                Some(DesiredProvider {
                    id,
                    key,
                    kind,
                    adapter,
                    config_json,
                    secret_ref,
                    revision,
                })
            })
            .collect())
    }
}

pub(crate) type DiagnosticProviderRow = (
    i64,
    String,
    String,
    String,
    Option<String>,
    i64,
    i64,
    Option<String>,
);
impl Database {
    pub(crate) async fn diagnostic_provider_row(
        &self,
        key: &str,
    ) -> Result<DiagnosticProviderRow, sqlx::Error> {
        sqlx::query_as("SELECT id,key,type,adapter,CASE WHEN length(CAST(config_json AS BLOB))<=65536 THEN config_json ELSE NULL END,revision,enabled,credential_json FROM providers WHERE key=? AND length(CAST(key AS BLOB))<=128 AND length(CAST(type AS BLOB))<=16 AND length(CAST(adapter AS BLOB))<=64").bind(key).fetch_one(&self.pool).await
    }
    pub(crate) async fn default_prewarm_templates(&self) -> Result<Vec<(i64,)>, sqlx::Error> {
        sqlx::query_as("SELECT DISTINCT t.id FROM agent_templates t JOIN agent_template_assignments a ON a.template_id=t.id JOIN agents g ON g.id=a.agent_id WHERE a.enabled=1 AND a.is_default=1 AND t.enabled=1 AND g.enabled=1 LIMIT 256").fetch_all(&self.pool).await
    }
    pub(crate) async fn provider_capability_identity(
        &self,
        key: &str,
    ) -> Result<(i64, String, String, i64), sqlx::Error> {
        sqlx::query_as("SELECT id,adapter,type,revision FROM providers WHERE key=?")
            .bind(key)
            .fetch_one(&self.pool)
            .await
    }
    pub(crate) async fn provider_overview(
        &self,
        tracked_ids: Option<Vec<i64>>,
    ) -> Result<(i64, Vec<(i64, String, i64)>), sqlx::Error> {
        let count = sqlx::query_scalar("SELECT COUNT(*) FROM providers")
            .fetch_one(&self.pool)
            .await?;
        let rows = if let Some(ids) = tracked_ids {
            if ids.is_empty() {
                Vec::new()
            } else {
                let mut query = QueryBuilder::<Sqlite>::new(
                    "SELECT id, '', revision FROM providers WHERE id IN (",
                );
                let mut values = query.separated(",");
                for id in ids {
                    values.push_bind(id);
                }
                values.push_unseparated(")");
                query.build_query_as().fetch_all(&self.pool).await?
            }
        } else {
            sqlx::query_as("SELECT id,key,revision FROM providers")
                .fetch_all(&self.pool)
                .await?
        };
        Ok((count, rows))
    }
}
