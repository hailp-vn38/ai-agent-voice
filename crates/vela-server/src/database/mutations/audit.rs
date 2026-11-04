use serde::Serialize;
use sqlx::{Executor, FromRow, QueryBuilder, Sqlite, SqlitePool};

#[derive(Clone, Debug, Serialize, FromRow, PartialEq, Eq)]
pub struct AuditLogEventRow {
    pub id: i64,
    pub created_at: i64,
    pub request_id: String,
    pub resource_type: String,
    pub resource_id: Option<i64>,
    pub action: String,
    pub prior_revision: Option<i64>,
    pub new_revision: Option<i64>,
    pub outcome: String,
    pub error_kind: Option<String>,
    pub affected_rows: Option<i64>,
}

#[derive(Clone, Debug, Default)]
pub struct AuditLogFilter {
    pub page: u32,
    pub page_size: u32,
    pub resource_type: Option<String>,
    pub outcome: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct AuditLogPage {
    pub items: Vec<AuditLogEventRow>,
    pub total: u64,
    pub page: u32,
    pub page_size: u32,
}

fn push_audit_filters<'a>(qb: &mut QueryBuilder<'a, Sqlite>, filter: &'a AuditLogFilter) {
    if let Some(ref resource_type) = filter.resource_type {
        qb.push(" AND resource_type = ").push_bind(resource_type);
    }
    if let Some(ref outcome) = filter.outcome {
        qb.push(" AND outcome = ").push_bind(outcome);
    }
}

impl crate::database::Database {
    pub async fn list_audit_events(
        &self,
        filter: &AuditLogFilter,
    ) -> Result<AuditLogPage, sqlx::Error> {
        let mut count_qb: QueryBuilder<Sqlite> =
            QueryBuilder::new("SELECT COUNT(*) FROM admin_audit_events WHERE 1=1");
        push_audit_filters(&mut count_qb, filter);
        let total: (i64,) = count_qb.build_query_as().fetch_one(&self.pool).await?;

        let offset = i64::from(filter.page.saturating_sub(1)) * i64::from(filter.page_size);
        let limit = i64::from(filter.page_size);

        let mut list_qb: QueryBuilder<Sqlite> = QueryBuilder::new(
            "SELECT id, created_at, request_id, resource_type, resource_id, action, prior_revision, new_revision, outcome, error_kind, affected_rows FROM admin_audit_events WHERE 1=1",
        );
        push_audit_filters(&mut list_qb, filter);
        list_qb
            .push(" ORDER BY id DESC LIMIT ")
            .push_bind(limit)
            .push(" OFFSET ")
            .push_bind(offset);

        let items: Vec<AuditLogEventRow> = list_qb.build_query_as().fetch_all(&self.pool).await?;
        Ok(AuditLogPage {
            items,
            total: total.0.max(0) as u64,
            page: filter.page,
            page_size: filter.page_size,
        })
    }
}

/// What an administration did, and therefore what its audit row records.
///
/// The outcome and the reason a row failed travel as one value because they are never independent:
/// a mismatched pair would be representable as two free strings and meaningless in practice.
#[derive(Clone, Copy)]
pub(crate) enum AuditOutcome {
    Success,
    RevisionConflict,
}

impl AuditOutcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::RevisionConflict => "conflict",
        }
    }

    /// The only reason kind V1 records, and it belongs to exactly one outcome.
    fn error_kind(self) -> Option<&'static str> {
        match self {
            Self::Success => None,
            Self::RevisionConflict => Some("revision_conflict"),
        }
    }
}

/// One audit row: enough to account for an administration without recording any of it.
///
/// `resource_id` is `None` for an operation that stands behind no single resource row — a scoped
/// History Purge names a scope rather than a resource and has no revision pair, so its `action` and
/// `affected_rows` are the only thing that identifies it.  Nothing about the text a purge deleted
/// does: a purge must leave a countable trace, not a second copy of what it removed.
#[allow(clippy::too_many_arguments)] // Each argument maps one-to-one to the immutable audit schema.
pub(crate) async fn audit<'e, E>(
    executor: E,
    request_id: &str,
    resource: &str,
    resource_id: Option<i64>,
    action: &str,
    prior: Option<i64>,
    new: Option<i64>,
    outcome: AuditOutcome,
    affected_rows: u64,
) -> Result<(), sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query("INSERT INTO admin_audit_events (created_at,request_id,resource_type,resource_id,action,prior_revision,new_revision,outcome,error_kind,affected_rows) VALUES (?,?,?,?,?,?,?,?,?,?)")
        .bind(crate::database::unix_seconds().unwrap_or_default())
        .bind(request_id)
        .bind(resource)
        .bind(resource_id)
        .bind(action)
        .bind(prior)
        .bind(new)
        .bind(outcome.as_str())
        .bind(outcome.error_kind())
        .bind(affected_rows as i64)
        .execute(executor)
        .await
        .map(|_| ())
}

pub(crate) async fn audit_conflict(
    pool: &SqlitePool,
    request_id: String,
    resource: &str,
    resource_id: i64,
    expected: i64,
) {
    audit_conflict_action(pool, request_id, resource, resource_id, expected, "update").await;
}

pub(crate) async fn audit_conflict_action(
    pool: &SqlitePool,
    request_id: String,
    resource: &str,
    resource_id: i64,
    expected: i64,
    action: &str,
) {
    let _ = audit(
        pool,
        &request_id,
        resource,
        Some(resource_id),
        action,
        Some(expected),
        None,
        AuditOutcome::RevisionConflict,
        1,
    )
    .await;
}
