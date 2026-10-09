//! Template persistence and assignment transactions.
use super::{
    Database,
    agents::get_agent_by,
    audit::{AuditOutcome, audit, audit_conflict},
    writes::WriteError,
};
use serde::Serialize;
use sqlx::{FromRow, QueryBuilder, Sqlite, SqlitePool};
mod relationships;
#[derive(Serialize, FromRow)]
pub(crate) struct Template {
    pub(crate) id: i64,
    pub(crate) key: String,
    pub(crate) name: String,
    pub(crate) description: Option<String>,
    pub(crate) language: String,
    pub(crate) prompt: String,
    pub(crate) enabled: i64,
    pub(crate) revision: i64,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
}
pub(crate) async fn template_by(database: &Database, key: &str) -> Result<Template, sqlx::Error> {
    sqlx::query_as("SELECT id,key,name,description,language,prompt,enabled,revision,created_at,updated_at FROM agent_templates WHERE key=?").bind(key).fetch_one(&database.pool).await
}
pub(crate) struct TemplateFilters {
    enabled: Option<bool>,
    q: Option<String>,
    language: Option<String>,
}
fn append_template_filters(builder: &mut QueryBuilder<Sqlite>, filters: &TemplateFilters) {
    let mut first = true;
    let mut clause = |builder: &mut QueryBuilder<Sqlite>| {
        builder.push(if first { " WHERE " } else { " AND " });
        first = false;
    };
    if let Some(enabled) = filters.enabled {
        clause(builder);
        builder.push("enabled=").push_bind(i64::from(enabled));
    }
    if let Some(language) = &filters.language {
        clause(builder);
        builder.push("language=").push_bind(language.clone());
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
            .push_bind(q.clone())
            .push(" ESCAPE '\\' OR description LIKE ")
            .push_bind(q)
            .push(" ESCAPE '\\')");
    }
}
async fn template_count(pool: &SqlitePool, filters: &TemplateFilters) -> Result<i64, sqlx::Error> {
    let mut builder = QueryBuilder::<Sqlite>::new("SELECT COUNT(*) FROM agent_templates");
    append_template_filters(&mut builder, filters);
    builder.build_query_scalar().fetch_one(pool).await
}
impl TemplateFilters {
    pub(crate) fn new(
        enabled: Option<bool>,
        q: Option<String>,
        language: Option<String>,
    ) -> Result<Self, ()> {
        let q = q.filter(|value| !value.is_empty());
        let language = language.filter(|value| !value.is_empty());
        if q.as_ref().is_some_and(|value| value.len() > 128)
            || language.as_ref().is_some_and(|value| value.len() > 32)
        {
            return Err(());
        }
        Ok(Self {
            enabled,
            q,
            language,
        })
    }
}

