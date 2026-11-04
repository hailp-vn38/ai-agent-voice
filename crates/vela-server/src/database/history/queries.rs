//! Bounded transcript reads and atomic scoped purge.
use crate::database::{
    Database,
    audit::{AuditOutcome, audit},
    writes::WriteError,
};
use serde::Serialize;
use sqlx::{FromRow, Sqlite};
#[derive(Serialize, FromRow)]
pub(crate) struct HistoryMessage {
    pub(crate) id: i64,
    pub(crate) session_id: String,
    pub(crate) device_id: i64,
    pub(crate) agent_id: i64,
    pub(crate) template_id: Option<i64>,
    pub(crate) sequence: i64,
    pub(crate) turn_id: Option<String>,
    pub(crate) role: String,
    pub(crate) text: String,
    /// Unix milliseconds UTC, matching the retention cutoff this archive is pruned by.
    pub(crate) created_at: i64,
}

pub(crate) struct HistoryFilters {
    pub(crate) session_id: Option<String>,
    pub(crate) device_id: Option<i64>,
    pub(crate) agent_id: Option<i64>,
    pub(crate) template_id: Option<i64>,
    pub(crate) role: Option<&'static str>,
}

impl HistoryFilters {
    /// Adds each present filter as one clause.  Absent filters contribute nothing, so the query is
    /// the whole archive only when the request asked for the whole archive.
    ///
    /// The set is consumed because every value it holds is bound into the query it builds.
    fn push(self, query: &mut sqlx::QueryBuilder<'_, Sqlite>) {
        if let Some(session_id) = self.session_id {
            query.push(" AND session_id = ").push_bind(session_id);
        }
        for (column, id) in [
            ("device_id", self.device_id),
            ("agent_id", self.agent_id),
            ("template_id", self.template_id),
        ] {
            if let Some(id) = id {
                query.push(" AND ").push(column).push(" = ").push_bind(id);
            }
        }
        if let Some(role) = self.role {
            query.push(" AND role = ").push_bind(role);
        }
    }
}

pub(crate) enum PurgeScope {
    Device(i64),
    Session(String),
    All,
}

impl PurgeScope {
    /// Adds this scope's predicate, or nothing at all for the confirmed all-history delete.
    fn push(self, query: &mut sqlx::QueryBuilder<'_, Sqlite>) {
        match self {
            Self::All => {}
            Self::Device(id) => {
                query.push(" AND device_id = ").push_bind(id);
            }
            Self::Session(id) => {
                query.push(" AND session_id = ").push_bind(id);
            }
        }
    }
}

impl Database {
    pub(crate) async fn list_history(
        &self,
        filters: HistoryFilters,
        sort: Option<&str>,
        page: u32,
        page_size: u32,
    ) -> Result<Vec<HistoryMessage>, sqlx::Error> {
        let order = match sort.unwrap_or("-created_at") {
            "created_at" => "created_at ASC",
            "sequence" => "sequence ASC",
            "-sequence" => "sequence DESC",
            _ => "created_at DESC",
        };
        let mut builder = sqlx::QueryBuilder::<Sqlite>::new(
            "SELECT id, session_id, device_id, agent_id, template_id, sequence, turn_id, role, text, \
         created_at FROM history_messages WHERE 1 = 1",
        );
        filters.push(&mut builder);
        builder
            .push(" ORDER BY ")
            .push(order)
            .push(" LIMIT ")
            .push_bind(i64::from(page_size))
            .push(" OFFSET ")
            .push_bind(i64::from((page - 1) * page_size));
        builder.build_query_as().fetch_all(&self.pool).await
    }
    pub(crate) async fn purge_history(
        &self,
        scope: PurgeScope,
        request_id: &str,
    ) -> Result<u64, WriteError> {
        let pool = &self.pool;
        let mut transaction = match pool.begin().await {
            Ok(value) => value,
            Err(error_value) => return Err(WriteError::Sql(error_value)),
        };
        let mut builder =
            sqlx::QueryBuilder::<Sqlite>::new("DELETE FROM history_messages WHERE 1 = 1");
        scope.push(&mut builder);
        let deleted = match builder.build().execute(&mut *transaction).await {
            Ok(result) => result.rows_affected(),
            Err(error_value) => {
                let _ = transaction.rollback().await;
                return Err(WriteError::Sql(error_value));
            }
        };
        // The deletion and the record of it are one transaction: an archive an operator cannot account
        // for is the one outcome worse than a failed purge.
        if audit(
            &mut *transaction,
            request_id,
            "history",
            None,
            "purge",
            None,
            None,
            AuditOutcome::Success,
            deleted,
        )
        .await
        .is_err()
            || transaction.commit().await.is_err()
        {
            return Err(WriteError::Unavailable);
        }
        Ok(deleted)
    }
}
