use sqlx::{Executor, Sqlite, SqlitePool};

/// What an administration did, and therefore what its audit row records.
///
/// The outcome and the reason a row failed travel as one value because they are never independent:
/// a mismatched pair would be representable as two free strings and meaningless in practice.
#[derive(Clone, Copy)]
pub(super) enum AuditOutcome {
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
pub(super) async fn audit<'e, E>(
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

pub(super) async fn audit_conflict(
    pool: &SqlitePool,
    request_id: String,
    resource: &str,
    resource_id: i64,
    expected: i64,
) {
    audit_conflict_action(pool, request_id, resource, resource_id, expected, "update").await;
}

pub(super) async fn audit_conflict_action(
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