pub(crate) struct TemplateInput<'a> {
    pub key: &'a str,
    pub name: &'a str,
    pub description: Option<&'a str>,
    pub language: &'a str,
    pub prompt: &'a str,
}
pub(crate) struct TemplateChanges<'a> {
    pub name: &'a str,
    pub description: Option<&'a str>,
    pub language: &'a str,
    pub prompt: &'a str,
    pub enabled: i64,
}
impl Database {
    pub(crate) async fn create_template(
        &self,
        input: TemplateInput<'_>,
        request_id: &str,
    ) -> Result<(), WriteError> {
        let pool = &self.pool;
        let mut tx = match pool.begin().await {
            Ok(v) => v,
            Err(_) => {
                return Err(WriteError::Unavailable);
            }
        };
        let time = crate::database::unix_seconds().unwrap_or_default();
        let result = sqlx::query("INSERT INTO agent_templates (key,name,description,language,prompt,created_at,updated_at) VALUES (?,?,?,?,?,?,?)").bind(input.key).bind(input.name).bind(input.description).bind(input.language).bind(input.prompt).bind(time).bind(time).execute(&mut *tx).await;
        let resource_id = match result {
            Ok(v) => v.last_insert_rowid(),
            Err(e) => return Err(WriteError::Mutation(e)),
        };
        if audit(
            &mut *tx,
            request_id,
            "template",
            Some(resource_id),
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
        }
        Ok(())
    }
    pub(crate) async fn update_template(
        &self,
        resource_id: i64,
        expected: i64,
        input: TemplateChanges<'_>,
        request_id: &str,
    ) -> Result<(), WriteError> {
        let pool = &self.pool;
        let mut tx = match pool.begin().await {
            Ok(v) => v,
            Err(_) => {
                return Err(WriteError::Unavailable);
            }
        };
        let updated = sqlx::query("UPDATE agent_templates SET name=?,description=?,language=?,prompt=?,enabled=?,revision=revision+1,updated_at=? WHERE id=? AND revision=?").bind(input.name).bind(input.description).bind(input.language).bind(input.prompt).bind(input.enabled).bind(crate::database::unix_seconds().unwrap_or_default()).bind(resource_id).bind(expected).execute(&mut *tx).await.map(|v| v.rows_affected()==1).unwrap_or(false);
        if !updated {
            let _ = tx.rollback().await;
            return Err(WriteError::Conflict("revision_conflict"));
        }
        if audit(
            &mut *tx,
            request_id,
            "template",
            Some(resource_id),
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
    pub(crate) async fn bind_template_provider(
        &self,
        key: &str,
        provider_type: &str,
        provider_key: &str,
        expected: i64,
        request_id: &str,
    ) -> Result<i64, WriteError> {
        let pool = &self.pool;
        let template = match template_by(self, key).await {
            Ok(v) => v,
            Err(sqlx::Error::RowNotFound) => {
                return Err(WriteError::NotFound);
            }
            Err(e) => return Err(WriteError::Sql(e)),
        };
        if template.revision != expected {
            return Err(WriteError::Conflict("revision_conflict"));
        };
        let provider: Result<(i64, String, i64), _> =
            sqlx::query_as("SELECT id,type,enabled FROM providers WHERE key=?")
                .bind(provider_key)
                .fetch_one(pool)
                .await;
        let (provider_id, provider_kind, enabled) = match provider {
            Ok(v) => v,
            Err(_) => return Err(WriteError::Invalid("invalid_provider")),
        };
        if provider_kind != provider_type || enabled != 1 {
            return Err(WriteError::Invalid("invalid_provider"));
        };
        let mut tx = match pool.begin().await {
            Ok(v) => v,
            Err(_) => {
                return Err(WriteError::Unavailable);
            }
        };
        if sqlx::query("INSERT INTO template_provider_bindings (template_id,provider_type,provider_id,created_at,updated_at) VALUES (?,?,?,?,?) ON CONFLICT(template_id,provider_type) DO UPDATE SET provider_id=excluded.provider_id,updated_at=excluded.updated_at").bind(template.id).bind(provider_type).bind(provider_id).bind(crate::database::unix_seconds().unwrap_or_default()).bind(crate::database::unix_seconds().unwrap_or_default()).execute(&mut *tx).await.is_err() || sqlx::query("UPDATE agent_templates SET revision=revision+1,updated_at=? WHERE id=? AND revision=?").bind(crate::database::unix_seconds().unwrap_or_default()).bind(template.id).bind(expected).execute(&mut *tx).await.map(|v|v.rows_affected()!=1).unwrap_or(true) || audit(&mut *tx,request_id,"template",Some(template.id),"bind_provider",Some(expected),Some(expected+1),AuditOutcome::Success, 1).await.is_err() || tx.commit().await.is_err(){return Err(WriteError::Unavailable)};
        Ok(template.id)
    }
    pub(crate) async fn set_default_template(
        &self,
        agent_key: &str,
        template_key: &str,
        expected: i64,
        request_id: &str,
    ) -> Result<i64, WriteError> {
        let pool = &self.pool;
        let agent = match get_agent_by(self, agent_key).await {
            Ok(v) => v,
            Err(sqlx::Error::RowNotFound) => {
                return Err(WriteError::NotFound);
            }
            Err(e) => return Err(WriteError::Sql(e)),
        };
        if agent.revision != expected {
            return Err(WriteError::Conflict("revision_conflict"));
        };
        let template = match template_by(self, template_key).await {
            Ok(v) if v.enabled == 1 => v,
            _ => return Err(WriteError::Invalid("invalid_template")),
        };
        let mut tx = match pool.begin().await {
            Ok(v) => v,
            Err(_) => {
                return Err(WriteError::Unavailable);
            }
        };
        let time = crate::database::unix_seconds().unwrap_or_default();
        if sqlx::query("UPDATE agent_template_assignments SET is_default=0 WHERE agent_id=? AND enabled=1").bind(agent.id).execute(&mut *tx).await.is_err()||sqlx::query("INSERT INTO agent_template_assignments(agent_id,template_id,is_default,enabled,created_at) VALUES (?,?,1,1,?) ON CONFLICT(agent_id,template_id) DO UPDATE SET is_default=1,enabled=1").bind(agent.id).bind(template.id).bind(time).execute(&mut *tx).await.is_err()||sqlx::query("UPDATE agents SET revision=revision+1,updated_at=? WHERE id=? AND revision=?").bind(time).bind(agent.id).bind(expected).execute(&mut *tx).await.map(|v|v.rows_affected()!=1).unwrap_or(true)||audit(&mut *tx,request_id,"agent",Some(agent.id),"set_default_template",Some(expected),Some(expected+1),AuditOutcome::Success, 1).await.is_err()||tx.commit().await.is_err(){return Err(WriteError::Unavailable)};
        Ok(template.id)
    }
    pub(crate) async fn assign_template(
        &self,
        agent_key: &str,
        template_key: &str,
        expected: i64,
        request_id: &str,
    ) -> Result<(), WriteError> {
        let pool = &self.pool;
        let agent = match get_agent_by(self, agent_key).await {
            Ok(v) => v,
            Err(sqlx::Error::RowNotFound) => {
                return Err(WriteError::NotFound);
            }
            Err(e) => return Err(WriteError::Sql(e)),
        };
        if agent.revision != expected {
            audit_conflict(pool, request_id.into(), "agent", agent.id, expected).await;
            return Err(WriteError::Conflict("revision_conflict"));
        }
        let template = match template_by(self, template_key).await {
            Ok(v) if v.enabled == 1 => v,
            _ => return Err(WriteError::Invalid("invalid_template")),
        };
        let mut tx = match pool.begin().await {
            Ok(v) => v,
            Err(_) => {
                return Err(WriteError::Unavailable);
            }
        };
        // An Agent inside the Template mechanism has no path back to server defaults, so leaving it
        // with assignments but no enabled default refuses every one of its devices with a 503. The
        // first assignment therefore takes the default slot whenever it can serve as one: an absent
        // core slot falls back to the deployment default, so only an explicit *broken* core binding
        // (disabled provider, or a provider whose type no longer matches the slot) makes the Template
        // structurally incomplete. The optional Speaker slot is never counted here, so binding a
        // Speaker provider cannot mask a missing core slot. A failed probe promotes anyway, because
        // the invariant matters more than the optimistic read.
        let broken_core: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM template_provider_bindings b JOIN providers p ON p.id=b.provider_id WHERE b.template_id=? AND b.provider_type IN ('vad','asr','llm','tts') AND (p.enabled!=1 OR b.provider_type!=p.type)")
        .bind(template.id).fetch_one(&mut *tx).await.unwrap_or(0);
        let promote: i64 = if broken_core == 0 {
            sqlx::query_scalar::<_, i64>("SELECT NOT EXISTS(SELECT 1 FROM agent_template_assignments WHERE agent_id=? AND enabled=1 AND is_default=1)")
            .bind(agent.id).fetch_one(&mut *tx).await.unwrap_or(1)
        } else {
            0
        };
        let mutated = sqlx::query("INSERT INTO agent_template_assignments(agent_id,template_id,is_default,enabled,created_at) VALUES (?,?,?,1,?) ON CONFLICT(agent_id,template_id) DO UPDATE SET enabled=1,is_default=MAX(is_default,excluded.is_default)")
        .bind(agent.id).bind(template.id).bind(promote).bind(crate::database::unix_seconds().unwrap_or_default()).execute(&mut *tx).await.is_ok()
        && sqlx::query("UPDATE agents SET revision=revision+1,updated_at=? WHERE id=? AND revision=?").bind(crate::database::unix_seconds().unwrap_or_default()).bind(agent.id).bind(expected).execute(&mut *tx).await.map(|v| v.rows_affected() == 1).unwrap_or(false);
        if !mutated {
            let _ = tx.rollback().await;
            audit_conflict(pool, request_id.into(), "agent", agent.id, expected).await;
            return Err(WriteError::Conflict("revision_conflict"));
        }
        if audit(
            &mut *tx,
            request_id,
            "agent",
            Some(agent.id),
            "assign_template",
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
    pub(crate) async fn template_conflict(&self, request_id: &str, id: i64, expected: i64) {
        audit_conflict(&self.pool, request_id.into(), "template", id, expected).await;
    }
    pub(crate) async fn list_templates(
        &self,
        filters: &TemplateFilters,
        sort: Option<&str>,
        page: u32,
        size: u32,
    ) -> Result<(i64, Vec<Template>), sqlx::Error> {
        let order = match sort.unwrap_or("key") {
            "-key" => "key DESC",
            "name" => "name ASC, key ASC",
            "-name" => "name DESC, key ASC",
            "language" => "language ASC, key ASC",
            _ => "key ASC",
        };
        let total = template_count(&self.pool, filters).await?;
        let mut builder = QueryBuilder::<Sqlite>::new(
            "SELECT id,key,name,description,language,prompt,enabled,revision,created_at,updated_at FROM agent_templates",
        );
        append_template_filters(&mut builder, filters);
        builder
            .push(" ORDER BY ")
            .push(order)
            .push(" LIMIT ")
            .push_bind(i64::from(size))
            .push(" OFFSET ")
            .push_bind(i64::from((page - 1) * size));
        Ok((total, builder.build_query_as().fetch_all(&self.pool).await?))
    }
}
